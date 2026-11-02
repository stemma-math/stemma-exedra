//! Durable wakes, bounded turns and the workshop's internal tools.
use crate::{
    api::{Json, Membership, forum},
    auth::{Scope, Viewer},
    db,
    error::{ApiError, ApiResult},
    model::{self, Message, Tool},
    notifications::{self, Notice},
    state::AppState,
    workshop,
};
use axum::extract::{Path, State};
use serde_json::{Value, json};
use stemma_core::{
    api::NotificationKind,
    workshop::{Ask, RecipeFile},
};

pub async fn recover(state: &AppState) -> ApiResult<()> {
    sqlx::query(
        "UPDATE workshop_requests
         SET thinking = 0, wake = 1
         WHERE thinking = 1",
    )
    .execute(&state.db)
    .await?;
    Ok(())
}
pub async fn wake_group(state: &AppState, group: i64) -> ApiResult<()> {
    sqlx::query(
        "UPDATE workshop_requests
         SET wake = 1
         WHERE orchestrated = 1 AND state IN ('waiting', 'running')
         AND thread_id IN (SELECT id
         FROM threads
         WHERE group_id = ?)",
    )
    .bind(group)
    .execute(&state.db)
    .await?;
    crate::merges::wake_group(state, group).await
}
pub async fn wake(state: &AppState, thread: i64) -> ApiResult<()> {
    sqlx::query(
        "UPDATE workshop_requests
         SET wake = 1
         WHERE thread_id = ? AND orchestrated = 1
         AND state IN ('waiting', 'running')",
    )
    .bind(thread)
    .execute(&state.db)
    .await?;
    Ok(())
}
pub async fn wake_message(state: &AppState, thread: i64, author: i64) -> ApiResult<()> {
    sqlx::query(
        "UPDATE workshop_requests
         SET wake = 1
         WHERE thread_id = ? AND orchestrated = 1
         AND state IN ('waiting', 'running')
         AND EXISTS (SELECT 1
         FROM threads t
         JOIN memberships m ON m.group_id = t.group_id
         WHERE t.id = workshop_requests.thread_id AND m.user_id = ?
         AND (t.created_by = m.user_id OR m.role = 'admin'))",
    )
    .bind(thread)
    .bind(author)
    .execute(&state.db)
    .await?;
    dispatch(state).await
}

