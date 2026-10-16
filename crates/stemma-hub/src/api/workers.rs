//! Workers: registering them, and the WebSocket they connect to.
//!
//! A group's admins register the group's workers; anyone registers their
//! own. Registering gives a token, shown once, that the worker connects
//! with. See [`stemma_core::protocol`] for what is said on the connection.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use stemma_core::api::{CreateWorker, CreatedWorker, User, Worker, WorkerScope};
use stemma_core::protocol::{HubMessage, PROTOCOL_VERSION, Welcome, WorkerMessage, WorkerOwner};
use tokio::sync::mpsc;

use super::{Json, Membership, required_text};
use crate::auth::{self, Viewer};
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::jobs::{self, Connected};
use crate::secret;
use crate::state::AppState;

pub const WORKER_TOKEN_PREFIX: &str = "stw_";

/// How often the hub pings a worker, and how long it waits to hear from it.
const PING_EVERY: Duration = Duration::from_secs(20);
const SILENCE_LIMIT: Duration = Duration::from_secs(70);

#[derive(sqlx::FromRow)]
struct WorkerRow {
    id: i64,
    name: String,
    prefix: String,
    group_slug: Option<String>,
    group_name: Option<String>,
    owner_id: Option<i64>,
    owner_username: Option<String>,
    owner_display_name: Option<String>,
    runners: String,
    lean: bool,
    workshop: bool,
    version: Option<String>,
    created_at: i64,
    last_seen_at: Option<i64>,
}

macro_rules! worker_query {
    () => {
        "SELECT w.id, w.name, w.prefix, g.slug AS group_slug,
        g.name AS group_name, w.owner_id, u.username AS owner_username,
        u.display_name AS owner_display_name, w.runners, w.lean, w.workshop, w.version, w.created_at,
        w.last_seen_at
         FROM workers w LEFT JOIN groups g ON g.id = w.group_id
              LEFT JOIN users u ON u.id = w.owner_id"
    };
}

impl WorkerRow {
    fn owner(&self) -> Option<User> {
        Some(User {
            id: self.owner_id?,
            username: self.owner_username.clone()?,
            display_name: self.owner_display_name.clone()?,
        })
    }

    fn to_api(&self, online: bool) -> Worker {
        Worker {
            id: self.id,
            name: self.name.clone(),
            scope: if self.owner_id.is_some() {
                WorkerScope::Member
            } else {
                WorkerScope::Group
            },
            group: self.group_slug.clone(),
            owner: self.owner(),
            prefix: self.prefix.clone(),
            online,
            runners: serde_json::from_str(&self.runners).unwrap_or_default(),
            lean: self.lean,
            workshop: self.workshop,
            version: self.version.clone(),
            created_at: db::from_millis(self.created_at),
            last_seen_at: self.last_seen_at.map(db::from_millis),
        }
    }
}

/// Whose workers to list or revoke.
#[derive(Clone, Copy)]
enum Whose {
    Group(i64),
    Member(i64),
}

async fn listing(state: &AppState, whose: Whose) -> ApiResult<Vec<Worker>> {
    let (query, id) = match whose {
        Whose::Group(id) => (
            concat!(
                worker_query!(),
                " WHERE w.group_id = ? AND w.revoked_at IS NULL ORDER BY w.created_at"
            ),
            id,
        ),
        Whose::Member(id) => (
            concat!(
                worker_query!(),
                " WHERE w.owner_id = ? AND w.revoked_at IS NULL ORDER BY w.created_at"
            ),
            id,
        ),
    };
    let rows: Vec<WorkerRow> = sqlx::query_as(query).bind(id).fetch_all(&state.db).await?;
    let workers = state.workers.lock().await;
    Ok(rows
        .iter()
        .map(|row| row.to_api(workers.is_online(row.id)))
        .collect())
}

