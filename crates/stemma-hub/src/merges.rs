//! The git orchestrator: one standard way for every pull request a process
//! opens. It prepares the pull request's merge into `main`, and a person
//! approves it.
//!
//! - Without conflicts, the prepared merge is git's own.
//! - With conflicts, the group's model resolves only the conflicting files
//!   and says, in plain words, how.
//!
//! Approving makes a true merge commit of `main` and the pull request's head
//! with the prepared tree, pushes it to the pull request's branch, and moves
//! `main` to it through the forge. Nothing at approval is left to a model.
//! Every change to `main` prepares its open pull requests again.
//!
//! The hub does this with git's plumbing on a bare copy of the repository:
//! no checkout, no build, nothing heavy.
use std::path::PathBuf;

use crate::{
    api::{Json, Membership},
    auth::Viewer,
    db,
    error::{ApiError, ApiResult},
    forge,
    model::{self, Message, Tool},
    notifications::{self, Notice},
    orchestrator,
    state::AppState,
};
use axum::extract::{Path as UrlPath, State};
use serde_json::json;
use stemma_core::{api::NotificationKind, workshop::Merge};

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    process_id: i64,
    group_id: i64,
    repository: String,
    pull_number: i64,
    url: String,
    branch: String,
    title: String,
    body: String,
    state: String,
    main: Option<String>,
    head: Option<String>,
    tree: Option<String>,
    conflicts: String,
    explanation: Option<String>,
    message: Option<String>,
    approved_by: Option<i64>,
}
const ROW: &str = "SELECT id, process_id, group_id, repository, pull_number, url, branch, title,
                   body, state, main, head, tree, conflicts, explanation, message, approved_by
                   FROM workshop_merges";

/// The most of one conflicting file the model is shown, on each side.
const MAX_CONFLICT_BYTES: usize = 200_000;

/// A pull request a process opened: its merge is prepared next.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn open(
    state: &AppState,
    process: i64,
    group: i64,
    repository: &str,
    branch: &str,
    url: &str,
    title: &str,
    body: &str,
) -> ApiResult<()> {
    let number: i64 = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .parse()
        .map_err(ApiError::internal)?;
    sqlx::query(
        "INSERT INTO workshop_merges
         (process_id, group_id, repository, pull_number, url, branch, title, body, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(process)
    .bind(group)
    .bind(repository)
    .bind(number)
    .bind(url)
    .bind(branch)
    .bind(title)
    .bind(body)
    .bind(db::now())
    .execute(&state.db)
    .await?;
    Ok(())
}

/// `main` of `repository` moved: its open pull requests are prepared again.
pub async fn main_moved(state: &AppState, group: i64, repository: &str) -> ApiResult<()> {
    sqlx::query(
        "UPDATE workshop_merges
         SET state = 'preparing', retry_at = NULL
         WHERE group_id = ? AND repository = ? AND state IN ('ready', 'waiting')",
    )
    .bind(group)
    .bind(repository)
    .execute(&state.db)
    .await?;
    tick(state).await
}

/// The group's model is back, or has a new key: what waited for it goes on.
pub async fn wake_group(state: &AppState, group: i64) -> ApiResult<()> {
    sqlx::query(
        "UPDATE workshop_merges
         SET state = 'preparing', resume_month = NULL
         WHERE group_id = ? AND state = 'waiting'",
    )
    .bind(group)
    .execute(&state.db)
    .await?;
    Ok(())
}

/// Prepares what is due. One preparation at a time; another tick finds
/// whatever this one left.
pub async fn tick(state: &AppState) -> ApiResult<()> {
    let Ok(_guard) = state.merge_changes.try_lock() else {
        return Ok(());
    };
    sqlx::query(
        "UPDATE workshop_merges
         SET state = 'preparing', resume_month = NULL
         WHERE resume_month IS NOT NULL AND resume_month != ?",
    )
    .bind(model::month())
    .execute(&state.db)
    .await?;
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{ROW}
         WHERE state = 'preparing' AND (retry_at IS NULL OR retry_at <= ?)
         ORDER BY id"
    )))
    .bind(db::now())
    .fetch_all(&state.db)
    .await?;
    for row in rows {
        if let Err(error) = prepare(state, &row).await {
            failed(state, &row, &error).await?;
        }
    }
    Ok(())
}