pub async fn dispatch(state: &AppState) -> ApiResult<()> {
    sqlx::query(
        "UPDATE workshop_requests
         SET wake = 1, resume_month = NULL
         WHERE resume_month IS NOT NULL AND resume_month != ?",
    )
    .bind(model::month())
    .execute(&state.db)
    .await?;
    sqlx::query(
        "UPDATE workshop_requests
         SET wake = 1, retry_at = NULL
         WHERE retry_at <= ?",
    )
    .bind(db::now())
    .execute(&state.db)
    .await?;
    // Each scheduled slot has its own wait deadline. Marking and waking are
    // one transaction, so competing dispatch tasks cannot replay the event.
    let mut tx = state.db.begin().await?;
    let expiring: Vec<i64> = sqlx::query_scalar(
        "UPDATE workshop_processes
         SET expiry_woken = 1
         WHERE expiry_woken = 0 AND expires_at <= ?
         AND work_id IN (SELECT id
         FROM work_leases
         WHERE state = 'queued')
         RETURNING thread_id",
    )
    .bind(db::now() + 60_000)
    .fetch_all(&mut *tx)
    .await?;
    for thread in expiring {
        sqlx::query(
            "UPDATE workshop_requests SET wake = 1
             WHERE thread_id = ? AND orchestrated = 1
             AND state IN ('waiting', 'running')",
        )
        .bind(thread)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    // Expiration wakes once before the deadline, even if no process was launched.
    sqlx::query(
        "UPDATE workshop_requests
         SET wake = 1, expires_at = NULL
         WHERE orchestrated = 1 AND expires_at <= ?
         AND state IN ('waiting', 'running')",
    )
    .bind(db::now() + 60_000)
    .execute(&state.db)
    .await?;
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT r.thread_id, t.group_id
         FROM workshop_requests r
         JOIN threads t ON t.id = r.thread_id
         WHERE r.orchestrated = 1 AND r.wake = 1 AND r.thinking = 0
         AND r.state IN ('waiting', 'running')
         ORDER BY r.created_at",
    )
    .fetch_all(&state.db)
    .await?;
    for (thread, group) in rows {
        // The conditional write makes claiming atomic across dispatch tasks.
        let changed = sqlx::query(
            "UPDATE workshop_requests
             SET thinking = 1, wake = 0
             WHERE thread_id = ? AND thinking = 0 AND wake = 1
             AND (SELECT count(*)
             FROM workshop_requests r
             JOIN threads t ON t.id = r.thread_id
             WHERE t.group_id = ? AND r.thinking = 1) < 3",
        )
        .bind(thread)
        .bind(group)
        .execute(&state.db)
        .await?
        .rows_affected();
        if changed == 0 {
            continue;
        }
        let state = state.clone();
        state.clone().sockets.spawn(async move {
            let result = tokio::select! {
                _ = state.shutdown.cancelled() => Ok(()),
                result = turn(&state, thread, group) => result,
            };
            if let Err(error) = result {
                let _ = failed(&state, thread, &error).await;
            }
            let _ = sqlx::query(
                "UPDATE workshop_requests
                 SET thinking = 0
                 WHERE thread_id = ?",
            )
            .bind(thread)
            .execute(&state.db)
            .await;
        });
    }
    Ok(())
}
/// Says why a turn stopped, and when the next one comes: next month for the
/// cap, a new key for a missing one, a minute later for anything else.
async fn failed(state: &AppState, thread: i64, error: &ApiError) -> ApiResult<()> {
    if model::waits_for_admins(error) {
        wait(state, thread, None, &error.message).await?;
    } else {
        let text = format!(
            "{}. Trying again in a minute",
            error.message.trim_end_matches('.')
        );
        wait(state, thread, None, &text).await?;
    }
    sqlx::query(
        "UPDATE workshop_requests
         SET resume_month = ?, retry_at = ?
         WHERE thread_id = ?",
    )
    .bind((error.code == "workshop_cap").then(model::month))
    .bind((!model::waits_for_admins(error)).then(|| db::now() + model::RETRY_MS))
    .bind(thread)
    .execute(&state.db)
    .await?;
    Ok(())
}
pub(crate) async fn viewer(state: &AppState, thread: i64) -> ApiResult<Viewer> {
    let id: i64 = sqlx::query_scalar(
        "SELECT created_by
         FROM threads
         WHERE id = ?",
    )
    .bind(thread)
    .fetch_one(&state.db)
    .await?;
    let user = workshop::user(state, id).await?;
    Ok(Viewer {
        id,
        username: user.username,
        display_name: user.display_name,
        hub_admin: false,
        with_token: false,
        personal_token: false,
        scope: Scope::Full,
    })
}
pub(crate) async fn request(
    state: &AppState,
    thread: i64,
) -> ApiResult<stemma_core::workshop::Request> {
    let (group, number): (i64, i64) = sqlx::query_as(
        "SELECT group_id, number
         FROM threads
         WHERE id = ?",
    )
    .bind(thread)
    .fetch_one(&state.db)
    .await?;
    let slug = Membership::of_group(state, group).await?.slug;
    Ok(workshop::read(
        State(state.clone()),
        viewer(state, thread).await?,
        Path((slug, number as u32)),
    )
    .await?
    .0)
}
/// Posts in a request's thread as Workshop, with the model that wrote it.
pub(crate) async fn post(
    state: &AppState,
    thread: i64,
    model: Option<&str>,
    text: &str,
) -> ApiResult<u32> {
    let group: i64 = sqlx::query_scalar(
        "SELECT group_id
         FROM threads
         WHERE id = ?",
    )
    .bind(thread)
    .fetch_one(&state.db)
    .await?;
    let group = Membership::of_group(state, group).await?;
    let who = viewer(state, thread).await?;
    // The hub's own steps name no model; an empty one still marks Workshop.
    let model = model.unwrap_or("");
    let text = crate::api::required_text(text, "The workshop's message", 100_000)?;
    let mut tx = state.db.begin().await?;
    let (number, stored) = forum::insert_message(
        &mut tx,
        &group,
        thread,
        who.id,
        None,
        None,
        &stemma_core::secrets::hide(&text),
        None,
        &[],
        true,
    )
    .await?;
    sqlx::query(
        "UPDATE messages
         SET orchestrator_model = ?, author_id = NULL
         WHERE thread_id = ? AND number = ?",
    )
    .bind(model)
    .bind(thread)
    .bind(number)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    forum::publish_post(state, &group, thread, number, &stored).await?;
    Ok(number)
}
/// Says in the thread what the request waits for, once, and keeps it.
pub(crate) async fn wait(
    state: &AppState,
    thread: i64,
    model: Option<&str>,
    text: &str,
) -> ApiResult<()> {
    let old: Option<String> = sqlx::query_scalar(
        "SELECT waiting
         FROM workshop_requests
         WHERE thread_id = ?",
    )
    .bind(thread)
    .fetch_one(&state.db)
    .await?;
    if old.as_deref() != Some(text) {
        post(state, thread, model, text).await?;
    }
    waits_for(state, thread, text).await
}
/// Keeps what the request waits for, for its header and the board only.
async fn waits_for(state: &AppState, thread: i64, text: &str) -> ApiResult<()> {
    sqlx::query(
        "UPDATE workshop_requests
         SET waiting = ?
         WHERE thread_id = ?",
    )
    .bind(text)
    .bind(thread)
    .execute(&state.db)
    .await?;
    Ok(())
}
pub(crate) async fn close(
    state: &AppState,
    thread: i64,
    model: Option<&str>,
    done: bool,
    text: &str,
) -> ApiResult<()> {
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1
         FROM workshop_processes p
         JOIN work_leases w ON w.id = p.work_id
         WHERE p.thread_id = ? AND w.state IN ('queued', 'running'))",
    )
    .bind(thread)
    .fetch_one(&state.db)
    .await?;
    if active {
        return Err(ApiError::invalid("Wait for the outstanding processes"));
    }
    let number = post(state, thread, model, text).await?;
    sqlx::query(
        "UPDATE workshop_requests
         SET state = ?, waiting = NULL, wake = 0, next_at = NULL
         WHERE thread_id = ? AND state IN ('waiting', 'running')",
    )
    .bind(if done { "done" } else { "failed" })
    .bind(thread)
    .execute(&state.db)
    .await?;
    let req = request(state, thread).await?;
    let group: i64 = sqlx::query_scalar(
        "SELECT group_id
         FROM threads
         WHERE id = ?",
    )
    .bind(thread)
    .fetch_one(&state.db)
    .await?;
    notifications::notify(
        state,
        &[req.requester.id],
        &Notice::new(if done {
            NotificationKind::RequestDone
        } else {
            NotificationKind::RequestFailed
        })
        .at(group, thread, Some(number))
        .text(text),
    )
    .await?;
    Ok(())
}
/// What Workshop chooses a recipe by: its manifest, without its files.
fn summaries(recipes: &[RecipeFile]) -> Value {
    json!(
        recipes
            .iter()
            .map(|file| match &file.recipe {
                Some(recipe) => json!({
                    "name": file.name, "version": file.version,
                    "title": recipe.manifest.title, "summary": recipe.manifest.summary,
                    "hint": recipe.manifest.hint, "inputs": recipe.manifest.inputs,
                    "outputs": recipe.outputs(), "permissions": recipe.manifest.permissions,
                    "timeout_minutes": recipe.manifest.timeout_minutes,
                }),
                None => json!({"name": file.name, "error": file.error}),
            })
            .collect::<Vec<_>>()
    )
}
fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Tool {
    Tool {
        name: name.into(),
        description: description.into(),
        parameters: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
    }
}
fn definitions() -> Vec<Tool> {
    let mut tools = crate::workshop_read::definitions();
    tools.extend([
        tool(
            "recipes",
            "List the group's recipes: what each is, takes and delivers",
            json!({}),
            &[],
        ),
        tool(
            "recipe",
            "Read a recipe, with its run.sh and files",
            json!({"name": {"type": "string"}}),
            &["name"],
        ),
        tool(
            "machines",
            "Connected eligible machines, owners, runners, models, version and room",
            json!({
                "recipe": {"type": "string"}, "runner": {"type": "string"},
                "model": {"type": ["string", "null"]}
            }),
            &["recipe", "runner"],
        ),
        tool(
            "launch",
            "Offer a recipe run under normal machine approvals",
            json!({
                "recipe": {"type": "string"},
                "inputs": {"type": "object", "additionalProperties": {"type": "string"}},
                "runner": {"type": "string", "enum": ["claude", "codex", "opencode"]},
                "model": {"type": ["string", "null"]},
                "assignment": {"type": "object", "properties": {
                    "kind": {"enum": ["mine", "member", "anyone"]},
                    "user": {"type": "integer"}
                }, "required": ["kind"]}
            }),
            &["recipe", "inputs", "runner", "assignment"],
        ),
        tool(
            "post",
            "Post progress or say what the request waits for, once: posting ends this turn",
            json!({"message": {"type": "string"}}),
            &["message"],
        ),
        tool(
            "close",
            "Close as done or failed, with a message",
            json!({"done": {"type": "boolean"}, "message": {"type": "string"}}),
            &["done", "message"],
        ),
    ]);
    tools
}
async fn turn(state: &AppState, thread: i64, group: i64) -> ApiResult<()> {
    let req = request(state, thread).await?;
    let member = Membership::of_group(state, group).await?;
    let v = viewer(state, thread).await?;
    let messages = forum::load_messages(state, &member, thread, 1, u32::MAX).await?;
    let recipes = crate::recipes::list(state, &member.slug).await?;
    let board = status(state, group).await?;
    let mut history = vec![
        Message::text("system", include_str!("../prompts/workshop.md")),
        Message::text(
            "user",
            json!({"request":req,"thread":messages,"recipes":summaries(&recipes),"board":board})
                .to_string(),
        ),
    ];
    let tools = definitions();
    let mut launched = false;
    // One message per turn: a model that posts and then goes on, or ends
    // with text too, would answer each wake several times.
    let mut posted = false;
    let mut exhausted = true;
    let limit = model::step_cap(state, group).await?;
    for _ in 0..limit {
        if !open(state, thread).await? {
            return Ok(());
        }
        let reply = model::call(state, group, &history, &tools).await?;
        let calls = reply.message.calls.clone();
        let text = reply.message.text.clone();
        history.push(reply.message);
        if calls.is_empty() {
            if !text.trim().is_empty() {
                post(state, thread, Some(&reply.model), &text).await?;
            }
            exhausted = false;
            break;
        }
        for call in calls {
            if !open(state, thread).await? {
                return Ok(());
            }
            let result = match call.name.as_str() {
                "recipes" => Ok(summaries(&recipes)),
                "recipe" => {
                    crate::recipes::read(state, &member.slug, arg(&call.arguments, "name")?)
                        .await
                        .map(|r| json!(r))
                }
                "machines" => machines(state, thread, &member.slug, &call.arguments).await,
                "launch" => {
                    let mut ask = req.ask.clone();
                    ask.recipe = arg(&call.arguments, "recipe")?.into();
                    ask.inputs = serde_json::from_value(call.arguments["inputs"].clone())
                        .map_err(|e| ApiError::invalid(format!("{e}")))?;
                    ask.runner = arg(&call.arguments, "runner")?.into();
                    ask.model = call.arguments["model"].as_str().map(str::to_owned);
                    ask.assignment = serde_json::from_value(call.arguments["assignment"].clone())
                        .map_err(|e| ApiError::invalid(format!("{e}")))?;
                    let result = launch(state, thread, ask).await;
                    if result.is_ok() {
                        launched = true;
                    }
                    result.map(|id| json!({"process":id,"state":"offered"}))
                }
                "post" if posted => Err(ApiError::invalid(
                    "This turn has posted already; its turn ends here",
                )),
                "post" => post(
                    state,
                    thread,
                    Some(&reply.model),
                    arg(&call.arguments, "message")?,
                )
                .await
                .map(|_| {
                    posted = true;
                    json!("Posted; this turn ends here")
                }),
                "close" => close(
                    state,
                    thread,
                    Some(&reply.model),
                    call.arguments["done"]
                        .as_bool()
                        .ok_or_else(|| ApiError::invalid("Missing done"))?,
                    arg(&call.arguments, "message")?,
                )
                .await
                .map(|_| json!("Closed")),
                name => {
                    crate::workshop_read::call(
                        state,
                        &v,
                        &member.slug,
                        name,
                        call.arguments.clone(),
                    )
                    .await
                }
            };
            history.push(Message::result(
                &call,
                match result {
                    Ok(value) => value.to_string(),
                    Err(e) => json!({"error":e.message}).to_string(),
                },
            ));
            if !open(state, thread).await? {
                return Ok(());
            }
        }
        if launched {
            break;
        }
        if posted {
            exhausted = false;
            break;
        }
    }
    if launched {
        waits_for(
            state,
            thread,
            "Waiting for the launched process and its machine's approval",
        )
        .await?;
    } else {
        let active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1
             FROM workshop_processes p
             JOIN work_leases w ON w.id = p.work_id
             WHERE p.thread_id = ?1 AND w.state IN ('queued', 'running'))",
        )
        .bind(thread)
        .fetch_one(&state.db)
        .await?;
        let waiting = if req.next_at.is_some_and(|at| at > db::now()) && req.processes.is_empty() {
            "Waiting for the scheduled time"
        } else if active {
            "Waiting for the outstanding processes"
        } else if exhausted {
            "Waiting for guidance from the requester or an admin; this turn reached its step limit"
        } else {
            "Waiting for guidance from the requester or an admin before launching more work"
        };
        waits_for(state, thread, waiting).await?;
    }
    Ok(())
}
async fn open(state: &AppState, thread: i64) -> ApiResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT state IN ('waiting', 'running')
         FROM workshop_requests
         WHERE thread_id = ?",
    )
    .bind(thread)
    .fetch_optional(&state.db)
    .await?
    .unwrap_or(false))
}
fn arg<'a>(args: &'a Value, name: &str) -> ApiResult<&'a str> {
    args[name]
        .as_str()
        .ok_or_else(|| ApiError::invalid(format!("Missing {name}")))
}

