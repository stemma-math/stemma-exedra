//! The client commands: signing in to a hub from this machine, a few plain
//! commands for the forum, and the MCP server for agents.

use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;

use anyhow::Context;
use clap::Args;
use serde_json::{Value, json};
use stemma_client::config::{self, ClientConfig};
use stemma_client::hub::Hub;
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
    /// What to read: t12, t12#45, t12#45-50, lean:Foo.bar, w7, p/notes, or
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
    /// Create a paper: its repository, from the template, which the
    /// workshop then compiles.
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
pub struct TasksArgs {
    #[command(flatten)]
    group: GroupArg,
    /// Only the tasks in this state: open, running, review, done, failed,
    /// rejected or cancelled.
    #[arg(long)]
    state: Option<String>,
    #[arg(long, short = 'n', default_value_t = 30)]
    limit: u32,
}

#[derive(Args)]
pub struct ReviewArgs {
    /// The task: w7.
    task: String,
    #[command(flatten)]
    group: GroupArg,
}

/// The number of a task, as written: `w7` or `7`.
fn task_number(written: &str) -> anyhow::Result<u32> {
    written
        .trim()
        .trim_start_matches('w')
        .parse()
        .with_context(|| format!("{written} is not a task: a task is w7"))
}

/// Approves a task's changes, as what you may approve them as: their
/// asker, or an admin.
pub async fn approve(args: ReviewArgs) -> anyhow::Result<()> {
    let number = task_number(&args.task)?;
    let toolbox = toolbox(args.group)?;
    let group = toolbox.group(None).await?;
    toolbox.hub().approve_task(&group, number).await?;
    let page = toolbox.hub().task(&group, number).await?;
    match page.task.changes.map(|c| c.needs).unwrap_or_default() {
        needs if needs.is_empty() => println!("Approved: w{number} is in the library."),
        _ => println!("Approved: w{number} still waits for an admin's approval."),
    }
    Ok(())
}

/// Rejects a task's changes: their asker, or an admin.
pub async fn reject(args: ReviewArgs) -> anyhow::Result<()> {
    let number = task_number(&args.task)?;
    let toolbox = toolbox(args.group)?;
    let group = toolbox.group(None).await?;
    toolbox.hub().reject_task(&group, number).await?;
    println!("Rejected the changes of w{number}.");
    Ok(())
}

#[derive(Args)]
pub struct TaskArgs {
    /// What to do, in a line.
    title: String,
    /// The message that asks for it: t12#48, or a link to it.
    #[arg(long, short)]
    origin: String,
    /// The agent handing it over for you, such as "Claude Code".
    #[arg(long)]
    agent: Option<String>,
    /// The paper to work on, as p/notes; the library if absent.
    #[arg(long)]
    pub paper: Option<String>,
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
    /// With --read-only, still offer create_task, to hand tasks to the
    /// workshop.
    #[arg(long, requires = "read_only")]
    hand_over: bool,
    /// With --read-only, still offer open_thread, for the workshop's agent
    /// to ask the group something.
    #[arg(long, requires = "read_only")]
    open_threads: bool,
    /// The hub, instead of the one signed in to.
    #[arg(long, env = "STEMMA_HUB")]
    hub: Option<String>,
    /// Read the token from this file, instead of the one signed in with.
    #[arg(long)]
    token_file: Option<PathBuf>,
}

pub async fn login(args: LoginArgs) -> anyhow::Result<()> {
    let config = match args.token {
        Some(token) => stemma_client::sign_in::save(&args.hub, token, None, args.group).await?,
        None => {
            let username = match args.username {
                Some(username) => username,
                None => prompt("Username: ")?,
            };
            let password = rpassword::prompt_password("Password: ")?;
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
    let group = &config.group;
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
    let path = config::path()?;
    let config = ClientConfig::load_from(&path)?;
    let (Some(url), Some(token)) = (&config.hub, &config.token) else {
        println!("Not signed in.");
        return Ok(());
    };
    if let Some(id) = config.token_id {
        match Hub::new(url, Some(token.clone()))?.revoke_token(id).await {
            Ok(()) => println!("Revoked this machine's token."),
            Err(e) => eprintln!("Could not revoke the token ({e}); revoke it in the web app."),
        }
    }
    std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    println!("Signed out of {url}.");
    Ok(())
}

/// The tools, as configured on this machine.
fn toolbox(group: GroupArg) -> anyhow::Result<Toolbox> {
    let config = ClientConfig::load()?.with_env();
    Ok(Toolbox::new(config.hub()?, group.group.or(config.group)))
}

async fn run(
    toolbox: &Toolbox,
    tool: &str,
    args: Value,
    agent: Option<&str>,
) -> anyhow::Result<()> {
    let text = toolbox.call(tool, args, agent).await?;
    print!("{text}");
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

pub async fn tasks(args: TasksArgs) -> anyhow::Result<()> {
    let toolbox = toolbox(args.group)?;
    let mut call = json!({"limit": args.limit});
    if let Some(state) = args.state {
        call["state"] = json!(state);
    }
    run(&toolbox, "list_tasks", call, None).await
}

pub async fn task(args: TaskArgs) -> anyhow::Result<()> {
    let toolbox = toolbox(args.group)?;
    if std::io::stdin().is_terminal() {
        eprintln!("Write what the workshop is to do, and end it with Ctrl-D.");
    }
    let mut instructions = String::new();
    std::io::stdin().read_to_string(&mut instructions)?;
    anyhow::ensure!(
        !instructions.trim().is_empty(),
        "the instructions are empty"
    );
    let mut call = json!({
        "title": args.title,
        "instructions": instructions,
        "origin": args.origin,
    });
    if let Some(paper) = args.paper {
        call["paper"] = json!(paper);
    }
    run(&toolbox, "create_task", call, args.agent.as_deref()).await
}

pub async fn mcp(args: McpArgs) -> anyhow::Result<()> {
    let mut config = ClientConfig::load()?.with_env();
    if let Some(hub) = args.hub {
        config.hub = Some(hub);
    }
    if let Some(path) = &args.token_file {
        config.token = Some(config::read_token_file(path)?);
    }
    let toolbox = Toolbox::new(config.hub()?, args.group.group.or(config.group))
        .read_only(args.read_only)
        .hands_over(args.hand_over)
        .opens_threads(args.open_threads);
    StemmaServer::new(toolbox, args.agent).serve_stdio().await
}

fn prompt(question: &str) -> anyhow::Result<String> {
    eprint!("{question}");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_owned())
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
