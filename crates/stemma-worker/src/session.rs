//! A person's private conversation with one of their agents: what
//! `stemma claude`, `stemma codex` and `stemma opencode` open.
//!
//! The agent runs interactively, with its unmodified program and the
//! person's own sign-in, launched as [`crate::launch`] says: Stemma makes a
//! token for the session, which lasts at most [`SESSION_MINUTES`] and is
//! revoked when the agent exits; writes, in a private folder of its own,
//! that token and what the agent reads its tools from (`stemma mcp`, with
//! that token, in the group); and gives the agent Stemma's instructions,
//! in `prompts/session.md`. Nothing is installed into the agent's own
//! configuration, and the folder goes when the session ends.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use stemma_client::Hub;
use stemma_core::api::{Group, PersonalAgent, User, runners};

use crate::launch::{self, Launch, Mcp, Mode, Prepared};

/// Stemma's instructions to an agent in a person's session.
pub const SESSION_INSTRUCTIONS: &str = include_str!("../prompts/session.md");

/// The most a session's token lasts, in minutes: a long day.
pub const SESSION_MINUTES: u32 = 16 * 60;

/// What to open.
#[derive(Clone, Debug)]
pub struct Options {
    /// One of [`runners::ALL`].
    pub runner: String,
    /// The handle of one of the person's agents with that runner; their
    /// default one for it if absent.
    pub agent: Option<String>,
    /// The group; the only one they are in if absent.
    pub group: Option<String>,
    /// More arguments for the agent, after its own.
    pub args: Vec<String>,
    /// The `stemma` binary, for the agent's MCP server.
    pub stemma: PathBuf,
    /// The agent's program, if not its usual one.
    pub program: Option<String>,
    /// Where the session's folder goes; the system's temporary folder if
    /// absent.
    pub temp_dir: Option<PathBuf>,
    /// The name of the machine, for the token's.
    pub host: String,
}

/// A session, open: its agent ready to run.
pub struct Session {
    pub prepared: Prepared,
    pub agent: PersonalAgent,
    pub person: User,
    pub group: Group,
    /// The session's private folder, with its token and files.
    pub dir: PathBuf,
    token_id: i64,
}

impl Session {
    /// Opens a session for the person `hub` acts as: picks their agent and
    /// the group, makes the session's token and writes its files.
    pub async fn open(hub: &Hub, options: Options) -> anyhow::Result<Session> {
        let me = hub.me().await.context("reaching the hub")?;
        let group = match &options.group {
            Some(slug) => me
                .groups
                .iter()
                .find(|g| &g.slug == slug)
                .with_context(|| format!("you are not in a group {slug}"))?,
            None => match me.groups.as_slice() {
                [only] => only,
                [] => anyhow::bail!("you are in no group yet"),
                _ => anyhow::bail!("you are in several groups: name one with --group"),
            },
        }
        .clone();
        let agent = pick(hub, &options).await?;

        let token = hub
            .create_expiring_token(
                &format!(
                    "{}, a session on {}",
                    runners::name(&options.runner),
                    options.host
                ),
                SESSION_MINUTES,
            )
            .await
            .context("making the session's token")?;
        let token_id = token.token.id;
        let opened = prepare(
            &options,
            hub.base(),
            &me.user,
            &group,
            &agent,
            &token.secret,
        );
        match opened {
            Ok((dir, prepared)) => Ok(Session {
                prepared,
                agent,
                person: me.user,
                group,
                dir,
                token_id,
            }),
            Err(error) => {
                let _ = hub.revoke_token(token_id).await;
                Err(error)
            }
        }
    }

    /// Ends the session: its token is revoked and its folder removed.
    pub async fn close(self, hub: &Hub) -> anyhow::Result<()> {
        let _ = std::fs::remove_dir_all(&self.dir);
        hub.revoke_token(self.token_id)
            .await
            .context("revoking the session's token; it expires by itself")?;
        Ok(())
    }
}

/// The person's agent for the session.
async fn pick(hub: &Hub, options: &Options) -> anyhow::Result<PersonalAgent> {
    let runner = runners::name(&options.runner);
    let handle = match &options.agent {
        Some(handle) => handle.trim().trim_start_matches('@').to_lowercase(),
        None => {
            PersonalAgent::default_of_runner(&options.runner)
                .with_context(|| format!("there is no runner {}", options.runner))?
                .handle
        }
    };
    let agents = hub.my_agents().await.context("reading your agents")?;
    let agent = agents
        .into_iter()
        .find(|a| a.handle == handle)
        .with_context(|| format!("you have no agent @{handle}: add it in your settings"))?;
    anyhow::ensure!(
        agent.runner == options.runner,
        "@{handle} runs {}, not {runner}",
        runners::name(&agent.runner)
    );
    Ok(agent)
}

/// Writes the session's folder and prepares the agent's launch.
fn prepare(
    options: &Options,
    hub: &str,
    person: &User,
    group: &Group,
    agent: &PersonalAgent,
    token: &str,
) -> anyhow::Result<(PathBuf, Prepared)> {
    let dir = private_dir(options.temp_dir.as_deref())?;
    let prepared = (|| {
        let token_file = dir.join("token");
        write_private(&token_file, token.as_bytes())?;
        let instructions = SESSION_INSTRUCTIONS
            .replace("{agent}", &agent.display_name)
            .replace("{person}", &person.display_name)
            .replace("{group}", &group.name);
        let mcp = Mcp {
            command: options.stemma.clone(),
            args: vec![
                "mcp".into(),
                "--hub".into(),
                hub.to_owned(),
                "--group".into(),
                group.slug.clone(),
                "--token-file".into(),
                token_file.display().to_string(),
                "--agent".into(),
                agent.display_name.clone(),
            ],
        };
        let mut args = agent.args.clone();
        args.extend(options.args.iter().cloned());
        launch::prepare(&Launch {
            runner: &options.runner,
            program: options.program.as_deref(),
            model: agent.model.as_deref(),
            args: &args,
            instructions: &instructions,
            mcp: &mcp,
            files: &dir,
            mode: Mode::Session {
                allowed: stemma_client::tools::read_only_names()
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            },
        })
    })();
    match prepared {
        Ok(prepared) => Ok((dir, prepared)),
        Err(error) => {
            let _ = std::fs::remove_dir_all(&dir);
            Err(error)
        }
    }
}

/// A new folder that only its owner may enter.
fn private_dir(parent: Option<&Path>) -> anyhow::Result<PathBuf> {
    let parent = parent.map_or_else(std::env::temp_dir, Path::to_owned);
    let dir = parent.join(format!("stemma-session-{:016x}", rand::random::<u64>()));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(&dir)
        .with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

fn write_private(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(path)
        .with_context(|| format!("writing {}", path.display()))?;
    std::io::Write::write_all(&mut file, contents)?;
    Ok(())
}
