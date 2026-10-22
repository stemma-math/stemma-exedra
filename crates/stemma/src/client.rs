//! The client commands: signing in to a hub from this machine, a few plain
//! commands for the forum, and the MCP server for agents.

use std::path::PathBuf;

use clap::Args;
use stemma_client::config::{self, Credentials};
use stemma_client::mcp::StemmaServer;
use stemma_client::tools::Toolbox;

#[derive(Args)]
pub struct LoginArgs {
    /// The hub's address, such as https://stemma.example.org.
    hub: String,
    /// Use this personal token (from the web app's settings) instead of
    /// signing in with a password.
    #[arg(long, env = "STEMMA_TOKEN", hide_env_values = true)]
    token: Option<String>,
    /// The username to sign in with; asked if absent.
    #[arg(long)]
    username: Option<String>,
    /// The group to use when none is named.
    #[arg(long)]
    group: Option<String>,
}

/// Which group a command is about.
#[derive(Args)]
pub struct GroupArg {
    /// The group's short name; the one chosen at login if absent.
    #[arg(long, short, env = "STEMMA_GROUP")]
    group: Option<String>,
}

#[derive(Args)]
pub struct McpArgs {
    #[command(flatten)]
    group: GroupArg,
    /// The agent posts are marked as written by; else the MCP client's name.
    #[arg(long)]
    agent: Option<String>,
    /// Offer only the tools that read.
    #[arg(long)]
    read_only: bool,
    /// The hub, instead of the one signed in to.
    #[arg(long, env = "STEMMA_HUB")]
    hub: Option<String>,
    /// Read the token from this file, instead of the one signed in with.
    #[arg(long)]
    token_file: Option<PathBuf>,
    /// Where downloaded attachments are saved.
    #[arg(long)]
    attachment_dir: Option<PathBuf>,
}

pub async fn login(args: LoginArgs) -> anyhow::Result<()> {
    let config = match args.token {
        Some(token) => stemma_client::sign_in::save(&args.hub, token, None, args.group).await?,
        None => {
            let username = match args.username {
                Some(username) => username,
                None => cliclack::input("Username").interact()?,
            };
            let password: String = cliclack::password("Password").interact()?;
            stemma_client::sign_in::password(
                &args.hub,
                &username,
                &password,
                &hostname(),
                args.group,
            )
            .await?
        }
    };
    let hub = config.hub()?;
    let me = hub.me().await?;
    let url = hub.base();
    if config::active_group(None)?.is_none()
        && let Some(group) = me.groups.first()
    {
        config::Paths::discover()?.use_group(&group.slug)?;
    }
    let group = config::active_group(None)?;
    println!("Signed in to {url} as {}.", me.user.username);
    match (&group, me.groups.len()) {
        (Some(group), 1) => println!("Your group is {group}."),
        (Some(group), _) => println!(
            "Commands use the group {group} unless told otherwise (--group); you are in {}.",
            me.groups
                .iter()
                .map(|g| g.slug.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        (None, _) => println!("You are in no group yet."),
    }
    println!("Settings saved in {}.", config::path()?.display());
    Ok(())
}

pub async fn logout() -> anyhow::Result<()> {
    if let Some(warning) = stemma_client::sign_in::logout().await? {
        eprintln!("{warning}");
    }
    println!("Signed out.");
    Ok(())
}

pub async fn mcp(args: McpArgs) -> anyhow::Result<()> {
    let mut config = Credentials::load()?.with_env();
    if let Some(hub) = args.hub {
        config.hub = Some(hub);
    }
    if let Some(path) = &args.token_file {
        config.token = Some(config::read_token_file(path)?);
    }
    let mut toolbox = Toolbox::new(config.hub()?, config::active_group(args.group.group)?)
        .read_only(args.read_only);
    if let Some(dir) = args.attachment_dir.or_else(|| {
        args.token_file
            .and_then(|p| p.parent().map(|p| p.join("attachments")))
    }) {
        toolbox = toolbox.attachment_dir(dir);
    }
    StemmaServer::new(toolbox, args.agent).serve_stdio().await
}

pub fn hostname() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
        })
        .map(|h| h.trim().to_owned())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "this machine".into())
}
