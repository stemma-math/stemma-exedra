//! This computer's service and agents' image.
use clap::{Args, Subcommand};
use stemma_worker::{image, service};

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
            image::build_for(None).await?;
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