/// A preparation that failed waits for an admin on a missing key or the
/// cap, and otherwise tries again in a minute.
async fn failed(state: &AppState, row: &Row, error: &ApiError) -> ApiResult<()> {
    if model::waits_for_admins(error) {
        sqlx::query(
            "UPDATE workshop_merges
             SET state = 'waiting', message = ?, resume_month = ?
             WHERE id = ?",
        )
        .bind(&error.message)
        .bind((error.code == "workshop_cap").then(model::month))
        .bind(row.id)
        .execute(&state.db)
        .await?;
        if row.message.as_deref() != Some(&error.message) {
            say(
                state,
                row,
                &format!(
                    "Pull request [#{}]({}) in {} has conflicts with main. {}",
                    row.pull_number, row.url, row.repository, error.message
                ),
            )
            .await?;
        }
    } else {
        sqlx::query(
            "UPDATE workshop_merges
             SET message = ?, retry_at = ?
             WHERE id = ?",
        )
        .bind(format!(
            "{}. Trying again in a minute",
            error.message.trim_end_matches('.')
        ))
        .bind(db::now() + model::RETRY_MS)
        .bind(row.id)
        .execute(&state.db)
        .await?;
    }
    Ok(())
}

async fn slug(state: &AppState, group: i64) -> ApiResult<String> {
    Ok(sqlx::query_scalar(
        "SELECT slug
         FROM groups
         WHERE id = ?",
    )
    .bind(group)
    .fetch_one(&state.db)
    .await?)
}

async fn thread_of(state: &AppState, process: i64) -> ApiResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT thread_id
         FROM workshop_processes
         WHERE id = ?",
    )
    .bind(process)
    .fetch_one(&state.db)
    .await?)
}

/// Posts in the pull request's request, as Workshop.
async fn say(state: &AppState, row: &Row, text: &str) -> ApiResult<u32> {
    let thread = thread_of(state, row.process_id).await?;
    orchestrator::post(state, thread, None, text).await
}

/// What the pull request is now in the forge: open, merged or closed.
async fn settled(state: &AppState, row: &Row, slug: &str) -> ApiResult<bool> {
    let forge = state
        .forge
        .as_ref()
        .ok_or_else(|| ApiError::not_found("Forge"))?;
    let pull = forge
        .pull(slug, &row.repository, row.pull_number)
        .await
        .map_err(forge::api_error)?;
    let ended = if pull["merged"] == true {
        Some(("merged", "Merged into main"))
    } else if pull["state"] == "closed" {
        Some(("closed", "Closed without merging"))
    } else {
        None
    };
    if let Some((status, text)) = ended {
        sqlx::query(
            "UPDATE workshop_merges
             SET state = ?, message = ?
             WHERE id = ?",
        )
        .bind(status)
        .bind(text)
        .bind(row.id)
        .execute(&state.db)
        .await?;
    }
    Ok(ended.is_some())
}

