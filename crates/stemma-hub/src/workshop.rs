//! The board and process queue. The hub offers work; only machines approve it.
use crate::{
    api::{Json, Membership, forum, required_text},
    auth::Viewer,
    db,
    error::{ApiError, ApiResult},
    forge, lease,
    notifications::{self, Notice},
    recipes,
    state::AppState,
};
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use stemma_core::{
    api::{NotificationKind, Thread, ThreadKind, User},
    protocol::*,
    recipe::{InputKind, Repository},
    workshop::*,
};

#[derive(sqlx::FromRow)]
struct Row {
    thread_id: i64,
    recipe: String,
    ask: String,
    state: String,
    recurring: bool,
    paused: bool,
    next_at: Option<i64>,
}
const ROW: &str = "SELECT r.thread_id, r.recipe, r.ask, r.state, r.recurring, r.paused, r.next_at
                   FROM workshop_requests r";
fn decode<T: serde::de::DeserializeOwned>(s: &str) -> ApiResult<T> {
    serde_json::from_str(s).map_err(ApiError::internal)
}
fn encode<T: serde::Serialize>(v: &T) -> ApiResult<String> {
    serde_json::to_string(v).map_err(ApiError::internal)
}
async fn user(state: &AppState, id: i64) -> ApiResult<User> {
    let (username, display_name): (String, String) =
        sqlx::query_as("SELECT username, display_name FROM users WHERE id = ?")
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    Ok(User {
        id,
        username,
        display_name,
    })
}
async fn find(state: &AppState, group: i64, number: u32) -> ApiResult<Row> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{ROW} JOIN threads t ON t.id = r.thread_id WHERE t.group_id = ? AND t.number = ?"
    )))
    .bind(group)
    .bind(number)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found("Request"))
}
#[derive(Deserialize)]
pub struct Filter {
    state: Option<String>,
}
pub async fn list_recipes(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<RecipeFile>>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    Ok(Json(recipes::list(&state, &group.slug).await?))
}
pub async fn read_recipe(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, name)): Path<(String, String)>,
) -> ApiResult<Json<RecipeFile>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    Ok(Json(recipes::read(&state, &group.slug, &name).await?))
}
pub async fn list(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    Query(filter): Query<Filter>,
) -> ApiResult<Json<Vec<Request>>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{ROW}
         JOIN threads t ON t.id = r.thread_id
         WHERE t.group_id = ?
         ORDER BY t.number DESC LIMIT 200"
    )))
    .bind(group.group_id)
    .fetch_all(&state.db)
    .await?;
    let mut out = Vec::new();
    for row in rows {
        let matches = match filter.state.as_deref() {
            None | Some("open") => ["waiting", "running"].contains(&row.state.as_str()),
            Some("scheduled") => !row.recurring && row.next_at.is_some(),
            Some("recurring") => {
                row.recurring && ["waiting", "running"].contains(&row.state.as_str())
            }
            Some("finished") => !["waiting", "running"].contains(&row.state.as_str()),
            Some(_) => return Err(ApiError::invalid("Unknown board filter")),
        };
        if matches {
            out.push(to_api(&state, row).await?);
        }
    }
    Ok(Json(out))
}
pub async fn read(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, number)): Path<(String, u32)>,
) -> ApiResult<Json<Request>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    Ok(Json(
        to_api(&state, find(&state, group.group_id, number).await?).await?,
    ))
}
async fn to_api(state: &AppState, mut row: Row) -> ApiResult<Request> {
    let thread: Thread = forum::find_thread_by_id(state, row.thread_id).await?.into();
    let ask: Ask = decode(&row.ask)?;
    let mut processes = Vec::new();
    #[derive(sqlx::FromRow)]
    struct P {
        id: i64,
        retry_of: Option<i64>,
        slot: i64,
        state: String,
        machine: Option<String>,
        owner_id: Option<i64>,
        started_at: Option<i64>,
        finished_at: Option<i64>,
        branches: String,
        usage: String,
        error: Option<String>,
    }
    let rows: Vec<P> = sqlx::query_as(
        "SELECT p.id, p.retry_of, p.slot, w.state, w.machine, p.owner_id, p.started_at,
         p.finished_at, p.branches, p.usage, w.error
         FROM workshop_processes p
         JOIN work_leases w ON w.id = p.work_id
         WHERE p.thread_id = ?
         ORDER BY p.id DESC",
    )
    .bind(row.thread_id)
    .fetch_all(&state.db)
    .await?;
    for p in rows {
        processes.push(Process {
            number: p.id,
            retry_of: p.retry_of,
            slot: p.slot,
            state: if p.state == "failed"
                && p.error
                    .as_ref()
                    .is_some_and(|e| e.contains("expired while waiting"))
            {
                "expired".into()
            } else if p.state == "passed" {
                "done".into()
            } else if p.state == "queued" {
                "waiting".into()
            } else {
                p.state
            },
            machine: p.machine,
            owner: match p.owner_id {
                Some(id) => Some(user(state, id).await?),
                None => None,
            },
            started_at: p.started_at,
            finished_at: p.finished_at,
            branches: decode(&p.branches)?,
            usage: decode(&p.usage)?,
            error: p.error,
        });
    }
    if row.state == "running"
        && !processes.iter().any(|p| p.state == "running")
        && (row.next_at.is_some() || processes.iter().any(|p| p.state == "waiting"))
    {
        row.state = "waiting".into();
    }
    let pending: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT u.display_name
         FROM workshop_offers o
         JOIN api_tokens t ON t.id = o.machine_id
         JOIN users u ON u.id = t.user_id
         JOIN workshop_processes p ON p.id = o.process_id
         JOIN work_leases w ON w.id = p.work_id
         WHERE p.thread_id = ? AND o.decision = 'pending' AND w.state = 'queued'",
    )
    .bind(row.thread_id)
    .fetch_all(&state.db)
    .await?;
    let waiting = if row.state == "waiting" {
        Some(if row.paused {
            "Paused".into()
        } else if row.next_at.is_some_and(|a| a > db::now()) {
            "Waiting for its time".into()
        } else if !pending.is_empty() {
            format!("Waiting for approval on {}'s machines", pending.join(", "))
        } else {
            match ask.assignment {
                Assignment::Mine => format!(
                    "Waiting for one of {}'s machines",
                    thread.created_by.display_name
                ),
                Assignment::Member { user: id } => format!(
                    "Waiting for one of {}'s machines",
                    user(state, id).await?.display_name
                ),
                Assignment::Anyone => {
                    format!("Waiting for a machine with {}", runner_name(&ask.runner))
                }
            }
        })
    } else {
        None
    };
    Ok(Request {
        requester: thread.created_by.clone(),
        thread,
        recipe: decode(&row.recipe)?,
        ask,
        state: row.state,
        paused: row.paused,
        next_at: row.next_at,
        waiting,
        processes,
    })
}
fn may_ask(viewer: &Viewer) -> ApiResult<()> {
    if viewer.agent().is_some_and(|a| !a.session)
        || matches!(viewer.scope, crate::auth::Scope::Process { .. })
    {
        return Err(ApiError::forbidden(
            "This token only reads workshop requests",
        ));
    }
    Ok(())
}
pub async fn ask(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    Json(mut ask): Json<Ask>,
) -> ApiResult<Json<Request>> {
    may_ask(&viewer)?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    if !group.role.can_write() {
        return Err(ApiError::forbidden("Guests read the board"));
    }
    ask.title = required_text(&ask.title, "The request's title", 200)?;
    ask.schedule.validate().map_err(ApiError::invalid)?;
    if ![1, 24, 168].contains(&ask.wait_hours) {
        return Err(ApiError::invalid("Wait for an hour, a day or a week"));
    }
    if !["claude", "codex", "opencode"].contains(&ask.runner.as_str()) {
        return Err(ApiError::invalid("Unknown runner"));
    }
    if let Some(model) = &ask.model {
        required_text(model, "The model", 200)?;
    }
    if let Assignment::Member { user } = ask.assignment {
        let valid: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1
             FROM memberships
             WHERE group_id = ?
             AND user_id = ?
             AND role IN ('admin', 'member'))",
        )
        .bind(group.group_id)
        .bind(user)
        .fetch_one(&state.db)
        .await?;
        if !valid {
            return Err(ApiError::invalid("Choose a group member"));
        }
    }
    let file = recipes::read(&state, &group.slug, &ask.recipe).await?;
    let recipe = file.recipe.as_ref().ok_or_else(|| {
        ApiError::invalid(
            file.error
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
        )
    })?;
    recipe
        .render(&format!("{}.md", file.name), &ask.inputs)
        .map_err(|e| ApiError::invalid(e.to_string()))?;
    if !recipe.front.runners.is_empty() && !recipe.front.runners.contains(&ask.runner) {
        return Err(ApiError::invalid("The recipe does not accept this runner"));
    }
    for (name, input) in &recipe.front.inputs {
        let Some(value) = ask.inputs.get(name).filter(|v| !v.trim().is_empty()) else {
            continue;
        };
        if value.len() > 100_000 {
            return Err(ApiError::invalid("An input is too long"));
        }
        match input.kind {
            InputKind::Paper => {
                crate::papers::find(&state, group.group_id, value).await?;
            }
            InputKind::Thread => {
                let number = value
                    .trim_start_matches('t')
                    .parse()
                    .map_err(|_| ApiError::invalid("Expected a thread number"))?;
                forum::find_thread(&state, group.group_id, number).await?;
            }
            InputKind::Declaration => {
                if !crate::library::resolve(&state, group.group_id, value)
                    .await?
                    .exists
                {
                    return Err(ApiError::not_found("Declaration"));
                }
            }
            InputKind::Text => {}
        }
    }
    let now = db::now();
    let at = ask.schedule.at.unwrap_or(now);
    if ask.schedule.until.is_some_and(|u| u < at) {
        return Err(ApiError::invalid("The series ends before its first slot"));
    }
    let mut tx = state.db.begin().await?;
    let number: i64 = sqlx::query_scalar(
        "UPDATE groups SET last_thread = last_thread+1 WHERE id = ? RETURNING last_thread",
    )
    .bind(group.group_id)
    .fetch_one(&mut *tx)
    .await?;
    let ids = crate::text::encode(&mut tx, group.group_id, &ask.title).await?;
    let thread = sqlx::query(
        "INSERT INTO threads (group_id, number, title, title_ids, kind, created_by,
         created_at, last_activity_at)
         VALUES (?, ?, ?, ?, 'request', ?, ?, ?)",
    )
    .bind(group.group_id)
    .bind(number)
    .bind(&ask.title)
    .bind(ids)
    .bind(viewer.id)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();
    sqlx::query(
        "INSERT INTO workshop_requests (thread_id, recipe, ask, recurring, next_at,
         created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(thread)
    .bind(encode(&file)?)
    .bind(encode(&ask)?)
    .bind(ask.schedule.every.is_some())
    .bind(at)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let thread_api: Thread = forum::find_thread_by_id(&state, thread).await?.into();
    state.publish(
        group.group_id,
        stemma_core::api::Event::ThreadCreated { thread: thread_api },
    );
    tick(&state).await?;
    Ok(Json(
        to_api(&state, find(&state, group.group_id, number as u32).await?).await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    action: String,
}
pub async fn action(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, number)): Path<(String, u32)>,
    Json(action): Json<Action>,
) -> ApiResult<Json<Request>> {
    may_ask(&viewer)?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    let row = find(&state, group.group_id, number).await?;
    if !group.role.can_write() {
        return Err(ApiError::forbidden("Guests read workshop requests"));
    }
    if !["waiting", "running"].contains(&row.state.as_str()) {
        return Err(ApiError::conflict("This request has already finished"));
    }
    let thread: Thread = forum::find_thread_by_id(&state, row.thread_id)
        .await?
        .into();
    if viewer.id != thread.created_by.id {
        group.require_admin()?;
    }
    match action.action.as_str() {
        "pause" | "resume"
            if row.recurring && ["waiting", "running"].contains(&row.state.as_str()) =>
        {
            sqlx::query("UPDATE workshop_requests SET paused = ? WHERE thread_id = ?")
                .bind(action.action == "pause")
                .bind(row.thread_id)
                .execute(&state.db)
                .await?;
        }
        "cancel" | "stop" => {
            let mut tx = state.db.begin().await?;
            sqlx::query(
                "UPDATE workshop_requests SET state='cancelled', next_at=NULL WHERE thread_id=?",
            )
            .bind(row.thread_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE api_tokens
                 SET revoked_at = ?
                 WHERE id IN (SELECT token_id
                 FROM work_leases
                 WHERE id IN (SELECT work_id
                 FROM workshop_processes
                 WHERE thread_id = ?)
                 AND state IN ('queued', 'running'))",
            )
            .bind(db::now())
            .bind(row.thread_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE work_leases
                 SET state = 'cancelled'
                 WHERE id IN (SELECT work_id FROM workshop_processes WHERE thread_id = ?)
                 AND state IN ('queued', 'running')",
            )
            .bind(row.thread_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            cancel_machines(&state, row.thread_id).await?;
        }
        _ => {
            return Err(ApiError::invalid(
                "Use cancel, pause, resume or stop on an open recurring request",
            ));
        }
    }
    Ok(Json(
        to_api(&state, find(&state, group.group_id, number).await?).await?,
    ))
}
async fn cancel_machines(state: &AppState, thread: i64) -> ApiResult<()> {
    let leases: Vec<(i64, i64, Option<i64>)> = sqlx::query_as(
        "SELECT w.id, w.attempts, w.holder
         FROM work_leases w
         JOIN workshop_processes p ON p.work_id = w.id
         WHERE p.thread_id = ? AND w.state = 'cancelled'",
    )
    .bind(thread)
    .fetch_all(&state.db)
    .await?;
    let offers: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT o.machine_id, o.process_id
         FROM workshop_offers o
         JOIN workshop_processes p ON p.id = o.process_id
         WHERE p.thread_id = ? AND o.decision IN ('offered', 'pending')",
    )
    .bind(thread)
    .fetch_all(&state.db)
    .await?;
    let machines = state.build_machines.lock().await;
    for (machine, process) in offers {
        if let Some(m) = machines.0.get(&machine) {
            let _ = m.sender.send(HubMessage::ProcessTaken { process });
        }
    }
    for (id, attempt, holder) in leases {
        if let Some(m) = holder.and_then(|id| machines.0.get(&id)) {
            let _ = m.sender.send(HubMessage::CancelProcess {
                lease: LeaseRef {
                    id,
                    attempt: attempt as u32,
                },
            });
        }
    }
    Ok(())
}
/// The scheduler coalesces missed slots and atomically advances next_at together
/// with insertion. A second hub task observes the advanced slot and does nothing.
pub async fn tick(state: &AppState) -> ApiResult<()> {
    let now = db::now();
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{ROW} WHERE paused=0 AND next_at<=? AND state IN ('waiting','running')"
    )))
    .bind(now)
    .fetch_all(&state.db)
    .await?;
    for row in rows {
        let ask: Ask = decode(&row.ask)?;
        let slot = row.next_at.unwrap();
        let next = ask.schedule.next_after(slot, now);
        let mut tx = state.db.begin().await?;
        let changed = sqlx::query(
            "UPDATE workshop_requests
             SET next_at = ?
             WHERE thread_id = ?
             AND next_at = ?
             AND paused = 0
             AND state IN ('waiting', 'running')",
        )
        .bind(next)
        .bind(row.thread_id)
        .bind(slot)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if changed == 0 {
            tx.rollback().await?;
            continue;
        }
        let group: i64 = sqlx::query_scalar("SELECT group_id FROM threads WHERE id=?")
            .bind(row.thread_id)
            .fetch_one(&mut *tx)
            .await?;
        let work =
            sqlx::query("INSERT INTO work_leases(group_id,kind,created_at) VALUES (?,'process',?)")
                .bind(group)
                .bind(now)
                .execute(&mut *tx)
                .await?
                .last_insert_rowid();
        // A missed slot fires once now, with a full machine-wait interval.
        sqlx::query(
            "INSERT INTO workshop_processes(thread_id,work_id,slot,expires_at) VALUES (?,?,?,?)",
        )
        .bind(row.thread_id)
        .bind(work)
        .bind(slot)
        .bind(now + i64::from(ask.wait_hours) * 3_600_000)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }
    sqlx::query(
        "UPDATE work_leases
         SET state = 'failed', error = 'The process exceeded its deadline'
         WHERE kind = 'process'
         AND state = 'running'
         AND id IN (SELECT work_id FROM workshop_processes WHERE deadline <= ?)",
    )
    .bind(now)
    .execute(&state.db)
    .await?;
    sqlx::query(
        "UPDATE work_leases
         SET state = 'failed', error = 'The request expired while waiting for a machine'
         WHERE kind = 'process'
         AND state = 'queued'
         AND id IN (SELECT work_id FROM workshop_processes WHERE expires_at <= ?)",
    )
    .bind(now)
    .execute(&state.db)
    .await?;
    if let Err(error) = finish_pending(state).await {
        tracing::error!(error = error.message, "finalizing workshop processes");
    }
    dispatch(state).await
}
async fn offer(state: &AppState, process: i64) -> ApiResult<ProcessOffer> {
    let (thread, expires): (i64, i64) =
        sqlx::query_as("SELECT thread_id,expires_at FROM workshop_processes WHERE id=?")
            .bind(process)
            .fetch_one(&state.db)
            .await?;
    let row: Row = sqlx::query_as(sqlx::AssertSqlSafe(format!("{ROW} WHERE thread_id=?")))
        .bind(thread)
        .fetch_one(&state.db)
        .await?;
    let ask: Ask = decode(&row.ask)?;
    let recipe: RecipeFile = decode(&row.recipe)?;
    let t: Thread = forum::find_thread_by_id(state, thread).await?.into();
    let group_id: i64 = sqlx::query_scalar("SELECT group_id FROM threads WHERE id=?")
        .bind(thread)
        .fetch_one(&state.db)
        .await?;
    let g = Membership::of_group(state, group_id).await?;
    let prompt = recipe
        .recipe
        .as_ref()
        .ok_or_else(|| ApiError::internal("invalid stored recipe"))?
        .render(&format!("{}.md", recipe.name), &ask.inputs)
        .map_err(ApiError::internal)?;
    Ok(ProcessOffer {
        process,
        group: GroupRef {
            id: group_id,
            slug: g.slug,
            name: g.name,
        },
        request: ThreadRef {
            number: t.number,
            title: t.title,
            kind: ThreadKind::Request,
        },
        requester: t.created_by.clone(),
        assigned_by: t.created_by,
        recipe,
        inputs: ask.inputs,
        prompt,
        runner: ask.runner,
        model: ask.model,
        recurring: row.recurring,
        expires_at: expires,
    })
}
fn eligible(machine: &crate::builds::Machine, offer: &ProcessOffer, ask: &Ask) -> bool {
    let scope = machine.scope(&offer.group.slug);
    scope.enabled
        && scope.concurrency > 0
        && scope.runners.contains_key(&offer.runner)
        && offer
            .model
            .as_ref()
            .is_none_or(|model| scope.runners.get(&offer.runner) == Some(model))
        && offer
            .recipe
            .recipe
            .as_ref()
            .is_some_and(|r| r.accepts_version(&machine.version))
        && (machine.owner == offer.requester.id || scope.others == Others::Ask)
        && match ask.assignment {
            Assignment::Mine => machine.owner == offer.requester.id,
            Assignment::Member { user } => machine.owner == user,
            Assignment::Anyone => true,
        }
}
pub async fn dispatch(state: &AppState) -> ApiResult<()> {
    let _turn = state.process_changes.lock().await;
    let waiting: Vec<(i64, String)> = sqlx::query_as(
        "SELECT p.id, r.ask
         FROM workshop_processes p
         JOIN work_leases w ON w.id = p.work_id
         JOIN workshop_requests r ON r.thread_id = p.thread_id
         WHERE w.state = 'queued'
         AND p.expires_at > ?
         AND r.state IN ('waiting', 'running')
         ORDER BY p.id",
    )
    .bind(db::now())
    .fetch_all(&state.db)
    .await?;
    let machines = state.build_machines.lock().await;
    for (id, raw) in waiting {
        let ask: Ask = decode(&raw)?;
        let offer = offer(state, id).await?;
        for (holder, machine) in &machines.0 {
            if !eligible(machine, &offer, &ask) || machine.sender.is_closed() {
                continue;
            }
            let member: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1
                 FROM memberships m
                 JOIN api_tokens t ON t.user_id = m.user_id
                 WHERE m.group_id = ?
                 AND m.user_id = ?
                 AND m.role IN ('admin', 'member')
                 AND t.id = ?
                 AND t.revoked_at IS NULL
                 AND (t.expires_at IS NULL OR t.expires_at > ?))",
            )
            .bind(offer.group.id)
            .bind(machine.owner)
            .bind(holder)
            .bind(db::now())
            .fetch_one(&state.db)
            .await?;
            if !member {
                continue;
            }
            let room: i64 = sqlx::query_scalar(
                "SELECT count(*)
                 FROM work_leases
                 WHERE kind = 'process' AND state = 'running' AND holder = ?",
            )
            .bind(holder)
            .fetch_one(&state.db)
            .await?;
            if room >= machine.scope(&offer.group.slug).concurrency as i64 {
                continue;
            }
            let sent = sqlx::query(
                "INSERT INTO workshop_offers(process_id, machine_id, connection)
                 VALUES (?, ?, ?) ON CONFLICT(process_id, machine_id) DO UPDATE
                 SET connection = excluded.connection, decision = 'offered'
                 WHERE workshop_offers.connection != excluded.connection
                 AND workshop_offers.decision NOT IN ('declined', 'ran')",
            )
            .bind(id)
            .bind(holder)
            .bind(machine.connection as i64)
            .execute(&state.db)
            .await?
            .rows_affected();
            if sent > 0 {
                let _ = machine
                    .sender
                    .send(HubMessage::ProcessOffer(Box::new(offer.clone())));
            }
        }
    }
    Ok(())
}
/// Accept is a machine's assertion that local approval covers this immutable
/// offer. The conditional lease update chooses exactly one winner.
pub async fn decision(
    state: &AppState,
    holder: i64,
    connection: u64,
    process: i64,
    accept: bool,
    pending: bool,
) -> ApiResult<()> {
    let _turn = state.process_changes.lock().await;
    let machines = state.build_machines.lock().await;
    let Some(machine) = machines
        .0
        .get(&holder)
        .filter(|m| m.connection == connection)
        .cloned()
    else {
        return Ok(());
    };
    drop(machines);
    let delivered: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1
         FROM workshop_offers
         WHERE process_id = ?
         AND machine_id = ?
         AND connection = ?
         AND decision IN ('offered', 'pending'))",
    )
    .bind(process)
    .bind(holder)
    .bind(connection as i64)
    .fetch_one(&state.db)
    .await?;
    if !delivered {
        if accept {
            let _ = machine.sender.send(HubMessage::ProcessTaken { process });
        }
        return Ok(());
    }
    let offer = offer(state, process).await?;
    if !accept {
        let changed = sqlx::query(
            "UPDATE workshop_offers
             SET decision = ?
             WHERE process_id = ? AND machine_id = ? AND decision = 'offered'",
        )
        .bind(if pending { "pending" } else { "declined" })
        .bind(process)
        .bind(holder)
        .execute(&state.db)
        .await?
        .rows_affected();
        if pending && changed > 0 {
            notifications::notify(
                state,
                &[machine.owner],
                &Notice::new(NotificationKind::ProcessApproval)
                    .at(offer.group.id, thread_id(state, process).await?, None)
                    .by(offer.requester.id, None)
                    .text(&format!(
                        "{} asks your workshop to {}",
                        offer.requester.display_name,
                        offer
                            .recipe
                            .recipe
                            .as_ref()
                            .unwrap()
                            .front
                            .title
                            .to_lowercase()
                    )),
            )
            .await?;
        }
        return Ok(());
    }
    let (work, attempts, state_name, raw): (i64, i64, String, String) = sqlx::query_as(
        "SELECT w.id, w.attempts, w.state, r.ask
         FROM workshop_processes p
         JOIN work_leases w ON w.id = p.work_id
         JOIN workshop_requests r ON r.thread_id = p.thread_id
         WHERE p.id = ?",
    )
    .bind(process)
    .fetch_one(&state.db)
    .await?;
    let ask: Ask = decode(&raw)?;
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1
         FROM memberships m
         JOIN api_tokens t ON t.user_id = m.user_id
         WHERE m.group_id = ?
         AND m.user_id = ?
         AND m.role IN ('admin', 'member')
         AND t.id = ?
         AND t.revoked_at IS NULL
         AND (t.expires_at IS NULL OR t.expires_at > ?))",
    )
    .bind(offer.group.id)
    .bind(machine.owner)
    .bind(holder)
    .bind(db::now())
    .fetch_one(&state.db)
    .await?;
    let running: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM work_leases WHERE holder=? AND kind='process' AND state='running'",
    )
    .bind(holder)
    .fetch_one(&state.db)
    .await?;
    if state_name != "queued"
        || !valid
        || !eligible(&machine, &offer, &ask)
        || offer.expires_at <= db::now()
    {
        if state_name == "queued" {
            sqlx::query(
                "DELETE
                 FROM workshop_offers
                 WHERE process_id = ? AND machine_id = ? AND decision != 'ran'",
            )
            .bind(process)
            .bind(holder)
            .execute(&state.db)
            .await?;
        }
        let _ = machine.sender.send(HubMessage::ProcessTaken { process });
        return Ok(());
    }
    // Capacity can fill between offering and accepting. Keep this machine's
    // approved offer: it retries acceptance when it has room again.
    if running >= machine.scope(&offer.group.slug).concurrency as i64 {
        return Ok(());
    }
    let recipe = offer.recipe.recipe.as_ref().unwrap();
    let mut repositories = Vec::new();
    let forge = state
        .forge
        .as_ref()
        .ok_or_else(|| ApiError::not_found("Forge"))?;
    for writes in &recipe.front.writes {
        let name = match writes {
            Repository::Library => "library".to_owned(),
            Repository::Paper => {
                let input = recipe
                    .front
                    .inputs
                    .iter()
                    .find(|(_, i)| i.kind == InputKind::Paper)
                    .unwrap()
                    .0;
                let slug = offer
                    .inputs
                    .get(input)
                    .ok_or_else(|| ApiError::invalid("Missing paper"))?;
                format!("paper-{slug}")
            }
        };
        let base = forge
            .branch_commit(&offer.group.slug, &name, "main")
            .await
            .map_err(forge::api_error)?
            .ok_or_else(|| ApiError::not_found("Repository main"))?;
        repositories.push(ProcessRepository {
            path: format!("/git/{}/{}.git", offer.group.slug, name),
            base,
            branch: format!("workshop/{}-{process}", offer.request.number),
        });
    }
    let deadline = db::now() + i64::from(recipe.front.timeout_minutes) * 60_000;
    let token = crate::secret::token("stp_");
    let lease = LeaseRef {
        id: work,
        attempt: attempts as u32 + 1,
    };
    let thread = thread_id(state, process).await?;
    let runner_name = runner_name(&offer.runner);
    let mut tx = state.db.begin().await?;
    let token_id = sqlx::query(
        "INSERT INTO api_tokens(user_id, name, token_hash, prefix, created_at, kind,
         group_id, build_work, lease_attempt, thread_id, agent, can_post, attachments,
         expires_at)
         VALUES (?, 'Process', ?, ?, ?, 'process', ?, ?, ?, ?, ?, 1, 'group', ?)",
    )
    .bind(machine.owner)
    .bind(crate::secret::token_hash(&token))
    .bind(&token[..10])
    .bind(db::now())
    .bind(offer.group.id)
    .bind(work)
    .bind(lease.attempt)
    .bind(thread)
    .bind(runner_name)
    .bind(db::now() + lease::EXPIRY_MILLIS)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();
    if !lease::acquire(&mut tx, lease, holder, connection, token_id, &machine.name).await? {
        tx.rollback().await?;
        let _ = machine.sender.send(HubMessage::ProcessTaken { process });
        return Ok(());
    }
    let branches: Vec<ProcessBranch> = repositories
        .iter()
        .map(|r| ProcessBranch {
            repository: r
                .path
                .rsplit('/')
                .next()
                .unwrap()
                .trim_end_matches(".git")
                .into(),
            branch: r.branch.clone(),
            commit: None,
            pull_request: None,
        })
        .collect();
    sqlx::query(
        "UPDATE workshop_processes SET owner_id=?,started_at=?,deadline=?,branches=? WHERE id=?",
    )
    .bind(machine.owner)
    .bind(db::now())
    .bind(deadline)
    .bind(encode(&branches)?)
    .bind(process)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE workshop_requests SET state='running' WHERE thread_id=?")
        .bind(thread)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE workshop_offers SET decision='ran' WHERE process_id=? AND machine_id=?")
        .bind(process)
        .bind(holder)
        .execute(&mut *tx)
        .await?;
    let others: Vec<i64> = sqlx::query_scalar(
        "UPDATE workshop_offers
         SET decision = 'taken'
         WHERE process_id = ? AND machine_id != ? AND decision IN ('offered', 'pending')
         RETURNING machine_id",
    )
    .bind(process)
    .bind(holder)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let machines = state.build_machines.lock().await;
    for id in others {
        if let Some(m) = machines.0.get(&id) {
            let _ = m.sender.send(HubMessage::ProcessTaken { process });
        }
    }
    drop(machines);
    let _ = machine
        .sender
        .send(HubMessage::Process(Box::new(ProcessSpec {
            lease,
            offer,
            owner: user(state, machine.owner).await?,
            token,
            repositories,
            deadline,
        })));
    Ok(())
}
pub fn runner_name(handle: &str) -> &str {
    match handle {
        "claude" => "Claude",
        "codex" => "Codex",
        "opencode" => "OpenCode",
        _ => handle,
    }
}
async fn thread_id(state: &AppState, process: i64) -> ApiResult<i64> {
    Ok(
        sqlx::query_scalar("SELECT thread_id FROM workshop_processes WHERE id=?")
            .bind(process)
            .fetch_one(&state.db)
            .await?,
    )
}
pub async fn finished(
    state: &AppState,
    holder: i64,
    connection: u64,
    lease: LeaseRef,
    result: ProcessResult,
) -> ApiResult<()> {
    let _turn = state.process_changes.lock().await;
    let id: Option<i64> = sqlx::query_scalar(
        "SELECT p.id
         FROM workshop_processes p
         JOIN work_leases w ON w.id = p.work_id
         WHERE w.id = ?
         AND w.holder = ?
         AND w.connection = ?
         AND w.attempts = ?
         AND w.state = 'running'
         AND w.heartbeat > ?
         AND p.deadline > ?",
    )
    .bind(lease.id)
    .bind(holder)
    .bind(connection as i64)
    .bind(lease.attempt)
    .bind(db::now() - lease::EXPIRY_MILLIS)
    .bind(db::now())
    .fetch_optional(&state.db)
    .await?;
    let Some(id) = id else {
        return Ok(());
    };
    let mut tx = state.db.begin().await?;
    let changed = sqlx::query(
        "UPDATE work_leases SET state=?,error=? WHERE id=? AND state='running' AND attempts=?",
    )
    .bind(if result.error.is_none() {
        "passed"
    } else {
        "failed"
    })
    .bind(result.error.as_deref().map(stemma_core::secrets::hide))
    .bind(lease.id)
    .bind(lease.attempt)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed == 0 {
        tx.rollback().await?;
        return Ok(());
    }
    sqlx::query("UPDATE workshop_processes SET summary=?,usage=? WHERE id=?")
        .bind(stemma_core::secrets::hide(&result.summary))
        .bind(encode(&result.usage)?)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if result.error.is_some() {
        retry(&mut tx, lease.id).await?;
    }
    sqlx::query(
        "UPDATE api_tokens SET revoked_at=? WHERE id=(SELECT token_id FROM work_leases WHERE id=?)",
    )
    .bind(db::now())
    .bind(lease.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    drop(_turn);
    finish_pending(state).await?;
    dispatch(state).await
}
/// Durable finalization also recovers after a machine or hub disappears. Pushes
/// are discovered from the forge, never trusted from an agent's result.
pub async fn finish_pending(state: &AppState) -> ApiResult<()> {
    let _turn = state.process_changes.lock().await;
    #[derive(sqlx::FromRow)]
    struct Finished {
        id: i64,
        work_id: i64,
        state: String,
        error: Option<String>,
        summary: Option<String>,
        branches: String,
    }
    let rows: Vec<Finished> = sqlx::query_as(
        "SELECT p.id, p.work_id, w.state, w.error, p.summary, p.branches
         FROM workshop_processes p
         JOIN work_leases w ON w.id = p.work_id
         WHERE w.state IN ('passed', 'failed', 'cancelled') AND p.finished_at IS NULL
         ORDER BY p.id",
    )
    .fetch_all(&state.db)
    .await?;
    for Finished {
        id,
        work_id: work,
        state: status,
        error,
        summary,
        branches: raw,
    } in rows
    {
        let offer = offer(state, id).await?;
        let thread = thread_id(state, id).await?;
        let mut branches: Vec<ProcessBranch> = decode(&raw)?;
        if let Some(forge) = &state.forge {
            for branch in &mut branches {
                if let Some(head) = forge
                    .branch_commit(&offer.group.slug, &branch.repository, &branch.branch)
                    .await
                    .map_err(forge::api_error)?
                {
                    branch.commit = Some(head);
                    if branch.pull_request.is_none() {
                        branch.pull_request = Some(
                            forge
                                .open_process_pull_request(
                                    &offer.group.slug,
                                    &branch.repository,
                                    &branch.branch,
                                    &offer.request.title,
                                    &format!(
                                        "For [{}]({}/g/{}/t/{}).",
                                        offer.request.title,
                                        state.config.base_url(),
                                        offer.group.slug,
                                        offer.request.number
                                    ),
                                )
                                .await
                                .map_err(forge::api_error)?,
                        );
                    }
                }
            }
        }
        let pushed = branches.iter().any(|b| b.commit.is_some());
        let expired = error
            .as_ref()
            .is_some_and(|e| e.contains("expired while waiting"));
        let mut text = summary.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| {
            format!(
                "Process {id}: {}.",
                if expired { "expired" } else { &status }
            )
        });
        if let Some(error) = &error {
            text.push_str(&format!("\n\n{error}"));
        }
        for b in branches.iter().filter(|b| b.pull_request.is_some()) {
            text.push_str(&format!(
                "\n\n[Pull request for {}]({})",
                b.repository,
                b.pull_request.as_ref().unwrap()
            ));
        }
        let successor: Option<i64> =
            sqlx::query_scalar("SELECT id FROM workshop_processes WHERE retry_of=?")
                .bind(id)
                .fetch_optional(&state.db)
                .await?;
        if let Some(next) = successor {
            text.push_str(&format!(
                "\n\nRetrying as process {next}, waiting for another eligible machine."
            ));
        }
        let group = Membership::of_group(state, offer.group.id).await?;
        let owner: Option<i64> =
            sqlx::query_scalar("SELECT owner_id FROM workshop_processes WHERE id=?")
                .bind(id)
                .fetch_one(&state.db)
                .await?;
        let mut tx = state.db.begin().await?;
        sqlx::query("UPDATE workshop_processes SET finished_at=?,branches=?,pushed=? WHERE id=?")
            .bind(db::now())
            .bind(encode(&branches)?)
            .bind(pushed)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE api_tokens
             SET revoked_at = ?
             WHERE id = (SELECT token_id FROM work_leases WHERE id = ?)",
        )
        .bind(db::now())
        .bind(work)
        .execute(&mut *tx)
        .await?;
        let remaining: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM workshop_processes p
             JOIN work_leases w ON w.id = p.work_id
             WHERE p.thread_id = ? AND w.state IN ('queued', 'running')",
        )
        .bind(thread)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE workshop_requests
             SET state = CASE WHEN ? > 0 THEN 'running' WHEN recurring = 1
             AND next_at IS NOT NULL THEN 'waiting' ELSE ? END
             WHERE thread_id = ? AND state IN ('waiting', 'running')",
        )
        .bind(remaining)
        .bind(if status == "passed" {
            "done"
        } else if expired {
            "expired"
        } else if status == "cancelled" {
            "cancelled"
        } else {
            "failed"
        })
        .bind(thread)
        .execute(&mut *tx)
        .await?;
        let (number, stored) = forum::insert_message(
            &mut tx,
            &group,
            thread,
            owner.unwrap_or(offer.requester.id),
            Some(runner_name(&offer.runner)),
            Some(if offer.runner == "claude" {
                "claude-code"
            } else {
                &offer.runner
            }),
            &stemma_core::secrets::hide(&text),
            None,
            &[],
            true,
        )
        .await?;
        tx.commit().await?;
        let m = forum::publish_post(state, &group, thread, number, &stored).await?;
        let pending: Vec<i64> = sqlx::query_scalar(
            "UPDATE workshop_offers
             SET decision = 'taken'
             WHERE process_id = ? AND decision IN ('offered', 'pending')
             RETURNING machine_id",
        )
        .bind(id)
        .fetch_all(&state.db)
        .await?;
        let machines = state.build_machines.lock().await;
        for id_machine in pending {
            if let Some(machine) = machines.0.get(&id_machine) {
                let _ = machine
                    .sender
                    .send(HubMessage::ProcessTaken { process: id });
            }
        }
        drop(machines);
        let kind = if expired {
            NotificationKind::RequestExpired
        } else if status == "passed" {
            NotificationKind::RequestDone
        } else {
            NotificationKind::RequestFailed
        };
        if successor.is_none() && status != "cancelled" {
            notifications::notify(
                state,
                &[offer.requester.id],
                &Notice::new(kind)
                    .at(offer.group.id, thread, Some(m.number))
                    .text(&text),
            )
            .await?;
        }
        if pushed {
            notifications::notify(
                state,
                &[offer.requester.id],
                &Notice::new(NotificationKind::ProcessPullRequest)
                    .at(offer.group.id, thread, Some(m.number))
                    .text("A pull request was opened for your request"),
            )
            .await?;
        }
    }
    Ok(())
}

