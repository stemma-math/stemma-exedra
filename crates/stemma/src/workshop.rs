//! Approval commands act on this computer only, never through a hub API.
use clap::{Args, Subcommand};
use stemma_worker::approvals::{self, Limits};
#[derive(Args)]
pub struct WorkshopArgs {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// List offers waiting for this computer's owner.
    Pending,
    /// Approve one offer, its recurring series or this recipe version from its requester.
    Approve {
        process: i64,
        #[arg(long, conflicts_with = "recipe")]
        series: bool,
        #[arg(long)]
        recipe: bool,
        /// Last allowed date and time, in RFC 3339 format.
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        runs: Option<u32>,
        #[arg(long)]
        minutes: Option<u32>,
    },
    /// Decline a pending offer.
    Decline { process: i64 },
    /// List standing approvals and their remaining limits.
    Approvals,
    /// Revoke a standing approval on this computer.
    Revoke { approval: u64 },
    /// Read this computer's workshop decision and run log.
    Log,
}
pub fn run(args: WorkshopArgs) -> anyhow::Result<()> {
    let paths = stemma_client::config::Paths::discover()?;
    let credentials = stemma_client::config::Credentials::load()?.with_env();
    let hub = credentials
        .hub
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Sign in to a hub first"))?;
    match args.command {
        Command::Pending => {
            let s = approvals::read(&paths.state)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &s.pending
                        .iter()
                        .filter(|p| p.hub == hub
                            && p.decision == "pending"
                            && p.offer.expires_at > approvals::now())
                        .collect::<Vec<_>>()
                )?
            );
        }
        Command::Approvals => {
            let s = approvals::read(&paths.state)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &s.approvals
                        .iter()
                        .filter(|a| a.hub == hub)
                        .collect::<Vec<_>>()
                )?
            );
        }
        Command::Approve {
            process,
            series,
            recipe,
            until,
            runs,
            minutes,
        } => {
            let until = until
                .map(|s| {
                    time::OffsetDateTime::parse(&s, &time::format_description::well_known::Rfc3339)
                        .map(|d| (d.unix_timestamp_nanos() / 1_000_000) as i64)
                })
                .transpose()?;
            anyhow::ensure!(
                series || recipe || (until.is_none() && runs.is_none() && minutes.is_none()),
                "Limits apply to a series or recipe approval"
            );
            approvals::approve(
                &paths.state,
                hub,
                process,
                if series {
                    "series"
                } else if recipe {
                    "recipe"
                } else {
                    "once"
                },
                Limits {
                    until,
                    runs,
                    minutes,
                },
            )?;
            println!("Approved process {process} on this computer.");
        }
        Command::Decline { process } => {
            approvals::decline(&paths.state, hub, process)?;
            println!("Declined process {process}.");
        }
        Command::Revoke { approval } => {
            approvals::revoke(&paths.state, hub, approval)?;
            println!("Revoked approval {approval}.");
        }
        Command::Log => {
            let path = paths.state.join("logs/workshop.jsonl");
            if path.exists() {
                print!("{}", std::fs::read_to_string(path)?);
            }
        }
    }
    Ok(())
}