async fn prepare(state: &AppState, row: &Row) -> ApiResult<()> {
    let slug = slug(state, row.group_id).await?;
    if settled(state, row, &slug).await? {
        return Ok(());
    }
    let forge = state.forge.as_ref().unwrap();
    let main = forge
        .branch_commit(&slug, &row.repository, "main")
        .await
        .map_err(forge::api_error)?
        .ok_or_else(|| ApiError::not_found("Repository main"))?;
    let Some(head) = forge
        .branch_commit(&slug, &row.repository, &row.branch)
        .await
        .map_err(forge::api_error)?
    else {
        sqlx::query(
            "UPDATE workshop_merges
             SET state = 'closed', message = 'Its branch is gone'
             WHERE id = ?",
        )
        .bind(row.id)
        .execute(&state.db)
        .await?;
        return Ok(());
    };
    // Nothing moved since the last preparation: it stands.
    if row.main.as_deref() == Some(&main)
        && row.head.as_deref() == Some(&head)
        && row.tree.is_some()
    {
        sqlx::query(
            "UPDATE workshop_merges
             SET state = 'ready', message = NULL
             WHERE id = ?",
        )
        .bind(row.id)
        .execute(&state.db)
        .await?;
        return Ok(());
    }
    let repo = Bare::fetch(state, &slug, &row.repository, &[&row.branch]).await?;
    let merged = repo.merge_tree(&main, &head).await?;
    let (tree, conflicts, explanation) = if merged.conflicts.is_empty() {
        (merged.tree, vec![], None)
    } else {
        let resolution = resolve(state, row, &repo, &merged).await?;
        let tree = repo.with_files(&merged.tree, &resolution.files).await?;
        (
            tree,
            merged.conflicts.iter().map(|c| c.path.clone()).collect(),
            Some(resolution.explanation),
        )
    };
    sqlx::query(
        "UPDATE workshop_merges
         SET state = 'ready', main = ?, head = ?, tree = ?, conflicts = ?, explanation = ?,
         message = NULL, retry_at = NULL
         WHERE id = ?",
    )
    .bind(&main)
    .bind(&head)
    .bind(&tree)
    .bind(serde_json::to_string(&conflicts).map_err(ApiError::internal)?)
    .bind(&explanation)
    .bind(row.id)
    .execute(&state.db)
    .await?;
    let text = match &explanation {
        None => format!(
            "Pull request [#{}]({}) in {} merges cleanly into main. Approve it to merge.",
            row.pull_number, row.url, row.repository
        ),
        Some(explanation) => format!(
            "Pull request [#{}]({}) in {} conflicts with main in {}. A merge that resolves \
             it is ready: {explanation}\n\nApprove it to merge.",
            row.pull_number,
            row.url,
            row.repository,
            conflicts.join(", ")
        ),
    };
    say(state, row, &text).await?;
    Ok(())
}

/// How the group's model resolves the conflicting files.
struct Resolution {
    files: Vec<(String, Option<String>, String)>,
    explanation: String,
}

