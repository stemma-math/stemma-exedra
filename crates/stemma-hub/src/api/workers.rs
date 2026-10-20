//! Machines: registering them, their agents for the workshop, and the
//! WebSocket they connect to.
//!
//! Anyone registers their machines, which run their own agents, for them
//! only; an admin may also make one of theirs work for a group's workshop
//! (see [`crate::workshop::machines`]). Registering gives a token, shown
//! once, that the machine connects with. See [`stemma_core::protocol`]
//! for what is said on the connection.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use futures_util::StreamExt;
use stemma_core::api::{
    CreateMachine, CreatedMachine, Isolation, Machine, UpdateMachine, User, WorkshopAgent,
};
use stemma_core::protocol::{HubMessage, PROTOCOL_VERSION, Welcome, WorkerMessage};
use tokio::sync::mpsc;

use super::{Json, required_text};
use crate::auth::{self, Viewer};
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::jobs::{self, Connected};
use crate::secret;
use crate::state::AppState;
use crate::workshop::machines;

pub const WORKER_TOKEN_PREFIX: &str = "stw_";

/// How often the hub pings a machine, and how long it waits to hear from
/// it.
const PING_EVERY: Duration = Duration::from_secs(20);
const SILENCE_LIMIT: Duration = Duration::from_secs(70);

#[derive(sqlx::FromRow)]
struct MachineRow {
    id: i64,
    name: String,
    prefix: String,
    owner_id: i64,
    username: String,
    display_name: String,
    runners: String,
    lean: bool,
    latex: bool,
    isolation: Option<String>,
    preferred: bool,
    version: Option<String>,
    created_at: i64,
    last_seen_at: Option<i64>,
}

macro_rules! machine_query {
    () => {
        "SELECT w.id, w.name, w.prefix, w.owner_id, u.username, u.display_name, w.runners,
                w.lean, w.latex, w.isolation, w.preferred, w.version, w.created_at,
                w.last_seen_at
         FROM workers w JOIN users u ON u.id = w.owner_id"
    };
}

impl MachineRow {
    fn owner(&self) -> User {
        User {
            id: self.owner_id,
            username: self.username.clone(),
            display_name: self.display_name.clone(),
        }
    }

    async fn machine(&self, state: &AppState) -> ApiResult<Machine> {
        let online = state.workers.lock().await.is_online(self.id);
        Ok(Machine {
            id: self.id,
            name: self.name.clone(),
            prefix: self.prefix.clone(),
            online,
            runners: serde_json::from_str(&self.runners).unwrap_or_default(),
            lean: self.lean,
            latex: self.latex,
            isolation: self.isolation.as_deref().and_then(Isolation::parse),
            preferred: self.preferred,
            workshop: machines::group_slugs_of(state, self.id).await?,
            workshop_agents: machines::agents(state, self.id).await?,
            version: self.version.clone(),
            created_at: db::from_millis(self.created_at),
            last_seen_at: self.last_seen_at.map(db::from_millis),
        })
    }
}

async fn row(state: &AppState, id: i64) -> ApiResult<MachineRow> {
    Ok(sqlx::query_as(concat!(machine_query!(), " WHERE w.id = ?"))
        .bind(id)
        .fetch_one(&state.db)
        .await?)
}

/// The viewer's machines.
pub async fn list_machines(
    State(state): State<AppState>,
    viewer: Viewer,
) -> ApiResult<Json<Vec<Machine>>> {
    viewer.require_full()?;
    let rows: Vec<MachineRow> = sqlx::query_as(concat!(
        machine_query!(),
        " WHERE w.owner_id = ? AND w.revoked_at IS NULL ORDER BY w.created_at"
    ))
    .bind(viewer.id)
    .fetch_all(&state.db)
    .await?;
    let mut machines = Vec::with_capacity(rows.len());
    for row in rows {
        machines.push(row.machine(&state).await?);
    }
    Ok(Json(machines))
}