/// A launch creates a process on the existing request. Each process retains
/// its own recipe and inputs.
pub(crate) async fn launch(state: &AppState, thread: i64, ask: Ask) -> ApiResult<i64> {
    let _guard = state.process_changes.lock().await;
    if !open(state, thread).await? {
        return Err(ApiError::invalid("The request is closed"));
    }
    let group: i64 = sqlx::query_scalar(
        "SELECT group_id
         FROM threads
         WHERE id = ?",
    )
    .bind(thread)
    .fetch_one(&state.db)
    .await?;
    let slug = Membership::of_group(state, group).await?.slug;
    let file = crate::recipes::read(state, &slug, &ask.recipe).await?;
    validate(state, group, &file, &ask).await?;
    let original = request(state, thread).await?;
    if original.ask.schedule.at.is_some_and(|at| at > db::now()) && original.processes.is_empty() {
        return Err(ApiError::invalid("Wait for the request's scheduled time"));
    }
    if original.processes.iter().any(|process| {
        ["waiting", "running"].contains(&process.state.as_str())
            && process.recipe.version == file.version
            && process.inputs == ask.inputs
    }) {
        return Err(ApiError::conflict(
            "A process for these recipe inputs is already outstanding",
        ));
    }
    #[derive(sqlx::FromRow)]
    struct Previous {
        id: i64,
        retry_of: Option<i64>,
        pushed: bool,
        recipe: Option<String>,
        ask: Option<String>,
    }
    let previous: Option<Previous> = sqlx::query_as(
        "SELECT p.id, p.retry_of, p.pushed, p.recipe, p.ask
         FROM workshop_processes p
         JOIN work_leases w ON w.id = p.work_id
         WHERE p.thread_id = ? AND w.state = 'failed'
         AND p.id = (SELECT max(latest.id)
         FROM workshop_processes latest
         WHERE latest.thread_id = p.thread_id)
         ORDER BY p.id DESC LIMIT 1",
    )
    .bind(thread)
    .fetch_optional(&state.db)
    .await?;
    let mut retry_of = None;
    if let Some(Previous {
        id,
        retry_of: retried,
        pushed,
        recipe: Some(raw_recipe),
        ask: Some(raw_ask),
    }) = previous
    {
        let old_file: RecipeFile = serde_json::from_str(&raw_recipe).map_err(ApiError::internal)?;
        let old_ask: Ask = serde_json::from_str(&raw_ask).map_err(ApiError::internal)?;
        if old_file.version == file.version && old_ask.inputs == ask.inputs {
            if retried.is_some() || pushed {
                return Err(ApiError::invalid(
                    "This process cannot be retried again or after a push; close with a reason",
                ));
            }
            retry_of = Some(id);
        }
    }
    let mut tx = state.db.begin().await?;
    let work = sqlx::query(
        "INSERT INTO work_leases (group_id, kind, created_at) VALUES (?, 'process', ?)",
    )
    .bind(group)
    .bind(db::now())
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();
    let id = sqlx::query(
        "INSERT INTO workshop_processes
         (thread_id, work_id, slot, expires_at, recipe, ask, retry_of)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(thread)
    .bind(work)
    .bind(db::now())
    .bind(db::now() + i64::from(ask.wait_hours) * 3_600_000)
    .bind(serde_json::to_string(&file).map_err(ApiError::internal)?)
    .bind(serde_json::to_string(&ask).map_err(ApiError::internal)?)
    .bind(retry_of)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();
    if let Some(previous) = retry_of {
        sqlx::query(
            "INSERT INTO workshop_offers (process_id, machine_id, connection, decision)
             SELECT ?, machine_id, connection, 'ran'
             FROM workshop_offers
             WHERE process_id = ? AND decision = 'ran'",
        )
        .bind(id)
        .bind(previous)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "UPDATE workshop_requests
         SET state = 'waiting', waiting = NULL, expires_at = NULL
         WHERE thread_id = ?",
    )
    .bind(thread)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    drop(_guard);
    workshop::dispatch(state).await?;
    Ok(id)
}
async fn validate(state: &AppState, group: i64, file: &RecipeFile, ask: &Ask) -> ApiResult<()> {
    if !["claude", "codex", "opencode"].contains(&ask.runner.as_str()) {
        return Err(ApiError::invalid("Unknown runner"));
    }
    if let stemma_core::workshop::Assignment::Member { user } = ask.assignment {
        let valid: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1
             FROM memberships
             WHERE group_id = ? AND user_id = ? AND role IN ('member', 'admin'))",
        )
        .bind(group)
        .bind(user)
        .fetch_one(&state.db)
        .await?;
        if !valid {
            return Err(ApiError::invalid("Choose a group member"));
        }
    }
    workshop::check_inputs(state, group, file, &ask.inputs).await
}
async fn machines(state: &AppState, thread: i64, slug: &str, args: &Value) -> ApiResult<Value> {
    let req = request(state, thread).await?;
    let recipe = crate::recipes::read(state, slug, arg(args, "recipe")?).await?;
    let mut output = vec![];
    let live = state.build_machines.lock().await;
    for (id, m) in &live.0 {
        let scope = m.scope(slug);
        let member: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1
             FROM memberships
             WHERE group_id = (SELECT id
             FROM groups
             WHERE slug = ?)
             AND user_id = ? AND role IN ('admin', 'member'))",
        )
        .bind(slug)
        .bind(m.owner)
        .fetch_one(&state.db)
        .await?;
        if !member
            || m.sender.is_closed()
            || !scope.enabled
            || !scope.runners.contains_key(arg(args, "runner")?)
            || (m.owner != req.requester.id && scope.others != stemma_core::protocol::Others::Ask)
            || !recipe
                .recipe
                .as_ref()
                .is_some_and(|r| r.accepts_version(&m.version))
            || args["model"].as_str().is_some_and(|model| {
                scope
                    .runners
                    .get(arg(args, "runner").unwrap_or(""))
                    .map(String::as_str)
                    != Some(model)
            })
        {
            continue;
        }
        let running: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM work_leases
             WHERE holder = ? AND kind = 'process' AND state = 'running'",
        )
        .bind(id)
        .fetch_one(&state.db)
        .await?;
        if running >= i64::from(scope.concurrency) {
            continue;
        }
        output.push(json!({
            "owner": workshop::user(state, m.owner).await?, "machine": m.name,
            "connected": true, "runners": scope.runners, "version": m.version,
            "room": i64::from(scope.concurrency) - running, "others": scope.others
        }));
    }
    Ok(json!(output))
}
pub(crate) async fn status(state: &AppState, group: i64) -> ApiResult<Value> {
    let merges = crate::merges::of_group(state, group).await?;
    let turns: Vec<(i64, bool, Option<String>)> = sqlx::query_as(
        "SELECT t.number, r.thinking,
         coalesce(r.waiting, CASE r.state WHEN 'running' THEN 'A process is running'
         ELSE 'Waiting for work' END)
         FROM workshop_requests r
         JOIN threads t ON t.id = r.thread_id
         WHERE t.group_id = ? AND r.state IN ('waiting', 'running')",
    )
    .bind(group)
    .fetch_all(&state.db)
    .await?;
    let turns: Vec<Value> = turns
        .into_iter()
        .map(|(number, thinking, waiting)| {
            json!({"number": number, "thinking": thinking, "waiting": waiting})
        })
        .collect();
    let model = model::settings(state, group).await?.public();
    Ok(json!({"model": model, "merges": merges, "turns": turns}))
}
pub async fn board(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    Ok(Json(status(&state, group.group_id).await?))
}