/// Asks the group's model to resolve each conflicting file, from its three
/// sides: the common base, `main`, and the pull request.
async fn resolve(
    state: &AppState,
    row: &Row,
    repo: &Bare,
    merged: &Merged,
) -> ApiResult<Resolution> {
    if !model::has_key(state, row.group_id).await? {
        return Err(ApiError::new(
            axum::http::StatusCode::CONFLICT,
            "workshop_key",
            "Resolving them needs the group's model: an admin gives it a key",
        ));
    }
    let mut files = Vec::new();
    for conflict in &merged.conflicts {
        let mut sides = serde_json::Map::new();
        for (name, object) in [
            ("base", &conflict.base),
            ("main", &conflict.ours),
            ("pull_request", &conflict.theirs),
        ] {
            let text = match object {
                Some(object) => {
                    let bytes = repo.blob(object).await?;
                    if bytes.len() > MAX_CONFLICT_BYTES {
                        return Err(ApiError::invalid(format!(
                            "{} is too large to resolve here: merge it by hand",
                            conflict.path
                        )));
                    }
                    json!(String::from_utf8_lossy(&bytes))
                }
                None => serde_json::Value::Null,
            };
            sides.insert(name.into(), text);
        }
        files.push(json!({"path": conflict.path, "sides": sides}));
    }
    let tools = vec![Tool {
        name: "resolve".into(),
        description: "Give each conflicting file's merged content, or null to delete it, and \
                      explain the resolution in plain words"
            .into(),
        parameters: json!({"type": "object", "properties": {
            "files": {"type": "array", "items": {"type": "object", "properties": {
                "path": {"type": "string"},
                "content": {"type": ["string", "null"]}
            }, "required": ["path", "content"], "additionalProperties": false}},
            "explanation": {"type": "string"}
        }, "required": ["files", "explanation"], "additionalProperties": false}),
    }];
    let context = json!({
        "pull_request": {"title": row.title, "body": row.body, "repository": row.repository},
        "conflicts": files,
    });
    let mut history = vec![
        Message::text("system", include_str!("../prompts/merge.md")),
        Message::text("user", context.to_string()),
    ];
    let wanted: Vec<&str> = merged.conflicts.iter().map(|c| c.path.as_str()).collect();
    for _ in 0..model::step_cap(state, row.group_id).await? {
        let reply = model::call(state, row.group_id, &history, &tools).await?;
        let calls = reply.message.calls.clone();
        history.push(reply.message);
        for call in calls {
            let answer = (|| {
                if call.name != "resolve" {
                    return Err("Unknown tool".to_owned());
                }
                let explanation = call.arguments["explanation"]
                    .as_str()
                    .map(str::trim)
                    .filter(|e| !e.is_empty())
                    .ok_or("Explain the resolution")?;
                let given = call.arguments["files"].as_array().ok_or("Give the files")?;
                let mut resolved = Vec::new();
                for file in given {
                    let path = file["path"].as_str().ok_or("Each file needs its path")?;
                    let conflict = merged
                        .conflicts
                        .iter()
                        .find(|c| c.path == path)
                        .ok_or_else(|| format!("{path} has no conflict"))?;
                    let content = match &file["content"] {
                        serde_json::Value::Null => None,
                        serde_json::Value::String(text) => Some(text.clone()),
                        _ => return Err(format!("{path}: give text or null")),
                    };
                    if content
                        .as_ref()
                        .is_some_and(|c| c.contains("<<<<<<<") && c.contains(">>>>>>>"))
                    {
                        return Err(format!("{path} still has conflict markers"));
                    }
                    resolved.push((path.to_owned(), content, conflict.mode.clone()));
                }
                if let Some(missing) = wanted
                    .iter()
                    .find(|p| !resolved.iter().any(|(path, _, _)| path == *p))
                {
                    return Err(format!("Resolve {missing} too"));
                }
                Ok(Resolution {
                    files: resolved,
                    explanation: explanation.to_owned(),
                })
            })();
            match answer {
                Ok(resolution) => return Ok(resolution),
                Err(why) => history.push(Message::result(&call, why)),
            }
        }
    }
    Err(ApiError::new(
        axum::http::StatusCode::CONFLICT,
        "workshop_provider",
        "The group's model did not resolve the conflicts within its steps: merge it by hand",
    ))
}

/// Merges a ready pull request, as its requester or a group admin approves.
pub async fn approve(
    State(state): State<AppState>,
    viewer: Viewer,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
) -> ApiResult<Json<Merge>> {
    if viewer.agent().is_some() || !matches!(viewer.scope, crate::auth::Scope::Full) {
        return Err(ApiError::forbidden("Only a person approves a merge"));
    }
    let group = Membership::of(&state, &viewer, &slug).await?;
    let row: Row = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{ROW}
         WHERE id = ? AND group_id = ?"
    )))
    .bind(id)
    .bind(group.group_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found("Pull request"))?;
    let thread = thread_of(&state, row.process_id).await?;
    let requester: i64 = sqlx::query_scalar(
        "SELECT created_by
         FROM threads
         WHERE id = ?",
    )
    .bind(thread)
    .fetch_one(&state.db)
    .await?;
    if viewer.id != requester {
        group.require_admin()?;
    }
    {
        let _guard = state.merge_changes.lock().await;
        let row: Row = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "{ROW}
             WHERE id = ?"
        )))
        .bind(id)
        .fetch_one(&state.db)
        .await?;
        if row.state != "ready" {
            return Err(ApiError::conflict(
                "This pull request is not ready to merge",
            ));
        }
        merge(&state, &row, &group.slug, &viewer).await?;
    }
    let row: Row = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{ROW}
         WHERE id = ?"
    )))
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    if row.state == "preparing" {
        tick(&state).await?;
        return Err(ApiError::conflict(
            "Main or the pull request moved since its merge was prepared: it is being prepared again",
        ));
    }
    Ok(Json(to_api(&state, row).await?))
}

