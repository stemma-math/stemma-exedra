//! The connection to the hub: outbound, over a WebSocket, kept up with
//! backoff. See [`stemma_core::protocol`] for what is said on it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use futures_util::{SinkExt, StreamExt};
use stemma_core::api::{JobKind, PREFIX};
use stemma_core::protocol::{
    Hello, HubMessage, Outcome, PROTOCOL_VERSION, Usage, WORKER_PATH, WorkerMessage,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode, header};
use tokio_tungstenite::tungstenite::{self, Message as WsMessage};
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

use crate::job::{self, Context, Progress};

/// How long the hub may stay silent before the worker reconnects; the hub
/// pings every 20 seconds.
const SILENCE_LIMIT: Duration = Duration::from_secs(75);
const LONGEST_WAIT: Duration = Duration::from_secs(60);

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Why a connection ended.
enum Ended {
    /// The hub went away, or the connection dropped: try again.
    Dropped(anyhow::Error),
    /// The hub will not take this worker: stop.
    Refused(anyhow::Error),
}

/// Stays connected to the hub, running its jobs, until stopped or refused.
pub async fn serve(context: Arc<Context>, token: String) -> anyhow::Result<()> {
    let mut wait = Duration::from_secs(1);
    loop {
        let started = Instant::now();
        match session(&context, &token).await {
            Ended::Refused(error) => return Err(error),
            Ended::Dropped(error) => {
                tracing::warn!(error = format!("{error:#}"), "disconnected from the hub");
            }
        }
        // A connection that lasted resets the backoff.
        if started.elapsed() > Duration::from_secs(60) {
            wait = Duration::from_secs(1);
        }
        let jitter = Duration::from_millis(rand::random::<u64>() % 1_000);
        tracing::info!("reconnecting in {}s", (wait + jitter).as_secs());
        tokio::time::sleep(wait + jitter).await;
        wait = (wait * 2).min(LONGEST_WAIT);
    }
}

/// The worker's WebSocket address on the hub.
pub fn socket_url(hub: &str) -> anyhow::Result<String> {
    let hub = hub.trim_end_matches('/');
    let url = if let Some(rest) = hub.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = hub.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        anyhow::bail!("a hub's address starts with http:// or https://")
    };
    Ok(format!("{url}{PREFIX}{WORKER_PATH}"))
}

fn tls() -> anyhow::Result<Connector> {
    use rustls_platform_verifier::ConfigVerifierExt;
    stemma_client::hub::install_crypto();
    let config = rustls::ClientConfig::with_platform_verifier()?;
    Ok(Connector::Rustls(Arc::new(config)))
}

async fn connect(context: &Context, token: &str) -> Result<Socket, Ended> {
    let url = socket_url(&context.hub).map_err(Ended::Refused)?;
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| Ended::Refused(e.into()))?;
    let bearer = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| Ended::Refused(anyhow::anyhow!("the token is not valid text")))?;
    request.headers_mut().insert(header::AUTHORIZATION, bearer);
    let connector = tls().map_err(Ended::Refused)?;
    match tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector))
        .await
    {
        Ok((socket, _)) => Ok(socket),
        Err(tungstenite::Error::Http(response))
            if response.status() == StatusCode::UNAUTHORIZED =>
        {
            Err(Ended::Refused(crate::machine::Stopped::SignedOut.into()))
        }
        Err(error) => Err(Ended::Dropped(
            anyhow::Error::new(error).context(format!("connecting to {url}")),
        )),
    }
}

async fn send(socket: &mut Socket, message: &WorkerMessage) -> anyhow::Result<()> {
    let text = serde_json::to_string(message)?;
    socket.send(WsMessage::Text(text.into())).await?;
    Ok(())
}

/// One connection, from hello to its end.
async fn session(context: &Arc<Context>, token: &str) -> Ended {
    let mut socket = match connect(context, token).await {
        Ok(socket) => socket,
        Err(ended) => return ended,
    };
    let hello = match hello().await {
        Ok(hello) => WorkerMessage::Hello(hello),
        Err(e) => return Ended::Refused(e),
    };
    if let Err(error) = send(&mut socket, &hello).await {
        return Ended::Dropped(error);
    }
    match welcome(&mut socket).await {
        Ok(Ok(welcome)) => {
            if let Err(error) = context
                .config
                .paths
                .reconcile_groups(&context.hub, &welcome.groups)
                .and_then(|()| {
                    context.config.paths.update_username(
                        &context.hub,
                        token,
                        &welcome.owner.username,
                    )
                })
            {
                tracing::warn!(%error, "could not reconcile names; will retry");
            }

            tracing::info!(
                "connected to {} as {}, one of {}'s machines; runners: {}; Lean: {}; LaTeX: {}",
                context.hub,
                welcome.name,
                welcome.owner.display_name,
                context.config.runner_names().join(", "),
                "local tools",
                "local tools"
            );
        }
        Ok(Err(reason)) => return Ended::Refused(crate::machine::Stopped::Refused(reason).into()),
        Err(error) => return Ended::Dropped(error),
    }
    let ended = work(context, &mut socket, RunningJobs::default()).await;
    let _ = socket.close(None).await;
    Ended::Dropped(ended)
}

