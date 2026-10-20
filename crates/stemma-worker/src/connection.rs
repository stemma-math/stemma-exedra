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
            Err(Ended::Refused(crate::machine::Stopped::NotKnown.into()))
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
    let hello = WorkerMessage::Hello(Hello {
        personal: context.config.personal,
        protocol: PROTOCOL_VERSION,
        version: env!("CARGO_PKG_VERSION").into(),
        runners: context.config.runner_names(),
        lean: lean(context),
        latex: latex(context),
        isolation: context.isolator.as_ref().map(|i| i.isolation()),
        concurrency: context.config.concurrency,
    });
    if let Err(error) = send(&mut socket, &hello).await {
        return Ended::Dropped(error);
    }
    match welcome(&mut socket).await {
        Ok(Ok(welcome)) => {
            tracing::info!(
                "connected to {} as {}, one of {}'s machines; runners: {}; Lean: {}; LaTeX: {}",
                context.hub,
                welcome.name,
                welcome.owner.display_name,
                context.config.runner_names().join(", "),
                if lean(context) { "yes" } else { "no" },
                if latex(context) { "yes" } else { "no" }
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
        let paper = kind == JobKind::PaperBuild
            || self
                .0
                .values()
                .any(|(kind, _)| *kind == JobKind::PaperBuild);
        !self.0.contains_key(&id) && self.0.len() < concurrency as usize + usize::from(paper)
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
    let mut check = tokio::time::interval(Duration::from_secs(15));

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
                        if !jobs.accepts(id, spec.kind(), context.config.concurrency) {
                            let message = WorkerMessage::Finished {
                                job: id,
                                outcome: Outcome::Failed { error: "This machine is at its job limit".into() },
                                usage: Usage::default(),
                            };
                            if let Err(error) = send(socket, &message).await { break error; }
                            continue;
                        }
                        let kind = spec.kind();
                        let context = context.clone();
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
                if heard.elapsed() > SILENCE_LIMIT {
                    break anyhow::anyhow!("the hub went silent");
                }
            }
        }
    }
}

/// Whether the workshop's jobs have Lean: in the image of their
/// containers, or else on the machine.
fn lean(context: &Context) -> bool {
    match &context.isolator {
        Some(crate::isolation::Isolator::Containers(_)) => true,
        _ => has_lean(),
    }
}

/// Whether the workshop's jobs have LaTeX: in the image of their
/// containers, or else on the machine.
fn latex(context: &Context) -> bool {
    match &context.isolator {
        Some(crate::isolation::Isolator::Containers(_)) => true,
        _ => crate::papers::has_latex(),
    }
}

/// Whether Lean is installed: `elan` or `lake` on the path.
pub fn has_lean() -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join("elan").is_file() || dir.join("lake").is_file())
}

#[cfg(test)]
mod tests {
    use super::{JobKind, socket_url};

    #[tokio::test]
    async fn dropping_the_connection_cancels_its_jobs() {
        let (sent, received) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let _sender = sent;
            std::future::pending::<()>().await;
        });
        let mut jobs = super::RunningJobs::default();
        jobs.0.insert(1, (JobKind::Reply, task));
        drop(jobs);
        assert!(received.await.is_err());
    }

    #[tokio::test]
    async fn enforces_normal_and_paper_slots() {
        let mut jobs = super::RunningJobs::default();
        let task = || tokio::spawn(std::future::pending());
        assert!(jobs.accepts(1, JobKind::Reply, 1));
        jobs.0.insert(1, (JobKind::Reply, task()));
        assert!(!jobs.accepts(2, JobKind::Reply, 1));
        assert!(jobs.accepts(2, JobKind::PaperBuild, 1));
        jobs.0.insert(2, (JobKind::PaperBuild, task()));
        assert!(!jobs.accepts(3, JobKind::PaperBuild, 1));
        assert!(!jobs.accepts(3, JobKind::Reply, 1));
        jobs.0.remove(&1).unwrap().1.abort();
        assert!(jobs.accepts(3, JobKind::Reply, 1));
        assert!(!jobs.accepts(2, JobKind::PaperBuild, 1));
    }

    #[tokio::test]
    async fn reports_excess_dispatch_as_failed_without_running_it() {
        use futures_util::{SinkExt, StreamExt};
        use stemma_core::protocol::{HubMessage, Outcome, WorkerMessage};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let hub = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let spec = serde_json::from_value(serde_json::json!({
                "id": 3, "group": {"slug": "geometry", "name": "Geometry"}, "token": "unused",
                "work": {"kind": "paper_build", "build": 1, "paper": "notes", "repository": "/unused",
                         "commit": "unused", "main": "main.tex", "upload": "/unused"}
            })).unwrap();
            socket
                .send(super::WsMessage::Text(
                    serde_json::to_string(&HubMessage::Job(Box::new(spec)))
                        .unwrap()
                        .into(),
                ))
                .await
                .unwrap();
            let reply = socket.next().await.unwrap().unwrap().into_text().unwrap();
            assert!(matches!(
                serde_json::from_str::<WorkerMessage>(&reply).unwrap(),
                WorkerMessage::Finished {
                    job: 3,
                    outcome: Outcome::Failed { .. },
                    ..
                }
            ));
            socket.close(None).await.unwrap();
        });
        let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let context = std::sync::Arc::new(crate::job::Context {
            config: toml::from_str("hub = 'http://unused'\nconcurrency = 1").unwrap(),
            hub: "http://unused".into(),
            stemma: dir.path().join("unused"),
            work_dir: dir.path().to_owned(),
            checkouts: Default::default(),
            isolator: None,
        });
        let mut jobs = super::RunningJobs::default();
        for (id, kind) in [(1, JobKind::Reply), (2, JobKind::PaperBuild)] {
            jobs.0
                .insert(id, (kind, tokio::spawn(std::future::pending())));
        }
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            super::work(&context, &mut socket, jobs).await;
            hub.await.unwrap();
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn socket_urls() {
        assert_eq!(
            socket_url("https://stemma.example.org/").unwrap(),
            "wss://stemma.example.org/api/v1/worker"
        );
        assert_eq!(
            socket_url("http://127.0.0.1:8080").unwrap(),
            "ws://127.0.0.1:8080/api/v1/worker"
        );
        assert!(socket_url("stemma.example.org").is_err());
    }
}