/// The prepared merge, made: only if neither `main` nor the head moved.
async fn merge(state: &AppState, row: &Row, slug: &str, viewer: &Viewer) -> ApiResult<()> {
    let forge = state
        .forge
        .as_ref()
        .ok_or_else(|| ApiError::not_found("Forge"))?;
    let (Some(main), Some(head), Some(tree)) = (&row.main, &row.head, &row.tree) else {
        return Err(ApiError::conflict(
            "This pull request is not ready to merge",
        ));
    };
    let again = || async {
        sqlx::query(
            "UPDATE workshop_merges
             SET state = 'preparing'
             WHERE id = ?",
        )
        .bind(row.id)
        .execute(&state.db)
        .await
    };
    let current_main = forge
        .branch_commit(slug, &row.repository, "main")
        .await
        .map_err(forge::api_error)?;
    let current_head = forge
        .branch_commit(slug, &row.repository, &row.branch)
        .await
        .map_err(forge::api_error)?;
    if current_main.as_deref() != Some(main) || current_head.as_deref() != Some(head) {
        again().await?;
        return Ok(());
    }
    let repo = Bare::fetch(state, slug, &row.repository, &[&row.branch]).await?;
    let message = format!(
        "Merge pull request #{} from {}\n\n{}\n\nApproved by {}.",
        row.pull_number, row.branch, row.title, viewer.display_name
    );
    let commit = repo.commit(tree, &[main, head], &message).await?;
    if repo.push(&commit, &row.branch).await.is_err() {
        again().await?;
        return Ok(());
    }
    match forge
        .fast_forward_pull(slug, &row.repository, row.pull_number, &commit)
        .await
    {
        Ok(()) => {}
        Err(e) if e.is_conflict() => {
            again().await?;
            return Ok(());
        }
        Err(e) => return Err(forge::api_error(e)),
    }
    sqlx::query(
        "UPDATE workshop_merges
         SET state = 'merged', approved_by = ?, message = 'Merged into main'
         WHERE id = ?",
    )
    .bind(viewer.id)
    .bind(row.id)
    .execute(&state.db)
    .await?;
    let paper: Option<i64> = sqlx::query_scalar(
        "SELECT id
         FROM papers
         WHERE group_id = ? AND repository = ?",
    )
    .bind(row.group_id)
    .bind(&row.repository)
    .fetch_optional(&state.db)
    .await?;
    crate::builds::refresh(state, row.group_id, &row.repository, paper).await?;
    let thread = thread_of(state, row.process_id).await?;
    let text = format!(
        "Merged pull request [#{}]({}) into {}'s main, as {} approved.",
        row.pull_number, row.url, row.repository, viewer.display_name
    );
    let number = say(state, row, &text).await?;
    let requester: i64 = sqlx::query_scalar(
        "SELECT created_by
         FROM threads
         WHERE id = ?",
    )
    .bind(thread)
    .fetch_one(&state.db)
    .await?;
    if requester != viewer.id {
        notifications::notify(
            state,
            &[requester],
            &Notice::new(NotificationKind::ReviewMerged)
                .at(row.group_id, thread, Some(number))
                .by(viewer.id, None)
                .text(&text),
        )
        .await?;
    }
    orchestrator::wake(state, thread).await?;
    Ok(())
}

async fn to_api(state: &AppState, row: Row) -> ApiResult<Merge> {
    Ok(Merge {
        id: row.id,
        process: row.process_id,
        repository: row.repository,
        number: row.pull_number,
        url: row.url,
        title: row.title,
        body: row.body,
        state: row.state,
        conflicts: serde_json::from_str(&row.conflicts).map_err(ApiError::internal)?,
        explanation: row.explanation,
        message: row.message,
        approved_by: match row.approved_by {
            Some(id) => Some(crate::workshop::user(state, id).await?),
            None => None,
        },
    })
}