/// Registers a worker: a group's, or a member's.
async fn register(
    state: &AppState,
    viewer: &Viewer,
    group_id: Option<i64>,
    body: CreateWorker,
) -> ApiResult<CreatedWorker> {
    let name = required_text(&body.name, "The name", 80)?;
    let secret = secret::token(WORKER_TOKEN_PREFIX);
    let prefix: String = secret.chars().take(WORKER_TOKEN_PREFIX.len() + 6).collect();
    let id = sqlx::query(
        "INSERT INTO workers (name, group_id, owner_id, token_hash, prefix, created_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&name)
    .bind(group_id)
    .bind(if group_id.is_none() {
        Some(viewer.id)
    } else {
        None
    })
    .bind(secret::token_hash(&secret))
    .bind(&prefix)
    .bind(viewer.id)
    .bind(db::now())
    .execute(&state.db)
    .await?
    .last_insert_rowid();
    let row: WorkerRow = sqlx::query_as(concat!(worker_query!(), " WHERE w.id = ?"))
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    Ok(CreatedWorker {
        worker: row.to_api(false),
        secret,
    })
}

async fn revoke(state: &AppState, whose: Whose, id: i64) -> ApiResult<()> {
    let (query, owner) = match whose {
        Whose::Group(owner) => (
            "UPDATE workers SET revoked_at = ?
             WHERE id = ? AND group_id = ? AND revoked_at IS NULL",
            owner,
        ),
        Whose::Member(owner) => (
            "UPDATE workers SET revoked_at = ?
             WHERE id = ? AND owner_id = ? AND revoked_at IS NULL",
            owner,
        ),
    };
    let done = sqlx::query(query)
        .bind(db::now())
        .bind(id)
        .bind(owner)
        .execute(&state.db)
        .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found("Worker"));
    }
    jobs::disconnect(state, id).await
}

/// The group's workers, for any member to see.
pub async fn list_group(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<Worker>>> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    Ok(Json(listing(&state, Whose::Group(group.group_id)).await?))
}

