//! One foreground service per computer, authenticated with its personal token.
use anyhow::Context as _;
use std::path::Path;
use stemma_client::config::{Credentials, Paths};
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, thiserror::Error)]
pub enum Stopped {
    #[error("This computer is signed out. Sign in again.")]
    SignedOut,
    #[error("The hub refused this computer: {0}")]
    Refused(String),
}
pub async fn run(stemma: std::path::PathBuf) -> anyhow::Result<()> {
    let paths = Paths::discover()?;
    std::fs::create_dir_all(&paths.state)?;
    let _lock = lock(
        &paths.state.join("machine.lock"),
        std::time::Duration::from_secs(2),
    )
    .await?;
    let credentials = Credentials::load()?.with_env();
    let token = credentials
        .token
        .context("This computer is signed out. Run stemma login.")?;
    let context =
        std::sync::Arc::new(crate::Context::start(crate::WorkerConfig::load(None)?, stemma).await?);
    tokio::select! {
        served = crate::connection::serve(context, token) => match served {
            Err(error) if error.downcast_ref::<Stopped>().is_some() => { tracing::warn!("{error}"); Ok(()) },
            result => result,
        },
        _ = tokio::signal::ctrl_c() => Ok(()),
        _ = terminate() => Ok(()),
    }
}
async fn terminate() {
    #[cfg(unix)]
    {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    }
    #[cfg(not(unix))]
    std::future::pending::<()>().await;
}
async fn lock(path: &Path, patience: std::time::Duration) -> anyhow::Result<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    let deadline = tokio::time::Instant::now() + patience;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(error) => return Err(anyhow::Error::from(error).context(
                "Another process is already serving this machine; stop it before running `stemma machine run`",
            )),
        }
    }
}