/// The pull requests of a request's processes, newest first.
pub(crate) async fn of_thread(state: &AppState, thread: i64) -> ApiResult<Vec<Merge>> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{ROW}
         WHERE process_id IN (SELECT id
         FROM workshop_processes
         WHERE thread_id = ?)
         ORDER BY id DESC"
    )))
    .bind(thread)
    .fetch_all(&state.db)
    .await?;
    let mut merges = Vec::new();
    for row in rows {
        merges.push(to_api(state, row).await?);
    }
    Ok(merges)
}

/// The group's open pull requests, and the latest settled ones, for the
/// board.
pub(crate) async fn of_group(state: &AppState, group: i64) -> ApiResult<serde_json::Value> {
    #[derive(sqlx::FromRow, serde::Serialize)]
    struct BoardMerge {
        id: i64,
        repository: String,
        number: i64,
        url: String,
        title: String,
        state: String,
        message: Option<String>,
        thread: i64,
    }
    let rows: Vec<BoardMerge> = sqlx::query_as(
        "SELECT m.id, m.repository, m.pull_number AS number, m.url, m.title, m.state,
         m.message, t.number AS thread
         FROM workshop_merges m
         JOIN workshop_processes p ON p.id = m.process_id
         JOIN threads t ON t.id = p.thread_id
         WHERE m.group_id = ?
         ORDER BY m.state IN ('merged', 'closed'), m.id DESC
         LIMIT 20",
    )
    .bind(group)
    .fetch_all(&state.db)
    .await?;
    Ok(json!(rows))
}

// ── Git's plumbing ─────────────────────────────────────────────────────

/// A conflicting file, with the objects of its three sides.
struct Conflict {
    path: String,
    mode: String,
    base: Option<String>,
    ours: Option<String>,
    theirs: Option<String>,
}

/// What `git merge-tree` made: a tree, with conflict markers in the
/// conflicting files.
struct Merged {
    tree: String,
    conflicts: Vec<Conflict>,
}

/// A bare copy of one of a group's repositories, in the hub's data folder.
struct Bare {
    dir: PathBuf,
    url: String,
    header: String,
}

impl Bare {
    /// The copy, with `main` and `branches` fetched from the forge.
    async fn fetch(state: &AppState, slug: &str, name: &str, branches: &[&str]) -> ApiResult<Bare> {
        let forge = state
            .forge
            .as_ref()
            .ok_or_else(|| ApiError::not_found("Forge"))?;
        let (url, header) = forge.git_remote(slug, name);
        let dir = state
            .config
            .data_dir
            .join("merges")
            .join(slug)
            .join(format!("{name}.git"));
        let repo = Bare { dir, url, header };
        if !repo.dir.join("HEAD").exists() {
            tokio::fs::create_dir_all(&repo.dir)
                .await
                .map_err(ApiError::internal)?;
            repo.git(&["init", "-q", "--bare"], None).await?;
        }
        let mut refspecs = vec!["+refs/heads/main:refs/heads/main".to_owned()];
        for branch in branches {
            refspecs.push(format!("+refs/heads/{branch}:refs/heads/{branch}"));
        }
        let mut args = vec!["fetch", "-q", "--no-tags", &repo.url];
        args.extend(refspecs.iter().map(String::as_str));
        repo.remote(&args).await?;
        Ok(repo)
    }

    fn command(&self, args: &[&str]) -> tokio::process::Command {
        let mut command = tokio::process::Command::new("git");
        command
            .arg("--git-dir")
            .arg(&self.dir)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", "Workshop")
            .env("GIT_AUTHOR_EMAIL", "workshop@stemma.invalid")
            .env("GIT_COMMITTER_NAME", "Workshop")
            .env("GIT_COMMITTER_EMAIL", "workshop@stemma.invalid")
            .kill_on_drop(true);
        command
    }

    /// Runs git on the copy; what it printed, or why it failed.
    async fn git(&self, args: &[&str], input: Option<&[u8]>) -> ApiResult<Vec<u8>> {
        let (status, stdout, stderr) = self.run(self.command(args), input).await?;
        if !status.success() {
            return Err(ApiError::internal(format!(
                "git {} failed: {}",
                args.first().unwrap_or(&""),
                String::from_utf8_lossy(&stderr).trim()
            )));
        }
        Ok(stdout)
    }