pub async fn create_group(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    Json(body): Json<CreateWorker>,
) -> ApiResult<(StatusCode, Json<CreatedWorker>)> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    group.require_admin()?;
    let created = register(&state, &viewer, Some(group.group_id), body).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub async fn revoke_group(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, id)): Path<(String, i64)>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    group.require_admin()?;
    revoke(&state, Whose::Group(group.group_id), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The viewer's own workers.
pub async fn list_mine(
    State(state): State<AppState>,
    viewer: Viewer,
) -> ApiResult<Json<Vec<Worker>>> {
    viewer.require_full()?;
    Ok(Json(listing(&state, Whose::Member(viewer.id)).await?))
}

pub async fn create_mine(
    State(state): State<AppState>,
    viewer: Viewer,
    Json(body): Json<CreateWorker>,
) -> ApiResult<(StatusCode, Json<CreatedWorker>)> {
    viewer.require_full()?;
    let created = register(&state, &viewer, None, body).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub async fn revoke_mine(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    revoke(&state, Whose::Member(viewer.id), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── The connection ────────────────────────────────────────────────────

/// A worker, as its token says.
struct Authenticated {
    id: i64,
    name: String,
    group_id: Option<i64>,
    owner: WorkerOwner,
    owner_id: Option<i64>,
}

async fn authenticate(state: &AppState, headers: &HeaderMap) -> ApiResult<Authenticated> {
    let invalid = || {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            "This worker token is not valid, or was revoked",
        )
    };
    let token = auth::bearer_token(headers).ok_or_else(invalid)?;
    let row: Option<(i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, group_id FROM workers WHERE token_hash = ? AND revoked_at IS NULL",
    )
    .bind(secret::token_hash(token))
    .fetch_optional(&state.db)
    .await?;
    let (id, group_id) = row.ok_or_else(invalid)?;
    let row: WorkerRow = sqlx::query_as(concat!(worker_query!(), " WHERE w.id = ?"))
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    let owner = match (&row.group_slug, &row.group_name, row.owner()) {
        (Some(slug), Some(name), _) => WorkerOwner::Group {
            slug: slug.clone(),
            name: name.clone(),
        },
        (_, _, Some(user)) => WorkerOwner::Member { user },
        _ => return Err(ApiError::internal("a worker with no owner")),
    };
    Ok(Authenticated {
        id,
        name: row.name,
        group_id,
        owner,
        owner_id: row.owner_id,
    })
}

/// The workers' WebSocket.
pub async fn connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> ApiResult<Response> {
    let worker = authenticate(&state, &headers).await?;
    Ok(upgrade.on_upgrade(move |socket| async move {
        let id = worker.id;
        if let Err(error) = serve(state, worker, socket).await {
            tracing::warn!(worker = id, %error, "worker connection ended");
        }
    }))
}

async fn send(socket: &mut WebSocket, message: &HubMessage) -> anyhow::Result<()> {
    let text = serde_json::to_string(message)?;
    socket.send(WsMessage::Text(text.into())).await?;
    Ok(())
}

async fn serve(
    state: AppState,
    worker: Authenticated,
    mut socket: WebSocket,
) -> anyhow::Result<()> {
    // The first message says what the worker can do.
    let hello = match tokio::time::timeout(Duration::from_secs(15), socket.next()).await {
        Ok(Some(Ok(WsMessage::Text(text)))) => match serde_json::from_str(&text) {
            Ok(WorkerMessage::Hello(hello)) => hello,
            _ => anyhow::bail!("the worker did not start with a hello"),
        },
        _ => anyhow::bail!("the worker said nothing"),
    };
    if hello.protocol != PROTOCOL_VERSION {
        let reason = format!(
            "This hub speaks version {PROTOCOL_VERSION} of the worker protocol, and the worker \
             version {}: update the one that is behind",
            hello.protocol
        );
        send(&mut socket, &HubMessage::Refused { reason }).await?;
        return Ok(());
    }
    sqlx::query(
        "UPDATE workers SET last_seen_at = ?, runners = ?, lean = ?, workshop = ?, version = ?
         WHERE id = ?",
    )
    .bind(db::now())
    .bind(serde_json::to_string(&hello.runners)?)
    .bind(hello.lean)
    .bind(hello.workshop)
    .bind(&hello.version)
    .bind(worker.id)
    .execute(&state.db)
    .await?;
    send(
        &mut socket,
        &HubMessage::Welcome(Welcome {
            protocol: PROTOCOL_VERSION,
            name: worker.name.clone(),
            owner: worker.owner.clone(),
        }),
    )
    .await?;
    tracing::info!(worker = worker.id, name = worker.name, runners = ?hello.runners, "worker connected");

    let connection = state.connections.fetch_add(1, Ordering::Relaxed) + 1;
    let (sender, mut outbox) = mpsc::unbounded_channel();
    jobs::connected(
        &state,
        worker.id,
        Connected {
            connection,
            group_id: worker.group_id,
            owner_id: worker.owner_id,
            runners: hello.runners,
            lean: hello.lean,
            workshop: hello.workshop,
            concurrency: hello.concurrency.clamp(1, 64),
            running: HashSet::new(),
            sender,
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!(e.message))?;

    let ended = talk(&state, worker.id, &mut socket, &mut outbox).await;
    let _ = socket.close().await;
    sqlx::query("UPDATE workers SET last_seen_at = ? WHERE id = ?")
        .bind(db::now())
        .bind(worker.id)
        .execute(&state.db)
        .await?;
    jobs::disconnected(&state, worker.id, connection)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    tracing::info!(worker = worker.id, "worker disconnected");
    ended
}

/// Passes messages both ways until the worker goes, or is replaced.
async fn talk(
    state: &AppState,
    worker_id: i64,
    socket: &mut WebSocket,
    outbox: &mut mpsc::UnboundedReceiver<HubMessage>,
) -> anyhow::Result<()> {
    let mut ping = tokio::time::interval(PING_EVERY);
    let mut heard = Instant::now();
    loop {
        tokio::select! {
            out = outbox.recv() => match out {
                Some(message) => send(socket, &message).await?,
                // Replaced by a newer connection, or revoked.
                None => return Ok(()),
            },
            incoming = socket.next() => {
                heard = Instant::now();
                let text = match incoming {
                    Some(Ok(WsMessage::Text(text))) => text,
                    Some(Ok(WsMessage::Close(_))) | None => return Ok(()),
                    Some(Ok(_)) => continue,
                    Some(Err(error)) => return Err(error.into()),
                };
                let message: WorkerMessage = match serde_json::from_str(&text) {
                    Ok(message) => message,
                    Err(error) => {
                        tracing::warn!(worker = worker_id, %error, "unreadable message from a worker");
                        continue;
                    }
                };
                let handled = match message {
                    WorkerMessage::Hello(_) => Ok(()),
                    WorkerMessage::Progress { job, text } => {
                        jobs::progress(state, worker_id, job, &text).await
                    }
                    WorkerMessage::Finished { job, outcome } => {
                        jobs::finished(state, worker_id, job, outcome).await
                    }
                };
                if let Err(error) = handled {
                    tracing::error!(worker = worker_id, error = error.message, "handling a worker's message");
                }
            }
            _ = ping.tick() => {
                if heard.elapsed() > SILENCE_LIMIT {
                    anyhow::bail!("the worker went silent");
                }
                socket.send(WsMessage::Ping(Default::default())).await?;
            }
        }
    }
}