pub async fn create_machine(
    State(state): State<AppState>,
    viewer: Viewer,
    Json(body): Json<CreateMachine>,
) -> ApiResult<(StatusCode, Json<CreatedMachine>)> {
    viewer.require_full()?;
    let name = required_text(&body.name, "The name", 80)?;
    let taken: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM workers WHERE owner_id = ? AND name = ? AND revoked_at IS NULL",
    )
    .bind(viewer.id)
    .bind(&name)
    .fetch_optional(&state.db)
    .await?;
    if taken.is_some() {
        return Err(ApiError::conflict(format!(
            "You already have a machine “{name}”"
        )));
    }
    let secret = secret::token(WORKER_TOKEN_PREFIX);
    let prefix: String = secret.chars().take(WORKER_TOKEN_PREFIX.len() + 6).collect();
    let id = sqlx::query(
        "INSERT INTO workers (name, owner_id, token_hash, prefix, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&name)
    .bind(viewer.id)
    .bind(secret::token_hash(&secret))
    .bind(&prefix)
    .bind(db::now())
    .execute(&state.db)
    .await?
    .last_insert_rowid();
    if body.preferred {
        prefer(&state, viewer.id, id, true).await?;
    }
    let machine = row(&state, id).await?.machine(&state).await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedMachine { machine, secret }),
    ))
}

/// Makes a machine the preferred one, or not: there is one at most.
async fn prefer(state: &AppState, owner: i64, id: i64, preferred: bool) -> ApiResult<()> {
    let mut tx = state.db.begin().await?;
    if preferred {
        sqlx::query("UPDATE workers SET preferred = 0 WHERE owner_id = ?")
            .bind(owner)
            .execute(&mut *tx)
            .await?;
    }
    let done = sqlx::query(
        "UPDATE workers SET preferred = ? WHERE id = ? AND owner_id = ? AND revoked_at IS NULL",
    )
    .bind(preferred)
    .bind(id)
    .bind(owner)
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found("Machine"));
    }
    tx.commit().await?;
    let now: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM workers WHERE owner_id = ? AND preferred AND revoked_at IS NULL",
    )
    .bind(owner)
    .fetch_optional(&state.db)
    .await?;
    state.workers.lock().await.set_preferred(owner, now);
    jobs::dispatch(state).await
}

pub async fn update_machine(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(id): Path<i64>,
    Json(body): Json<UpdateMachine>,
) -> ApiResult<Json<Machine>> {
    viewer.require_full()?;
    prefer(&state, viewer.id, id, body.preferred).await?;
    Ok(Json(row(&state, id).await?.machine(&state).await?))
}

/// Sets the agents one of the viewer's machines offers the workshop, in
/// order of preference.
pub async fn set_workshop_agents(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(id): Path<i64>,
    Json(body): Json<Vec<WorkshopAgent>>,
) -> ApiResult<Json<Vec<WorkshopAgent>>> {
    viewer.require_full()?;
    Ok(Json(
        machines::set_agents(&state, viewer.id, id, body).await?,
    ))
}

pub async fn revoke_machine(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    let done = sqlx::query(
        "UPDATE workers SET revoked_at = ?, preferred = 0
         WHERE id = ? AND owner_id = ? AND revoked_at IS NULL",
    )
    .bind(db::now())
    .bind(id)
    .bind(viewer.id)
    .execute(&state.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found("Machine"));
    }
    sqlx::query("DELETE FROM workshop_machines WHERE worker_id = ?")
        .bind(id)
        .execute(&state.db)
        .await?;
    jobs::disconnect(&state, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── The connection ────────────────────────────────────────────────────

async fn authenticate(state: &AppState, headers: &HeaderMap) -> ApiResult<MachineRow> {
    let invalid = || {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            "This machine's token is not valid, or was revoked",
        )
    };
    let token = auth::bearer_token(headers).ok_or_else(invalid)?;
    let row: Option<MachineRow> = sqlx::query_as(concat!(
        machine_query!(),
        " WHERE w.token_hash = ? AND w.revoked_at IS NULL"
    ))
    .bind(secret::token_hash(token))
    .fetch_optional(&state.db)
    .await?;
    row.ok_or_else(invalid)
}

/// The machines' WebSocket.
pub async fn connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> ApiResult<Response> {
    let machine = authenticate(&state, &headers).await?;
    let tracked = state.sockets.token();
    Ok(upgrade.on_upgrade(move |socket| async move {
        let _tracked = tracked;
        let id = machine.id;
        if let Err(error) = serve(state, machine, socket).await {
            tracing::warn!(worker = id, %error, "machine connection ended");
        }
    }))
}

async fn send(socket: &mut WebSocket, message: &HubMessage) -> anyhow::Result<()> {
    let text = serde_json::to_string(message)?;
    socket.send(WsMessage::Text(text.into())).await?;
    Ok(())
}