async fn welcome(
    socket: &mut Socket,
) -> anyhow::Result<Result<stemma_core::protocol::Welcome, String>> {
    let reply = tokio::time::timeout(Duration::from_secs(15), socket.next())
        .await
        .context("the hub did not answer the hello")?;
    match reply {
        Some(Ok(WsMessage::Text(text))) => match serde_json::from_str(&text)? {
            HubMessage::Welcome(welcome) => Ok(Ok(welcome)),
            HubMessage::Refused { reason } => Ok(Err(reason)),
            _ => anyhow::bail!("the hub did not start with a welcome"),
        },
        Some(Ok(_)) => anyhow::bail!("the hub did not start with a welcome"),
        Some(Err(error)) => Err(error.into()),
        None => anyhow::bail!("the hub closed the connection"),
    }
}

/// Aborting the service must also abort jobs; dropping a JoinHandle detaches it.
#[derive(Default)]
struct RunningJobs(HashMap<i64, (JobKind, JoinHandle<()>)>);

impl RunningJobs {
    fn accepts(&self, id: i64, kind: JobKind, concurrency: u32) -> bool {
        let _ = kind;
        !self.0.contains_key(&id) && self.0.len() < concurrency as usize
    }
}

impl Drop for RunningJobs {
    fn drop(&mut self) {
        for (_, task) in self.0.values() {
            task.abort();
        }
    }
}

/// Runs the hub's jobs until the connection ends; then stops them, for the
/// hub queues them again.
async fn work(context: &Arc<Context>, socket: &mut Socket, mut jobs: RunningJobs) -> anyhow::Error {
    let (progress, mut progress_out) = mpsc::unbounded_channel::<(i64, String)>();
    let (finished, mut finished_out) = mpsc::unbounded_channel::<(i64, Outcome, Usage)>();
    let mut heard = Instant::now();
    let mut check = tokio::time::interval(Duration::from_secs(2));
    let mut last_hello = String::new();
    let mut settings =
        stemma_client::config::Settings::load().unwrap_or_else(|_| context.config.settings.clone());
    let mut modification = None;

    loop {
        tokio::select! {
            incoming = socket.next() => {
                heard = Instant::now();
                let text = match incoming {
                    Some(Ok(WsMessage::Text(text))) => text,
                    Some(Ok(WsMessage::Close(_))) | None => break anyhow::anyhow!("the hub closed the connection"),
                    Some(Ok(_)) => continue,
                    Some(Err(error)) => break error.into(),
                };
                match serde_json::from_str::<HubMessage>(&text) {
                    Ok(HubMessage::Job(spec)) => {
                        let id = spec.id;
                        tracing::info!(job = id, kind = spec.kind().as_str(), "job received");
                        let group = stemma_core::api::Group { id: spec.group.id, slug: spec.group.slug.clone(), name: spec.group.name.clone(), role: stemma_core::api::Role::Member, created_at: time::OffsetDateTime::UNIX_EPOCH };
                        if let Err(error) = context.config.paths.reconcile_groups(&context.hub, &[group]) { tracing::warn!(%error, "could not reconcile a job's group name"); }
                        if let Ok(updated) = stemma_client::config::Settings::load() { settings = updated; }
                        let local = match context.config.paths.group_slug(&context.hub, spec.group.id, &spec.group.slug) { Ok(local) => local, Err(error) => break error };
                        let credentials = stemma_client::config::Credentials { hub: Some(context.hub.clone()), ..Default::default() };
                        let config = match crate::config::WorkerConfig::resolve(&settings, &context.config.paths, &credentials, Some(&local)) { Ok(c) => c, Err(error) => { break error; } };
                        if !jobs.accepts(id, spec.kind(), config.concurrency) {
                            let message = WorkerMessage::Finished {
                                job: id,
                                outcome: Outcome::Failed { error: "This machine is at its job limit".into() },
                                usage: Usage::default(),
                            };
                            if let Err(error) = send(socket, &message).await { break error; }
                            continue;
                        }
                        let kind = spec.kind();
                        let context = match Context::start(config, context.stemma.clone()).await {
                            Ok(c) => Arc::new(c),
                            Err(error) => {
                                let message = WorkerMessage::Finished { job: id, outcome: Outcome::Failed { error: format!("{error:#}") }, usage: Usage::default() };
                                if let Err(error) = send(socket, &message).await { break error; }
                                continue;
                            }
                        };
                        let progress = Progress::new(id, progress.clone());
                        let finished = finished.clone();
                        let task = tokio::spawn(async move {
                            let (outcome, usage) = job::run(context, *spec, progress).await;
                            let _ = finished.send((id, outcome, usage));
                        });
                        jobs.0.insert(id, (kind, task));
                    }
                    Ok(HubMessage::Cancel { job }) => {
                        if let Some((_, task)) = jobs.0.remove(&job) {
                            tracing::info!(job, "job cancelled");
                            task.abort();
                        }
                    }
                    Ok(HubMessage::Refused { reason }) => break anyhow::anyhow!("the hub refused: {reason}"),
                    Ok(HubMessage::Identity(me)) => {
                        let token = stemma_client::config::Credentials::load().ok().and_then(|c| c.with_env().token).unwrap_or_default();
                        if let Err(error) = context.config.paths.update_username(&context.hub, &token, &me.user.username) { tracing::warn!(%error, "could not update the saved username"); }
                        if let Err(error) = context.config.paths.reconcile_groups(&context.hub, &me.groups) {
                                tracing::warn!(%error, "could not reconcile names; will retry");
                            }
                        if let Ok(updated) = stemma_client::config::Settings::load() { settings = updated; }
                    }
                    Ok(HubMessage::Welcome(_)) => {}
                    Err(error) => tracing::warn!(%error, "unreadable message from the hub"),
                }
            }
            Some((job, text)) = progress_out.recv() => {
                if jobs.0.contains_key(&job)
                    && let Err(error) = send(socket, &WorkerMessage::Progress { job, text }).await
                {
                    break error;
                }
            }
            Some((job, outcome, usage)) = finished_out.recv() => {
                if jobs.0.remove(&job).is_none() {
                    continue;
                }
                tracing::info!(job, failed = matches!(outcome, Outcome::Failed { .. }), "job finished");
                if let Err(error) = send(socket, &WorkerMessage::Finished { job, outcome, usage }).await {
                    break error;
                }
            }
            _ = check.tick() => {
                let observed = std::fs::metadata(&settings.path).and_then(|m| m.modified()).ok();
                if observed != modification {
                    modification = observed;
                    match stemma_client::config::Settings::load() {
                        Ok(updated) => match hello_from(&updated).await {
                            Ok(hello) => {
                                let serialized = serde_json::to_string(&hello).unwrap();
                                if serialized != last_hello {
                                    if let Err(error) = send(socket, &WorkerMessage::Hello(hello)).await { break error; }
                                    last_hello = serialized;
                                }
                                settings = updated;
                            }
                            Err(error) => tracing::warn!(%error, "could not reload capabilities"),
                        },
                        Err(error) => tracing::warn!(%error, "configuration is invalid; keeping the last valid settings"),
                    }
                }
                if heard.elapsed() > SILENCE_LIMIT {
                    break anyhow::anyhow!("the hub went silent");
                }
            }
        }
    }
}

