//! This computer's service and agents' image.
use clap::{Args, Subcommand};
use stemma_worker::{image, isolation, service};

#[derive(Args)]
pub struct MachineArgs {
    #[command(subcommand)]
    command: Option<MachineCommand>,
}
#[derive(Subcommand)]
enum MachineCommand {
    /// Install and start the service at login.
    Start,
    /// Stop and remove the service.
    Stop,
    /// Run the service in this terminal.
    Run,
    /// Build the agents' image for Docker or Podman.
    Image,
}
pub async fn machine(args: MachineArgs) -> anyhow::Result<()> {
    match args.command {
        Some(MachineCommand::Start) => {
            let _ = stemma_client::config::Credentials::load()?
                .with_env()
                .hub()?
                .me()
                .await?;
            let progress = indicatif::ProgressBar::new_spinner();
            progress.set_style(stemma_design_terminal::themes::indicatif::spinner(
                &crate::output::stdout(),
            ));
            progress.set_message("Starting your agents");
            progress.enable_steady_tick(std::time::Duration::from_millis(100));
            service::install(&std::env::current_exe()?)?;
            progress.finish_and_clear();
            println!("Service started.");
        }
        Some(MachineCommand::Stop) => {
            service::uninstall()?;
            println!("Service stopped and removed.");
        }
        Some(MachineCommand::Run) => stemma_worker::machine::run(std::env::current_exe()?).await?,
        Some(MachineCommand::Image) => {
            let config = stemma_worker::WorkerConfig::load(None)?;
            let runtime = match config.isolation.runtime() {
                isolation::Runtime::Podman => isolation::Runtime::Podman,
                _ => isolation::Runtime::Docker,
            };
            let mut isolation = config.isolation.clone();
            isolation.runtime = Some(runtime);
            let engine = isolation::find_engine(&isolation, std::env::var_os("PATH").as_deref())
                .await
                .ok_or_else(|| anyhow::anyhow!("Install and start Docker or Podman"))?;
            let build = image::Build {
                tag: config.isolation.image(),
                source: image::Source::for_this_machine(),
                toolchain: image::TOOLCHAIN.into(),
                version: env!("CARGO_PKG_VERSION").into(),
            };
            image::build(engine.as_ref(), &build).await?;
        }
        None => {
            let status = service::status().await?;
            println!(
                "{} – {}",
                if status.signed_out {
                    "Signed out"
                } else if status.connected == Some(true) {
                    "Connected"
                } else {
                    "Offline"
                },
                if status.running {
                    "Service running"
                } else {
                    "Service stopped"
                }
            );
            println!("Logs: {}", status.logs);
        }
    }
    Ok(())
}