    /// Runs git with the forge's credentials.
    async fn remote(&self, args: &[&str]) -> ApiResult<Vec<u8>> {
        let mut command = self.command(&[]);
        command
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "http.extraHeader")
            .env("GIT_CONFIG_VALUE_0", &self.header)
            .args(args);
        let (status, stdout, stderr) = self.run(command, None).await?;
        if !status.success() {
            let text = String::from_utf8_lossy(&stderr).replace(&self.header, "[hidden]");
            return Err(ApiError::new(
                axum::http::StatusCode::BAD_GATEWAY,
                "forge",
                format!("The hub could not reach the forge's git: {}", text.trim()),
            ));
        }
        Ok(stdout)
    }

    async fn run(
        &self,
        mut command: tokio::process::Command,
        input: Option<&[u8]>,
    ) -> ApiResult<(std::process::ExitStatus, Vec<u8>, Vec<u8>)> {
        use tokio::io::AsyncWriteExt;
        command
            .stdin(if input.is_some() {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            })
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn().map_err(|e| {
            ApiError::internal(format!("The hub needs git to merge pull requests: {e}"))
        })?;
        if let Some(input) = input {
            let mut stdin = child.stdin.take().unwrap();
            stdin.write_all(input).await.map_err(ApiError::internal)?;
            drop(stdin);
        }
        let output = child.wait_with_output().await.map_err(ApiError::internal)?;
        Ok((output.status, output.stdout, output.stderr))
    }

    /// Merges `theirs` into `ours` without a checkout, leaving conflict
    /// markers in the files where both changed.
    async fn merge_tree(&self, ours: &str, theirs: &str) -> ApiResult<Merged> {
        let (status, stdout, stderr) = self
            .run(
                self.command(&["merge-tree", "--write-tree", "-z", ours, theirs]),
                None,
            )
            .await?;
        match status.code() {
            Some(0 | 1) => {}
            _ => {
                return Err(ApiError::internal(format!(
                    "git merge-tree failed: {}",
                    String::from_utf8_lossy(&stderr).trim()
                )));
            }
        }
        parse_merge_tree(&stdout)
    }

    async fn blob(&self, object: &str) -> ApiResult<Vec<u8>> {
        self.git(&["cat-file", "blob", object], None).await
    }

    /// `tree` with `files` in place of its own: each with its content, or
    /// deleted.
    async fn with_files(
        &self,
        tree: &str,
        files: &[(String, Option<String>, String)],
    ) -> ApiResult<String> {
        // One preparation runs at a time, under the hub's merge lock.
        let index = self.dir.join("stemma-merge-index");
        let _ = tokio::fs::remove_file(&index).await;
        let indexed = |args: &[&str]| {
            let mut command = self.command(args);
            command.env("GIT_INDEX_FILE", &index);
            command
        };
        let (status, _, stderr) = self.run(indexed(&["read-tree", tree]), None).await?;
        ensure(status, &stderr, "read-tree")?;
        for (path, content, mode) in files {
            let (status, _, stderr) = match content {
                None => {
                    self.run(
                        indexed(&["update-index", "--force-remove", "--", path]),
                        None,
                    )
                    .await?
                }
                Some(text) => {
                    let blob = self
                        .git(&["hash-object", "-w", "--stdin"], Some(text.as_bytes()))
                        .await?;
                    let blob = String::from_utf8_lossy(&blob).trim().to_owned();
                    let info = format!("{mode},{blob},{path}");
                    self.run(
                        indexed(&["update-index", "--add", "--cacheinfo", &info]),
                        None,
                    )
                    .await?
                }
            };
            ensure(status, &stderr, "update-index")?;
        }
        let (status, stdout, stderr) = self.run(indexed(&["write-tree"]), None).await?;
        let _ = tokio::fs::remove_file(&index).await;
        ensure(status, &stderr, "write-tree")?;
        Ok(String::from_utf8_lossy(&stdout).trim().to_owned())
    }

    /// A commit of `tree` with `parents`, as Workshop.
    async fn commit(&self, tree: &str, parents: &[&String], message: &str) -> ApiResult<String> {
        let mut args = vec!["commit-tree", tree];
        for parent in parents {
            args.extend(["-p", parent.as_str()]);
        }
        args.extend(["-m", message]);
        let out = self.git(&args, None).await?;
        Ok(String::from_utf8_lossy(&out).trim().to_owned())
    }

    /// Pushes `commit` to `branch` in the forge, as a fast-forward.
    async fn push(&self, commit: &str, branch: &str) -> ApiResult<()> {
        let refspec = format!("{commit}:refs/heads/{branch}");
        self.remote(&["push", "-q", &self.url, &refspec]).await?;
        Ok(())
    }
}