/// Stop model work before a request or group is removed.
pub async fn forget_thread(state: &AppState, thread: i64) -> ApiResult<()> {
    let _turn = state.process_changes.lock().await;
    sqlx::query(
        "UPDATE api_tokens
         SET revoked_at = ?
         WHERE id IN (SELECT token_id
         FROM work_leases
         WHERE id IN (SELECT work_id
         FROM workshop_processes
         WHERE thread_id = ?))",
    )
    .bind(db::now())
    .bind(thread)
    .execute(&state.db)
    .await?;
    sqlx::query(
        "UPDATE work_leases
         SET state = 'cancelled'
         WHERE kind = 'process'
         AND id IN (SELECT work_id FROM workshop_processes WHERE thread_id = ?)
         AND state IN ('queued', 'running')",
    )
    .bind(thread)
    .execute(&state.db)
    .await?;
    cancel_machines(state, thread).await
}

/// A retry is another process, rather than another machine overwriting a run's
/// history. The unique parent allows one successor, in this same transaction.
pub(crate) async fn retry(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    work: i64,
) -> ApiResult<Option<i64>> {
    #[derive(sqlx::FromRow)]
    struct Previous {
        id: i64,
        thread_id: i64,
        group_id: i64,
        slot: i64,
        expires_at: i64,
        retry_of: Option<i64>,
        pushed: bool,
        state: String,
    }
    let p: Previous = sqlx::query_as(
        "SELECT p.id, p.thread_id, w.group_id, p.slot, p.expires_at, p.retry_of, p.pushed,
         r.state
         FROM workshop_processes p
         JOIN work_leases w ON w.id = p.work_id
         JOIN workshop_requests r ON r.thread_id = p.thread_id
         WHERE w.id = ?",
    )
    .bind(work)
    .fetch_one(&mut **tx)
    .await?;
    if p.retry_of.is_some()
        || p.pushed
        || !["waiting", "running"].contains(&p.state.as_str())
        || p.expires_at <= db::now()
    {
        return Ok(None);
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM workshop_processes WHERE retry_of=?)")
            .bind(p.id)
            .fetch_one(&mut **tx)
            .await?;
    if exists {
        return Ok(None);
    }
    let new_work =
        sqlx::query("INSERT INTO work_leases(group_id,kind,created_at) VALUES (?,'process',?)")
            .bind(p.group_id)
            .bind(db::now())
            .execute(&mut **tx)
            .await?
            .last_insert_rowid();
    let next = sqlx::query(
        "INSERT INTO workshop_processes(thread_id, work_id, slot, expires_at, retry_of)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(p.thread_id)
    .bind(new_work)
    .bind(p.slot)
    .bind(p.expires_at)
    .bind(p.id)
    .execute(&mut **tx)
    .await?
    .last_insert_rowid();
    // A different connection on the same personal token is still the same machine.
    sqlx::query(
        "INSERT INTO workshop_offers(process_id, machine_id, connection, decision)
         SELECT ?, machine_id, connection, 'ran'
         FROM workshop_offers
         WHERE process_id = ? AND decision = 'ran'",
    )
    .bind(next)
    .bind(p.id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE workshop_requests
         SET state = CASE WHEN EXISTS(SELECT 1
         FROM workshop_processes p
         JOIN work_leases w ON w.id = p.work_id
         WHERE p.thread_id = ?1
         AND w.state = 'running') THEN 'running' ELSE 'waiting' END
         WHERE thread_id = ?1",
    )
    .bind(p.thread_id)
    .execute(&mut **tx)
    .await?;
    Ok(Some(next))
}
