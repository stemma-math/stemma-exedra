//! The client commands: signing in to a hub from this machine, a few plain
//! commands for the forum, and the MCP server for agents.

use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;

use clap::Args;
use serde_json::{Value, json};
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
pub struct ThreadsArgs {
    #[command(flatten)]
    group: GroupArg,
    /// Only chats.
    #[arg(long)]
    chats: bool,
    /// How many.
    #[arg(long, short = 'n', default_value_t = 30)]
    limit: u32,
}

#[derive(Args)]
pub struct ReadArgs {
    /// What to read: t12, t12#45, t12#45-50, lean:Foo.bar, p/notes, or
    /// a link to it.
    pub address: String,
    /// For a paper, the file of its source to read; its main file if
    /// absent.
    pub file: Option<String>,
    #[command(flatten)]
    group: GroupArg,
}

#[derive(Args)]
pub struct PapersArgs {
    #[command(flatten)]
    group: GroupArg,
}

#[derive(Args)]
pub struct PaperArgs {
    #[command(subcommand)]
    pub command: PaperCommand,
    #[command(flatten)]
    group: GroupArg,
}

#[derive(clap::Subcommand)]
pub enum PaperCommand {
    New {
        /// Its short name, in its address: notes, for p/notes.
        slug: String,
        /// Its title.
        #[arg(required = true)]
        title: Vec<String>,
    },
}

#[derive(Args)]
pub struct PostArgs {
    /// Where: a thread (t12), or the message to reply to (t12#45).
    address: String,
    /// The message, in Markdown; read from standard input if absent or `-`.
    body: Vec<String>,
    #[command(flatten)]
    group: GroupArg,
    /// The agent writing it for you, such as "Claude Code".
    #[arg(long)]
    agent: Option<String>,
}

#[derive(Args)]
pub struct SearchArgs {
    /// The words to find, and operators: `author:ada`, `since:7d`,
    /// `in:chats`, `kind:message`.
    #[arg(required = true)]
    query: Vec<String>,
    #[command(flatten)]
    group: GroupArg,
    #[arg(long, short = 'n', default_value_t = 20)]
    limit: u32,
}

#[derive(Args)]
pub struct LibraryArgs {
    /// Words to search the declarations for; how the library stands if
    /// absent.
    query: Vec<String>,
    #[command(flatten)]
    group: GroupArg,
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

/// The tools, as configured on this machine.
fn toolbox(group: GroupArg) -> anyhow::Result<Toolbox> {
    let config = Credentials::load()?.with_env();
    Ok(Toolbox::new(
        config.hub()?,
        config::active_group(group.group)?,
    ))
}

async fn run(
    toolbox: &Toolbox,
    tool: &str,
    args: Value,
    agent: Option<&str>,
) -> anyhow::Result<()> {
    let text = toolbox.call(tool, args, agent).await?;
    crate::output::text(&text);
    std::io::stdout().flush()?;
    Ok(())
}

pub async fn threads(args: ThreadsArgs) -> anyhow::Result<()> {
    let toolbox = toolbox(args.group)?;
    let mut call = json!({"limit": args.limit});
    if args.chats {
        call["kind"] = json!("chat");
    }
    run(&toolbox, "list_threads", call, None).await
}

pub async fn read(args: ReadArgs) -> anyhow::Result<()> {
    let toolbox = toolbox(args.group)?;
    if let Some(file) = args.file {
        return run(
            &toolbox,
            "read_paper",
            json!({"address": args.address, "file": file}),
            None,
        )
        .await;
    }
    run(
        &toolbox,
        "read_thread",
        json!({"address": args.address}),
        None,
    )
    .await
}

pub async fn papers(args: PapersArgs) -> anyhow::Result<()> {
    let toolbox = toolbox(args.group)?;
    run(&toolbox, "list_papers", json!({}), None).await
}

pub async fn paper(args: PaperArgs) -> anyhow::Result<()> {
    let toolbox = toolbox(args.group)?;
    let group = toolbox.group(None).await?;
    match args.command {
        PaperCommand::New { slug, title } => {
            let slug = slug.strip_prefix("p/").unwrap_or(&slug);
            let paper = toolbox
                .hub()
                .create_paper(&group, slug, &title.join(" "))
                .await?;
            println!(
                "Created p/{}, “{}”: {}{}",
                paper.slug,
                paper.title,
                toolbox.hub().base(),
                stemma_core::Address::Paper(paper.slug.clone()).web_path(&group)
            );
            println!(
                "Clone it with a personal token as the password: git clone {}{}",
                toolbox.hub().base(),
                paper.clone_path
            );
            if let Some(url) = &paper.forge_url {
                println!("Or in the forge: {url}");
            }
        }
    }
    Ok(())
}

pub async fn post(args: PostArgs) -> anyhow::Result<()> {
    let toolbox = toolbox(args.group)?;
    let body = if args.body.is_empty() || args.body == ["-"] {
        if std::io::stdin().is_terminal() {
            eprintln!("Write the message, and end it with Ctrl-D.");
        }
        let mut body = String::new();
        std::io::stdin().read_to_string(&mut body)?;
        body
    } else {
        args.body.join(" ")
    };
    anyhow::ensure!(!body.trim().is_empty(), "the message is empty");
    run(
        &toolbox,
        "post_message",
        json!({"thread": args.address, "body": body}),
        args.agent.as_deref(),
    )
    .await
}

pub async fn search(args: SearchArgs) -> anyhow::Result<()> {
    let toolbox = toolbox(args.group)?;
    run(
        &toolbox,
        "search",
        json!({"query": args.query.join(" "), "limit": args.limit}),
        None,
    )
    .await
}

pub async fn library(args: LibraryArgs) -> anyhow::Result<()> {
    let toolbox = toolbox(args.group)?;
    if args.query.is_empty() {
        run(&toolbox, "library_state", json!({}), None).await
    } else {
        let query = args.query.join(" ");
        run(&toolbox, "library_search", json!({"query": query}), None).await
    }
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
