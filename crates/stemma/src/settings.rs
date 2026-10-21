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
    /// List every key, the containers' too when there is no isolation.
    #[arg(long)]
    all: bool,
}

/// Whether a listing shows `key`: the containers' keys only with isolation,
/// unless all are asked for.
fn listed(key: &str, isolated: bool, all: bool) -> bool {
    all || isolated || !(key.contains(".container.") || key == "agents.image")
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
    /// Keep the value of a variable your agents' containers are given
    /// (agents.<runner>.container.env), among your credentials.
    #[command(subcommand)]
    Secret(SecretCommand),
}

#[derive(Subcommand)]
enum SecretCommand {
    /// Keep a variable's value, asked for, or read from standard input.
    Set { name: String },
    /// Forget a variable's value.
    Unset { name: String },
    /// List the variables whose values are kept, without them.
    List,
}

fn secret(command: SecretCommand) -> anyhow::Result<()> {
    use std::io::{IsTerminal, Read};
    let mut credentials = Credentials::load_from(&config::path()?)?;
    match command {
        SecretCommand::Set { name } => {
            anyhow::ensure!(
                !name.is_empty()
                    && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                    && !name.to_uppercase().starts_with("STEMMA_"),
                "{name} cannot be a container's variable"
            );
            let value = if std::io::stdin().is_terminal() {
                cliclack::password(format!("Value of {name}")).interact()?
            } else {
                let mut value = String::new();
                std::io::stdin().read_to_string(&mut value)?;
                value.trim().to_owned()
            };
            anyhow::ensure!(!value.is_empty(), "no value for {name}");
            credentials.secrets.insert(name, value);
        }
        SecretCommand::Unset { name } => {
            credentials.secrets.remove(&name);
        }
        SecretCommand::List => {
            for name in credentials.secrets.keys() {
                println!("{name}");
            }
            return Ok(());
        }
    }
    credentials.save()
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
        Some(ConfigCommand::Secret(command)) => secret(command)?,
        command => {
            let values = match command {
                Some(ConfigCommand::Get { key }) => {
                    vec![settings.get(&key, args.group.as_deref())?]
                }
                _ => {
                    let values = settings.explain(args.group.as_deref(), &Default::default())?;
                    let isolated = values
                        .iter()
                        .any(|v| v.key == "agents.isolation" && v.value.as_str() != Some("none"));
                    values
                        .into_iter()
                        .filter(|v| listed(&v.key, isolated, args.all))
                        .collect()
                }
            };
            if args.json {
                println!("{}", serde_json::to_string_pretty(&values)?);
            } else {
                for setting in values {
                    let out = crate::output::stdout();
                    println!(
                        "{} = {} – {}",
                        out.paint(stemma_design_terminal::Role::Identifier, &setting.key),
                        setting.value,
                        out.paint(
                            stemma_design_terminal::Role::Muted,
                            &setting.source.to_string()
                        )
                    );
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

#[cfg(test)]
mod tests {
    use super::listed;

    #[test]
    fn containers_keys_are_listed_with_isolation_or_all() {
        assert!(listed("agents.claude.model", false, false));
        assert!(!listed("agents.claude.container.env", false, false));
        assert!(!listed("agents.image", false, false));
        assert!(listed("agents.claude.container.env", true, false));
        assert!(listed("agents.image", false, true));
    }
}
