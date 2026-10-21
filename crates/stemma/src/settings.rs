//! Account group state and the shared settings editor.

use anyhow::Context;
use clap::{Args, Subcommand};
use stemma_client::config::{self, Credentials, Paths, Settings};

#[derive(Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    command: Option<ConfigCommand>,
    /// Explain settings for this group.
    #[arg(long, global = true)]
    group: Option<String>,
    /// Print structured output without colour.
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Show a setting's effective value and source.
    Get { key: String },
    /// Set a general value, or an override for --group.
    Set { key: String, value: String },
    /// Remove a value, falling back to the next layer.
    Unset { key: String },
    /// Show the settings file's path.
    Path,
    /// Open the settings file in $EDITOR.
    Edit,
}

pub fn config(args: ConfigArgs) -> anyhow::Result<()> {
    let settings = Settings::load()?;
    match args.command {
        Some(ConfigCommand::Path) => println!("{}", settings.path.display()),
        Some(ConfigCommand::Edit) => {
            let editor =
                std::env::var("EDITOR").context("set EDITOR to edit your configuration")?;
            std::fs::create_dir_all(settings.path.parent().context("settings directory")?)?;
            let mut parts = editor.split_whitespace();
            let status = std::process::Command::new(parts.next().context("EDITOR is empty")?)
                .args(parts)
                .arg(&settings.path)
                .status()?;
            anyhow::ensure!(status.success(), "the editor exited with {status}");
        }
        Some(ConfigCommand::Set { key, value }) => {
            settings.write(&key, Some(&value), args.group.as_deref())?
        }
        Some(ConfigCommand::Unset { key }) => settings.write(&key, None, args.group.as_deref())?,
        command => {
            let values = match command {
                Some(ConfigCommand::Get { key }) => {
                    vec![settings.get(&key, args.group.as_deref())?]
                }
                _ => settings.explain(args.group.as_deref(), &Default::default())?,
            };
            if args.json {
                println!("{}", serde_json::to_string_pretty(&values)?);
            } else {
                for setting in values {
                    println!("{} = {} – {}", setting.key, setting.value, setting.source);
                }
            }
        }
    }
    Ok(())
}

#[derive(Args)]
pub struct GroupArgs {
    #[command(subcommand)]
    command: Option<GroupCommand>,
}

#[derive(Subcommand)]
enum GroupCommand {
    /// Choose the active group for commands and agent sessions.
    Use { slug: String },
    /// List your groups.
    List,
}

pub async fn group(args: GroupArgs) -> anyhow::Result<()> {
    match args.command {
        None => println!(
            "{}",
            config::active_group(None)?
                .as_deref()
                .unwrap_or("No active group")
        ),
        Some(command) => {
            let me = Credentials::load()?.with_env().hub()?.me().await?;
            match command {
                GroupCommand::Use { slug } => {
                    anyhow::ensure!(
                        me.groups.iter().any(|g| g.slug == slug),
                        "you are not in a group {slug}"
                    );
                    Paths::discover()?.use_group(&slug)?;
                    println!("{slug}");
                }
                GroupCommand::List => {
                    for group in me.groups {
                        println!("{} – {}", group.slug, group.name);
                    }
                }
            }
        }
    }
    Ok(())
}