async fn serve(state: AppState, machine: MachineRow, mut socket: WebSocket) -> anyhow::Result<()> {
    // The first message says what the machine can do.
    let first = tokio::select! {
        biased;
        _ = state.shutdown.cancelled() => { close(&mut socket).await; return Ok(()); }
        first = tokio::time::timeout(Duration::from_secs(15), socket.next()) => first,
    };
    let hello = match first {
        Ok(Some(Ok(WsMessage::Text(text)))) => match serde_json::from_str(&text) {
            Ok(WorkerMessage::Hello(hello)) => hello,
            _ => anyhow::bail!("the machine did not start with a hello"),
        },
        _ => anyhow::bail!("the machine said nothing"),
    };
    if hello.protocol != PROTOCOL_VERSION {
        let reason = format!(
            "This hub speaks version {PROTOCOL_VERSION} of the worker protocol, and the machine \
             version {}: update the one that is behind",
            hello.protocol
        );
        send(&mut socket, &HubMessage::Refused { reason }).await?;
        return Ok(());
    }
    sqlx::query(
        "UPDATE workers SET last_seen_at = ?, runners = ?, lean = ?, latex = ?, isolation = ?,
                            version = ?
         WHERE id = ?",
    )
    .bind(db::now())
    .bind(serde_json::to_string(&hello.runners)?)
    .bind(hello.lean)
    .bind(hello.latex)
    .bind(hello.isolation.map(Isolation::as_str))
    .bind(&hello.version)
    .bind(machine.id)
    .execute(&state.db)
    .await?;
    send(
        &mut socket,
        &HubMessage::Welcome(Welcome {
            protocol: PROTOCOL_VERSION,
            name: machine.name.clone(),
            owner: machine.owner(),
        }),
    )
    .await?;
    tracing::info!(worker = machine.id, name = machine.name, runners = ?hello.runners, "machine connected");

    let connection = state.connections.fetch_add(1, Ordering::Relaxed) + 1;
    let (sender, mut outbox) = mpsc::unbounded_channel();
    let workshop = machines::groups_of(&state, machine.id)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    jobs::connected(
        &state,
        machine.id,
        Connected {
            personal: hello.personal,
            connection,
            owner_id: machine.owner_id,
            runners: hello.runners,
            lean: hello.lean,
            latex: hello.latex,
            isolation: hello.isolation,
            workshop,
            concurrency: hello.concurrency.clamp(1, 64),
            preferred: machine.preferred,
            running: Default::default(),
            sender,
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!(e.message))?;

    let ended = talk(&state, machine.id, &mut socket, &mut outbox).await;
    close(&mut socket).await;
    sqlx::query("UPDATE workers SET last_seen_at = ? WHERE id = ?")
        .bind(db::now())
        .bind(machine.id)
        .execute(&state.db)
        .await?;
    jobs::disconnected(&state, machine.id, connection)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    tracing::info!(worker = machine.id, "machine disconnected");
    ended
}

/// Send a normal close, bounded even if the peer stops reading.
async fn close(socket: &mut WebSocket) {
    let _ = tokio::time::timeout(
        Duration::from_secs(1),
        socket.send(WsMessage::Close(Some(axum::extract::ws::CloseFrame {
            code: axum::extract::ws::close_code::NORMAL,
            reason: "Connection closed".into(),
        }))),
    )
    .await;
}

/// Passes messages both ways until the machine goes, or is replaced.
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
            biased;
            _ = state.shutdown.cancelled() => return Ok(()),
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
                        tracing::warn!(worker = worker_id, %error, "unreadable message from a machine");
                        continue;
                    }
                };
                let handled = match message {
                    WorkerMessage::Hello(_) => Ok(()),
                    WorkerMessage::Progress { job, text } => {
                        jobs::progress(state, worker_id, job, &text).await
                    }
                    WorkerMessage::Finished { job, outcome, usage } => {
                        jobs::finished(state, worker_id, job, outcome, usage).await
                    }
                };
                if let Err(error) = handled {
                    tracing::error!(worker = worker_id, error = error.message, "handling a machine's message");
                }
            }
            _ = ping.tick() => {
                if heard.elapsed() > SILENCE_LIMIT {
                    anyhow::bail!("the machine went silent");
                }
                socket.send(WsMessage::Ping(Default::default())).await?;
            }
        }
    }
}