/// Current live capabilities; the same resolution supplies jobs and the app.
pub async fn hello() -> anyhow::Result<Hello> {
    let settings = stemma_client::config::Settings::load()?;
    hello_from(&settings).await
}

async fn hello_from(settings: &stemma_client::config::Settings) -> anyhow::Result<Hello> {
    let paths = stemma_client::config::Paths::discover()?;
    let credentials = stemma_client::config::Credentials::load()?.with_env();
    let resolve =
        |group| crate::config::WorkerConfig::resolve(settings, &paths, &credentials, group);
    let scope = |group| -> anyhow::Result<stemma_core::protocol::GroupAgents> {
        Ok(stemma_core::protocol::GroupAgents {
            attachments: settings.attachments(group)?,
            enabled: settings
                .get("agents.enabled", group)?
                .value
                .as_bool()
                .unwrap(),
            priority: settings.priority(group)?,
            runners: None,
            concurrency: Some(
                settings
                    .get("agents.concurrency", group)?
                    .value
                    .as_integer()
                    .unwrap() as u32,
            ),
        })
    };
    let agents = scope(None)?;
    let config = resolve(None)?;
    let found = available(&config).await;
    let mut groups = std::collections::BTreeMap::new();
    for slug in settings.groups() {
        let mut group = scope(Some(slug))?;
        let configured = resolve(Some(slug))?;
        let available = available(&configured).await;
        if available != found {
            group.runners = Some(available);
        }
        if group.enabled != agents.enabled
            || group.priority != agents.priority
            || group.concurrency != agents.concurrency
            || group.attachments != agents.attachments
            || group.runners.is_some()
        {
            groups.insert(slug.clone(), group);
        }
    }
    Ok(Hello {
        protocol: PROTOCOL_VERSION,
        version: env!("CARGO_PKG_VERSION").into(),
        runners: found,
        concurrency: config.concurrency,
        agents,
        groups,
    })
}

async fn available(config: &crate::config::WorkerConfig) -> Vec<String> {
    match crate::isolation::resolve(
        &config.isolation,
        &config.hub,
        std::env::var_os("PATH").as_deref(),
    )
    .await
    {
        Ok(crate::isolation::Isolator::Host) => {
            crate::detect::installed(&config.runners, std::env::var_os("PATH").as_deref()).await
        }
        Ok(crate::isolation::Isolator::Containers(_)) => config.runner_names(),
        Err(reason) => {
            tracing::warn!(%reason, "agents cannot run with this isolation configuration");
            Vec::new()
        }
    }
}
