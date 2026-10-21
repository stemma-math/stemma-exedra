//! `stemma claude`, `stemma codex`, `stemma opencode`: the person's agent,
//! interactively, with everything it needs to work with Stemma for the
//! session (see [`stemma_worker::session`]).

use anyhow::Context as _;
use clap::Args;
use stemma_client::config::Credentials;
use stemma_core::api::runners;
use stemma_worker::session::{Options, Session};

use crate::client::hostname;

#[derive(Args)]
pub struct LaunchArgs {
    /// The group to work with; the one chosen at login if absent.
    #[arg(long, short, env = "STEMMA_GROUP")]
    group: Option<String>,
    /// More arguments for the agent, after `--`.
    #[arg(last = true)]
    args: Vec<String>,
}

/// Opens the person's agent of `runner`, waits until it exits, and says
/// how it exited.
pub async fn launch(runner: &str, args: LaunchArgs) -> anyhow::Result<i32> {
    let config = Credentials::load()?.with_env();
    let hub = config.hub()?;
    let options = Options {
        runner: runner.to_owned(),
        group: stemma_client::config::active_group(args.group)?,
        args: args.args,
        stemma: std::env::current_exe().context("finding the stemma binary")?,
        program: None,
        temp_dir: None,
        host: hostname(),
    };
    let session = Session::open(&hub, options).await?;
    eprintln!(
        "{} for {} in {}, with Stemma's tools.",
        session.agent.display_name, session.person.display_name, session.group.name
    );
    let program = session.prepared.program.clone();
    let mut child = match session
        .prepared
        .command()
        .current_dir(&session.workspace.path)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            session.close(&hub).await?;
            return Err(error).with_context(|| {
                format!("running {program}: is {} installed?", runners::name(runner))
            });
        }
    };
    // Ctrl-C is the agent's to handle: this waits until it exits.
    let status = loop {
        tokio::select! {
            status = child.wait() => break status,
            _ = tokio::signal::ctrl_c() => {}
        }
    };
    session.close(&hub).await?;
    Ok(status?.code().unwrap_or(1))
}
