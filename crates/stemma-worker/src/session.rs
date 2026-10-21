//! A person's private conversation with one of their agents: what
//! `stemma claude`, `stemma codex` and `stemma opencode` open.
//!
//! The agent runs interactively, as the person and with everything their
//! agent normally has, in the folder they are in (or, asked to, in the
//! group's workspace), launched as [`crate::launch`] says. Stemma makes an
//! agent's token for the session (one group; reading it, and posting and
//! opening threads there as the agent), revoked when the agent exits;
//! writes, in a private folder of its own, that token and what the agent
//! reads its tools from (`stemma mcp`, with that token); and gives the
//! agent Stemma's instructions, in `prompts/session.md`.
//!
//! The agent is kept off the person's own sign-in: it runs with
//! `STEMMA_CONFIG_DIR` at the session's folder, which has no credentials,
//! and without `STEMMA_TOKEN` or `STEMMA_HUB`, so the `stemma` command
//! there is signed out; its instructions say to use Stemma's tools only;
//! and Claude Code is also told not to run `stemma`. An agent that insists
//! can still read the person's files, since it runs as them. Nothing is
//! installed into the agent's own configuration, and the folder goes when
//! the session ends.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use stemma_client::Hub;
use stemma_core::api::{AgentProfile, Group, User};

use crate::launch::{self, Launch, Mcp, Mode, Prepared};

/// Stemma's instructions to an agent in a person's session.
pub const SESSION_INSTRUCTIONS: &str = include_str!("../prompts/session.md");

/// What a session in the group's workspace is also told.
const WORKSPACE_INSTRUCTIONS: &str = "
You work in the group's workspace, which Stemma manages: `library/` and
`papers/<slug>/` are clones of the group's repositories at `main`, brought
up to date when a session or an answer starts, and anything else here may
go. Check your reasoning there (`lake build`, `latexmk`), and keep
{person}'s lasting work elsewhere.
";

/// What to open.
#[derive(Clone, Debug)]
pub struct Options {
    /// One of [`stemma_core::api::runners::ALL`].
    pub runner: String,
    /// The group; the only one they are in if absent.
    pub group: Option<String>,
    /// Whether to work in the group's workspace, brought up to date, rather
    /// than where the person is.
    pub workspace: bool,
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
    pub agent: AgentProfile,
    pub person: User,
    pub group: Group,
    /// The session's private folder, with its token and files.
    pub dir: PathBuf,
    token_id: i64,
    /// The group's workspace, when the session works there.
    pub workspace: Option<crate::workspace::Workspace>,
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
        let agent = AgentProfile::default_of_runner(&options.runner).context("unknown runner")?;
        let workspace = if options.workspace {
            let settings = stemma_client::config::Settings::load()?;
            let paths = stemma_client::config::Paths::discover()?;
            let credentials = stemma_client::config::Credentials::load()?.with_env();
            Some(
                crate::workspace::Workspace::open(
                    &settings,
                    &paths,
                    hub,
                    &group.slug,
                    credentials.token.as_deref().context("signed out")?,
                )
                .await?,
            )
        } else {
            None
        };
        let token = hub
            .create_agent_token(&group.slug, &options.runner, &options.host)
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
                workspace,
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

/// Writes the session's folder and prepares the agent's launch.
fn prepare(
    options: &Options,
    hub: &str,
    person: &User,
    group: &Group,
    agent: &AgentProfile,
    token: &str,
) -> anyhow::Result<(PathBuf, Prepared)> {
    let dir = private_dir(options.temp_dir.as_deref())?;
    let prepared = (|| {
        let token_file = dir.join("token");
        write_private(&token_file, token.as_bytes())?;
        let workspace = if options.workspace {
            WORKSPACE_INSTRUCTIONS
        } else {
            ""
        };
        let instructions = SESSION_INSTRUCTIONS
            .replace("{workspace}", workspace)
            .replace("{agent}", &agent.display_name)
            .replace("{person}", &person.display_name)
            .replace("{group}", &group.name)
            .trim_end()
            .to_owned();
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
        let config = crate::WorkerConfig::load(Some(&group.slug))?;
        let runner = config
            .runners
            .get(&options.runner)
            .context("unknown runner")?;
        let mut args = runner.args.clone();
        args.extend(options.args.iter().cloned());
        let mut prepared = launch::prepare(&Launch {
            runner: &options.runner,
            program: options.program.as_deref().or(runner.command.as_deref()),
            model: runner.model.as_deref(),
            args: &args,
            instructions: &instructions,
            mcp: &mcp,
            files: &dir,
            files_seen: &dir,
            mode: Mode::Session {
                allowed: stemma_client::tools::read_only_names()
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            },
        })?;
        // The `stemma` command, run by the agent, finds no sign-in here.
        prepared
            .env
            .push(("STEMMA_CONFIG_DIR".into(), dir.display().to_string()));
        Ok(prepared)
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
    let parent = parent
        .map(Path::to_owned)
        .unwrap_or(stemma_client::config::Paths::discover()?.state.join("jobs"));
    std::fs::create_dir_all(&parent)?;
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
