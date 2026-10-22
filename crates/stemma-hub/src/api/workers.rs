//! Live computers authenticate with personal tokens; no machine records exist.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use futures_util::StreamExt;
use stemma_core::api::User;
use stemma_core::protocol::{HubMessage, PROTOCOL_VERSION, Welcome, WorkerMessage};
use tokio::sync::mpsc;

use crate::auth;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::jobs::{self, Connected};
use crate::secret;
use crate::state::AppState;

/// How often the hub pings a machine, and how long it waits to hear from
/// it.
const PING_EVERY: Duration = Duration::from_secs(20);
const SILENCE_LIMIT: Duration = Duration::from_secs(70);

#[derive(sqlx::FromRow)]
struct MachineRow {
    id: i64,
    name: String,
    owner_id: i64,
    username: String,
    display_name: String,
}
impl MachineRow {
    fn owner(&self) -> User {
        User {
            id: self.owner_id,
            username: self.username.clone(),
            display_name: self.display_name.clone(),
        }
    }
}
async fn authenticate(state: &AppState, headers: &HeaderMap) -> ApiResult<MachineRow> {
    let invalid = || {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            "This computer is signed out. Sign in again.",
        )
    };
    let token = auth::bearer_token(headers).ok_or_else(invalid)?;
    let row = sqlx::query_as("SELECT t.id, t.name, t.user_id AS owner_id, u.username, u.display_name FROM api_tokens t JOIN users u ON u.id = t.user_id WHERE t.token_hash = ? AND t.kind = 'personal' AND t.revoked_at IS NULL AND (t.expires_at IS NULL OR t.expires_at > ?)")
        .bind(secret::token_hash(token)).bind(db::now()).fetch_optional(&state.db).await?;
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
    send(
        &mut socket,
        &HubMessage::Welcome(Welcome {
            groups: super::groups::groups_of(&state, machine.owner_id)
                .await
                .map_err(|e| anyhow::anyhow!(e.message))?,
            protocol: PROTOCOL_VERSION,
            name: machine.name.clone(),
            owner: machine.owner(),
        }),
    )
    .await?;
    tracing::info!(worker = machine.id, name = machine.name, runners = ?hello.runners, "machine connected");

    let connection = state.connections.fetch_add(1, Ordering::Relaxed) + 1;
    let (sender, mut outbox) = mpsc::unbounded_channel();
    jobs::connected(
        &state,
        machine.id,
        Connected {
            connection,
            owner_id: machine.owner_id,
            runners: hello.runners,
            groups: hello.groups,
            agents: hello.agents,
            concurrency: hello.concurrency.max(1),
            running: Default::default(),
            sender,
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!(e.message))?;

    let ended = talk(&state, machine.id, connection, &mut socket, &mut outbox).await;
    close(&mut socket).await;
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
    connection: u64,
    socket: &mut WebSocket,
    outbox: &mut mpsc::UnboundedReceiver<HubMessage>,
) -> anyhow::Result<()> {
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
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
                    WorkerMessage::Hello(hello) => {
                        anyhow::ensure!(hello.protocol == PROTOCOL_VERSION, "worker protocol changed");
                        jobs::hello(state, worker_id, connection, hello).await
                    },
                    WorkerMessage::Progress { job, text } => {
                        jobs::progress(state, worker_id, connection, job, &text).await
                    }
                    WorkerMessage::Finished { job, outcome, usage } => {
                        jobs::finished(state, worker_id, connection, job, outcome, usage).await
                    }
                };
                if let Err(error) = handled {
                    tracing::error!(worker = worker_id, error = error.message, "handling a machine's message");
                }
            }
            _ = ping.tick() => {
                let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM api_tokens WHERE id = ? AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > ?))").bind(worker_id).bind(db::now()).fetch_one(&state.db).await?;
                if !valid { return Ok(()); }

                if heard.elapsed() > SILENCE_LIMIT {
                    anyhow::bail!("the machine went silent");
                }
                let owner: i64 = sqlx::query_scalar("SELECT user_id FROM api_tokens WHERE id = ?").bind(worker_id).fetch_one(&state.db).await?;
                send(socket, &HubMessage::Identity(super::account::me_of(state, owner).await.map_err(|e| anyhow::anyhow!(e.message))?)).await?;
                socket.send(WsMessage::Ping(Default::default())).await?;
            }
        }
    }
}
