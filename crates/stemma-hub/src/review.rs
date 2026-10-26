//! Recipe-only reviews and a durable, per-repository merge queue.
use crate::{
    db,
    error::{ApiError, ApiResult},
    forge,
    model::{self, Message, Tool},
    orchestrator,
    state::AppState,
};
use serde_json::{Value, json};
use stemma_core::{
    protocol::ProcessRepository,
    workshop::{Ask, Assignment, ProcessBranch, RecipeFile},
};

#[derive(sqlx::FromRow)]
struct Review {
    id: i64,
    process_id: i64,
    group_id: i64,
    repository: String,
    pull_number: i64,
    branch: String,
    head: String,
    build_work: Option<i64>,
    state: String,
    rebases: i64,
    rebase_process: Option<i64>,
}
/// Called only while finalizing a recipe's process, never from a public API.
pub(crate) async fn enqueue(
    state: &AppState,
    process: i64,
    branches: &[ProcessBranch],
    recipe: &RecipeFile,
) -> ApiResult<bool> {
    if !recipe
        .recipe
        .as_ref()
        .is_some_and(|r| r.front.merge == stemma_core::recipe::Merge::Review)
    {
        return Ok(false);
    }
    let group: i64 = sqlx::query_scalar(
        "SELECT w.group_id
         FROM workshop_processes p
         JOIN work_leases w ON w.id = p.work_id
         WHERE p.id = ?",
    )
    .bind(process)
    .fetch_one(&state.db)
    .await?;
    let mut queued = false;
    for b in branches {
        if let (Some(head), Some(url)) = (&b.commit, &b.pull_request) {
            let number: i64 = url
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or("")
                .parse()
                .map_err(ApiError::internal)?;
            sqlx::query(
                "INSERT INTO workshop_reviews
                 (process_id, group_id, repository, pull_number, url, branch, head)
                 VALUES (?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT DO NOTHING",
            )
            .bind(process)
            .bind(group)
            .bind(&b.repository)
            .bind(number)
            .bind(url)
            .bind(&b.branch)
            .bind(head)
            .execute(&state.db)
            .await?;
            queued = true;
        }
    }
    Ok(queued)
}
pub async fn tick(state: &AppState) -> ApiResult<()> {
    let Ok(_guard) = state.review_changes.try_lock() else {
        return Ok(());
    };
    sqlx::query(
        "UPDATE workshop_reviews
         SET state = 'queued', resume_month = NULL
         WHERE resume_month IS NOT NULL AND resume_month != ?",
    )
    .bind(model::month())
    .execute(&state.db)
    .await?;
    let rows: Vec<Review> = sqlx::query_as(
        "SELECT v.id, v.process_id, v.group_id, v.repository, v.pull_number, v.branch,
         v.head, v.build_work, v.state, v.rebases, v.rebase_process
         FROM workshop_reviews v
         JOIN workshop_processes p ON p.id = v.process_id
         JOIN workshop_requests r ON r.thread_id = p.thread_id
         WHERE v.state NOT IN ('merged', 'refused', 'waiting') AND r.state IN ('waiting', 'running')
         AND (v.retry_at IS NULL OR v.retry_at <= ?)
         AND NOT EXISTS(SELECT 1
         FROM workshop_reviews earlier
         WHERE earlier.group_id = v.group_id AND earlier.repository = v.repository
         AND earlier.id < v.id AND earlier.state NOT IN ('merged', 'refused'))
         ORDER BY v.id",
    )
    .bind(db::now())
    .fetch_all(&state.db)
    .await?;
    for mut review in rows {
        let thread: i64 = sqlx::query_scalar(
            "SELECT thread_id
             FROM workshop_processes
             WHERE id = ?",
        )
        .bind(review.process_id)
        .fetch_one(&state.db)
        .await?;
        if let Err(error) = advance(state, &mut review, thread).await {
            failed(state, &review, thread, &error).await?;
        }
    }
    Ok(())
}
/// A review waits for an admin on a missing key or the cap, and otherwise
/// keeps its place in the queue and tries again in a minute.
async fn failed(state: &AppState, r: &Review, thread: i64, error: &ApiError) -> ApiResult<()> {
    let text = if model::waits_for_admins(error) {
        sqlx::query(
            "UPDATE workshop_reviews
             SET state = 'waiting', message = ?, resume_month = ?
             WHERE id = ?",
        )
        .bind(&error.message)
        .bind((error.code == "workshop_cap").then(model::month))
        .bind(r.id)
        .execute(&state.db)
        .await?;
        error.message.clone()
    } else {
        let text = format!(
            "{}. Trying again in a minute",
            error.message.trim_end_matches('.')
        );
        sqlx::query(
            "UPDATE workshop_reviews
             SET message = ?, retry_at = ?
             WHERE id = ?",
        )
        .bind(&text)
        .bind(db::now() + model::RETRY_MS)
        .bind(r.id)
        .execute(&state.db)
        .await?;
        text
    };
    orchestrator::wait(state, thread, None, &text).await
}
async fn advance(state: &AppState, r: &mut Review, thread: i64) -> ApiResult<()> {
    if !model::has_key(state, r.group_id).await? {
        return Err(ApiError::new(
            axum::http::StatusCode::CONFLICT,
            "workshop_key",
            "A group model API key is needed for this review",
        ));
    }
    let forge = state
        .forge
        .as_ref()
        .ok_or_else(|| ApiError::not_found("Forge"))?;
    let slug: String = sqlx::query_scalar(
        "SELECT slug
         FROM groups
         WHERE id = ?",
    )
    .bind(r.group_id)
    .fetch_one(&state.db)
    .await?;
    let pull = forge
        .pull(&slug, &r.repository, r.pull_number)
        .await
        .map_err(forge::api_error)?;
    if pull["merged"] == true {
        return finish(state, r, thread, "merged", "Merged into main", None).await;
    }
    if pull["state"] == "closed" {
        return finish(
            state,
            r,
            thread,
            "refused",
            "The pull request was closed without merging",
            None,
        )
        .await;
    }
    if pull["base"]["ref"] != "main" || pull["head"]["ref"].as_str() != Some(&r.branch) {
        return refuse(
            state,
            r,
            thread,
            &slug,
            "The pull request's assigned branch or target changed",
            None,
        )
        .await;
    }
    let head = forge
        .branch_commit(&slug, &r.repository, &r.branch)
        .await
        .map_err(forge::api_error)?
        .ok_or_else(|| ApiError::not_found("Pull request branch"))?;
    if r.state == "rebasing" {
        let (status, error): (String, Option<String>) = sqlx::query_as(
            "SELECT w.state, w.error
             FROM workshop_processes p
             JOIN work_leases w ON w.id = p.work_id
             WHERE p.id = ?",
        )
        .bind(r.rebase_process)
        .fetch_one(&state.db)
        .await?;
        if status == "queued" || status == "running" {
            return Ok(());
        }
        if status != "passed" || head == r.head {
            r.head = head;
            return rebase(
                state,
                r,
                thread,
                &slug,
                error
                    .as_deref()
                    .unwrap_or("The rebase did not update the branch"),
            )
            .await;
        }
        r.build_work = None;
        r.state = "queued".into();
    }
    if head != r.head {
        r.head = head;
        r.build_work = None;
    }
    // A head behind main needs a rebase before any build of it.
    let main = forge
        .branch_commit(&slug, &r.repository, "main")
        .await
        .map_err(forge::api_error)?
        .ok_or_else(|| ApiError::not_found("Repository main"))?;
    if !forge
        .contains(&slug, &r.repository, &r.head, &main)
        .await
        .map_err(forge::api_error)?
    {
        return rebase(
            state,
            r,
            thread,
            &slug,
            "The pull request's head does not contain current main",
        )
        .await;
    }
    if r.build_work.is_none() {
        let work = crate::builds::review_head(state, r.group_id, &r.repository, &r.head).await?;
        sqlx::query(
            "UPDATE workshop_reviews
             SET head = ?, build_work = ?, state = 'building', message = NULL
             WHERE id = ?",
        )
        .bind(&r.head)
        .bind(work)
        .bind(r.id)
        .execute(&state.db)
        .await?;
        r.build_work = Some(work);
        r.state = "building".into();
        orchestrator::wait(state, thread, None, "Building the pull request").await?;
    }
    let (status, commit, error): (String, String, Option<String>) = sqlx::query_as(
        "SELECT state, commit_sha, error
         FROM work_leases
         WHERE id = ?",
    )
    .bind(r.build_work)
    .fetch_one(&state.db)
    .await?;
    if status == "queued" || status == "running" {
        return Ok(());
    }
    if status != "passed" || commit != r.head {
        return refuse(
            state,
            r,
            thread,
            &slug,
            &format!(
                "The pull request's head did not build: {}",
                error.as_deref().unwrap_or(&status)
            ),
            None,
        )
        .await;
    }
    let (raw_recipe, raw_ask): (String, String) = sqlx::query_as(
        "SELECT coalesce(p.recipe, r.recipe), coalesce(p.ask, r.ask)
         FROM workshop_processes p
         JOIN workshop_requests r ON r.thread_id = p.thread_id
         WHERE p.id = ?",
    )
    .bind(r.process_id)
    .fetch_one(&state.db)
    .await?;
    let recipe: RecipeFile = serde_json::from_str(&raw_recipe).map_err(ApiError::internal)?;
    let ask: Ask = serde_json::from_str(&raw_ask).map_err(ApiError::internal)?;
    let files = forge
        .pull_files(&slug, &r.repository, r.pull_number)
        .await
        .map_err(forge::api_error)?;
    let diff = forge
        .pull_diff(&slug, &r.repository, r.pull_number)
        .await
        .map_err(forge::api_error)?;
    // The inspection is of this exact head; retain the declarations from changed
    // modules, including their sorry flags and complete axiom lists.
    let mut inspection = vec![];
    if r.repository == "library" {
        let rows: Vec<(String, String, String, bool, bool, String)> = sqlx::query_as(
            "SELECT d.name, d.module, d.type, d.sorry, d.data_sorry, d.axioms
             FROM library_declarations d
             JOIN library_builds b ON b.id = d.build_id
             WHERE b.work_id = ?",
        )
        .bind(r.build_work)
        .fetch_all(&state.db)
        .await?;
        let modules: Vec<String> = files
            .iter()
            .filter_map(|f| f["filename"].as_str())
            .filter_map(|f| f.strip_suffix(".lean"))
            .map(|f| f.replace('/', "."))
            .collect();
        for (name, module, type_, sorry, data_sorry, axioms) in rows {
            if !modules.contains(&module) {
                continue;
            }
            inspection.push(json!({
                "name": name, "module": module, "type": type_, "sorry": sorry,
                "data_sorry": data_sorry,
                "axioms": serde_json::from_str::<Value>(&axioms).map_err(ApiError::internal)?
            }));
        }
        if !recipe.recipe.as_ref().unwrap().front.accept_sorry
            && inspection
                .iter()
                .any(|d| d["sorry"] == true || d["data_sorry"] == true)
        {
            return refuse(
                state,
                r,
                thread,
                &slug,
                "The changed library contains sorry, which this recipe does not accept",
                None,
            )
            .await;
        }
    }
    sqlx::query(
        "UPDATE workshop_reviews
         SET state = 'reviewing'
         WHERE id = ?",
    )
    .bind(r.id)
    .execute(&state.db)
    .await?;
    let tools = vec![Tool {
        name: "decide".into(),
        description: "Decide this pull request, with a reason".into(),
        parameters: json!({"type": "object", "properties": {
            "decision": {"enum": ["merge", "refuse", "rebase"]},
            "message": {"type": "string"}
        }, "required": ["decision", "message"], "additionalProperties": false}),
    }];
    let context = json!({
        "pull": pull, "head": r.head, "main": main, "recipe": recipe,
        "request": ask, "build": {"state": status, "commit": commit},
        "inspection": inspection, "diff": diff, "files": files
    });
    let mut history = vec![
        Message::text("system", include_str!("../prompts/review.md")),
        Message::text("user", context.to_string()),
    ];
    let limit = model::step_cap(state, r.group_id).await?;
    for _ in 0..limit {
        let reply = model::call(state, r.group_id, &history, &tools).await?;
        let calls = reply.message.calls.clone();
        history.push(reply.message);
        for call in calls {
            if call.name != "decide" {
                history.push(Message::result(&call, "Unknown tool".into()));
                continue;
            }
            let message = call.arguments["message"].as_str().unwrap_or("").trim();
            if message.is_empty() {
                history.push(Message::result(
                    &call,
                    "A decision requires a message".into(),
                ));
                continue;
            }
            match call.arguments["decision"].as_str() {
                Some("refuse") => {
                    return refuse(state, r, thread, &slug, message, Some(&reply.model)).await;
                }
                Some("rebase") => return rebase(state, r, thread, &slug, message).await,
                Some("merge") => {
                    // Re-read both heads after the model call, while this repository's
                    // queue is held. Forgejo also fences the expected PR head.
                    let current_main = forge
                        .branch_commit(&slug, &r.repository, "main")
                        .await
                        .map_err(forge::api_error)?;
                    let current_head = forge
                        .branch_commit(&slug, &r.repository, &r.branch)
                        .await
                        .map_err(forge::api_error)?;
                    if current_head.as_deref() != Some(&r.head) {
                        sqlx::query(
                            "UPDATE workshop_reviews
                             SET build_work = NULL, state = 'queued'
                             WHERE id = ?",
                        )
                        .bind(r.id)
                        .execute(&state.db)
                        .await?;
                        return Ok(());
                    }
                    if current_main.as_deref() != Some(&main) {
                        return rebase(state, r, thread, &slug, "Main advanced during review")
                            .await;
                    }
                    let still_open: bool = sqlx::query_scalar(
                        "SELECT state IN ('waiting', 'running')
                         FROM workshop_requests
                         WHERE thread_id = ?",
                    )
                    .bind(thread)
                    .fetch_one(&state.db)
                    .await?;
                    if !still_open {
                        return Ok(());
                    }
                    match forge
                        .merge_pull(&slug, &r.repository, r.pull_number, &r.head, message)
                        .await
                    {
                        Ok(()) => {
                            let paper = paper(state, r).await?;
                            crate::builds::refresh(state, r.group_id, &r.repository, paper).await?;
                            return finish(
                                state,
                                r,
                                thread,
                                "merged",
                                &format!("Merged into main. {message}"),
                                Some(&reply.model),
                            )
                            .await;
                        }
                        Err(e) if e.is_conflict() => {
                            return rebase(state, r, thread, &slug, "Forgejo requires a rebase")
                                .await;
                        }
                        Err(e) => return Err(forge::api_error(e)),
                    }
                }
                _ => history.push(Message::result(
                    &call,
                    "Choose merge, refuse or rebase".into(),
                )),
            }
        }
    }
    Err(ApiError::invalid(
        "Review reached its step limit; waiting for the requester or an admin",
    ))
}
async fn paper(state: &AppState, r: &Review) -> ApiResult<Option<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT id
         FROM papers
         WHERE group_id = ? AND repository = ?",
    )
    .bind(r.group_id)
    .bind(&r.repository)
    .fetch_optional(&state.db)
    .await?)
}
async fn refuse(
    state: &AppState,
    r: &Review,
    thread: i64,
    slug: &str,
    text: &str,
    model: Option<&str>,
) -> ApiResult<()> {
    state
        .forge
        .as_ref()
        .unwrap()
        .review_comment(slug, &r.repository, r.pull_number, &r.head, text)
        .await
        .map_err(forge::api_error)?;
    finish(
        state,
        r,
        thread,
        "refused",
        &format!("Refused the pull request. {text}"),
        model,
    )
    .await
}
async fn finish(
    state: &AppState,
    r: &Review,
    thread: i64,
    status: &str,
    text: &str,
    model: Option<&str>,
) -> ApiResult<()> {
    sqlx::query(
        "UPDATE workshop_reviews
         SET state = ?, message = ?
         WHERE id = ?",
    )
    .bind(status)
    .bind(text)
    .bind(r.id)
    .execute(&state.db)
    .await?;
    let number = orchestrator::post(state, thread, model, text).await?;
    let req = orchestrator::request(state, thread).await?;
    crate::notifications::notify(
        state,
        &[req.requester.id],
        &crate::notifications::Notice::new(if status == "merged" {
            stemma_core::api::NotificationKind::ReviewMerged
        } else {
            stemma_core::api::NotificationKind::ReviewRefused
        })
        .at(r.group_id, thread, Some(number))
        .text(text),
    )
    .await?;
    orchestrator::wake(state, thread).await
}
async fn rebase(
    state: &AppState,
    r: &mut Review,
    thread: i64,
    slug: &str,
    reason: &str,
) -> ApiResult<()> {
    if r.rebases >= 3 {
        return refuse(
            state,
            r,
            thread,
            slug,
            "Three rebases failed to produce a mergeable pull request",
            None,
        )
        .await;
    }
    let (raw, machine, owner): (String, Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT coalesce(p.ask, r.ask), p.machine_id, p.owner_id
         FROM workshop_processes p
         JOIN workshop_requests r ON r.thread_id = p.thread_id
         JOIN work_leases w ON w.id = p.work_id
         WHERE p.id = ?",
    )
    .bind(r.process_id)
    .fetch_one(&state.db)
    .await?;
    let machine =
        machine.ok_or_else(|| ApiError::invalid("Waiting for the original machine to rebase"))?;
    let owner = owner.ok_or_else(|| ApiError::invalid("The original process has no owner"))?;
    let mut ask: Ask = serde_json::from_str(&raw).map_err(ApiError::internal)?;
    ask.recipe = "rebase".into();
    ask.inputs = [
        ("repository".into(), r.repository.clone()),
        ("branch".into(), r.branch.clone()),
    ]
    .into();
    ask.assignment = Assignment::Member { user: owner };
    let repositories = vec![ProcessRepository {
        path: format!("/git/{slug}/{}.git", r.repository),
        base: r.head.clone(),
        branch: r.branch.clone(),
        force_push: true,
    }];
    orchestrator::launch(
        state,
        thread,
        ask,
        Some(r.id),
        Some(repositories),
        Some(machine),
    )
    .await?;
    sqlx::query("UPDATE workshop_reviews SET message = ? WHERE id = ?")
        .bind(reason)
        .bind(r.id)
        .execute(&state.db)
        .await?;
    orchestrator::post(
        state,
        thread,
        None,
        &format!(
            "Needs a rebase: launched on {}'s original machine. {reason}",
            crate::workshop::user(state, owner).await?.display_name
        ),
    )
    .await?;
    Ok(())
}