fn ensure(status: std::process::ExitStatus, stderr: &[u8], what: &str) -> ApiResult<()> {
    if status.success() {
        Ok(())
    } else {
        Err(ApiError::internal(format!(
            "git {what} failed: {}",
            String::from_utf8_lossy(stderr).trim()
        )))
    }
}

/// Reads `git merge-tree --write-tree -z`: the tree, then each conflicting
/// file's stages (`<mode> <object> <stage>\t<path>`), up to an empty entry.
fn parse_merge_tree(out: &[u8]) -> ApiResult<Merged> {
    let text = String::from_utf8_lossy(out);
    let mut fields = text.split('\0');
    let tree = fields
        .next()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| ApiError::internal("git merge-tree gave no tree"))?;
    let mut conflicts: Vec<Conflict> = Vec::new();
    for field in fields {
        if field.is_empty() {
            break;
        }
        let Some((info, path)) = field.split_once('\t') else {
            break;
        };
        let parts: Vec<&str> = info.split(' ').collect();
        let [mode, object, stage] = parts[..] else {
            break;
        };
        let at = match conflicts.iter().position(|c| c.path == path) {
            Some(at) => at,
            None => {
                conflicts.push(Conflict {
                    path: path.to_owned(),
                    mode: mode.to_owned(),
                    base: None,
                    ours: None,
                    theirs: None,
                });
                conflicts.len() - 1
            }
        };
        let conflict = &mut conflicts[at];
        match stage {
            "1" => conflict.base = Some(object.to_owned()),
            "2" => {
                conflict.ours = Some(object.to_owned());
                conflict.mode = mode.to_owned();
            }
            "3" => {
                conflict.theirs = Some(object.to_owned());
                if conflict.ours.is_none() {
                    conflict.mode = mode.to_owned();
                }
            }
            _ => {}
        }
    }
    Ok(Merged { tree, conflicts })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_conflicts_of_a_merge() {
        let out = b"4b825dc642cb6eb9a060e54bf8d69288fbee4904\0\
100644 1111111111111111111111111111111111111111 1\tA.lean\0\
100644 2222222222222222222222222222222222222222 2\tA.lean\0\
100755 3333333333333333333333333333333333333333 3\tA.lean\0\
100644 4444444444444444444444444444444444444444 3\tB.lean\0\
\0Auto-merging A.lean\0CONFLICT (content): Merge conflict in A.lean\0";
        let merged = parse_merge_tree(out).unwrap();
        assert_eq!(merged.tree, "4b825dc642cb6eb9a060e54bf8d69288fbee4904");
        assert_eq!(merged.conflicts.len(), 2);
        let a = &merged.conflicts[0];
        assert_eq!(
            (a.path.as_str(), a.mode.as_str(), a.base.as_deref()),
            (
                "A.lean",
                "100644",
                Some("1111111111111111111111111111111111111111")
            )
        );
        assert_eq!(merged.conflicts[1].ours, None);
        assert!(
            parse_merge_tree(b"4b825dc642cb6eb9a060e54bf8d69288fbee4904\n")
                .unwrap()
                .conflicts
                .is_empty()
        );
    }
}
