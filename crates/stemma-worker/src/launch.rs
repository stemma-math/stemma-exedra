//! How Stemma launches each agent: the one place that knows the flags,
//! files and environment of Claude Code, Codex and OpenCode.
//!
//! An agent is launched with its unmodified program and the person's own
//! sign-in, and nothing is installed into its own configuration: what it
//! needs from Stemma, its instructions and Stemma's tools (`stemma mcp`,
//! with a token of its own), goes in its command line, in files of a
//! folder of its own and in its environment, which last as long as it
//! runs.
//!
//! It is launched in one of two ways:
//!
//! - [`Mode::Session`], interactive: the person's private conversation
//!   with their agent, which `stemma claude`, `stemma codex` and
//!   `stemma opencode` open.
//! - [`Mode::Job`], headless: a job of a machine or a worker, which
//!   answers in a thread or works on a task of the workshop.
//!
//! For each runner:
//!
//! - Claude Code reads Stemma's tools from an MCP configuration
//!   (`--mcp-config`) and its instructions from `--append-system-prompt`.
//!   A job runs it with `-p`, only those tools (`--strict-mcp-config`),
//!   allowed without asking.
//! - Codex gets Stemma's tools and instructions as configuration values on
//!   its command line (`-c mcp_servers.stemma.…`,
//!   `-c developer_instructions=…`). A job runs `codex exec` in a sandbox.
//! - OpenCode reads a configuration of its own from `OPENCODE_CONFIG`,
//!   which it merges with the person's: Stemma's tools, and its
//!   instructions in a file. A job runs `opencode run`, with what it may
//!   do in that configuration.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde_json::json;
use stemma_core::api::runners;

/// Stemma's MCP server, as an agent starts it: `stemma mcp …`.
#[derive(Clone, Debug)]
pub struct Mcp {
    pub command: PathBuf,
    pub args: Vec<String>,
}

/// How an agent is launched.
#[derive(Clone, Debug)]
pub enum Mode<'a> {
    /// Interactive, for the person: the Stemma tools named are allowed
    /// without asking, where the runner lets one say so.
    Session { allowed: Vec<String> },
    /// Headless, for a job.
    Job {
        /// The folder it works in, as it sees it.
        dir: &'a Path,
        /// Whether it edits files and runs commands there, as for a task.
        edits: bool,
        /// Whether it runs in a container of its own, which is then its
        /// sandbox, rather than the runner's own.
        contained: bool,
        /// The most steps it may take, for runners that count them.
        max_turns: Option<u32>,
        /// The Stemma tools it may use, by name.
        allowed: Vec<String>,
    },
}

/// What launching an agent takes.
#[derive(Clone, Debug)]
pub struct Launch<'a> {
    /// One of [`runners::ALL`].
    pub runner: &'a str,
    /// The program to run; the runner's usual one if absent.
    pub program: Option<&'a str>,
    /// The model, if not the runner's default.
    pub model: Option<&'a str>,
    /// More arguments, last: the worker's for the runner, the agent's, and
    /// the person's own.
    pub args: &'a [String],
    /// Stemma's instructions for it.
    pub instructions: &'a str,
    pub mcp: &'a Mcp,
    /// A folder of its own, private, where its files are written.
    pub files: &'a Path,
    /// Where the agent sees that folder: the same, unless it runs in a
    /// container.
    pub files_seen: &'a Path,
    pub mode: Mode<'a>,
}

/// A launch, ready: the program, its arguments and environment, and the
/// files written for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// The files written, in the launch's folder.
    pub files: Vec<PathBuf>,
    /// For a job of Codex, where it writes its last message.
    pub answer: Option<PathBuf>,
}

impl Prepared {
    /// The command, to run.
    pub fn command(&self) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(&self.program);
        command.args(&self.args);
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
    }
}

/// Writes what an agent needs and says how to run it.
pub fn prepare(launch: &Launch) -> anyhow::Result<Prepared> {
    let program = match launch.program {
        Some(program) => program.to_owned(),
        None => runners::program(launch.runner)
            .with_context(|| format!("there is no runner {}", launch.runner))?
            .to_owned(),
    };
    let mut prepared = Prepared {
        program,
        args: Vec::new(),
        env: Vec::new(),
        files: Vec::new(),
        answer: None,
    };
    match launch.runner {
        runners::CLAUDE_CODE => claude_code(launch, &mut prepared)?,
        runners::CODEX => codex(launch, &mut prepared),
        runners::OPENCODE => opencode(launch, &mut prepared)?,
        other => anyhow::bail!("there is no runner {other}"),
    }
    Ok(prepared)
}

/// Writes a file of the launch, readable only by its owner.
fn write(prepared: &mut Prepared, path: PathBuf, contents: &[u8]) -> anyhow::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(&path)
        .with_context(|| format!("writing {}", path.display()))?;
    std::io::Write::write_all(&mut file, contents)?;
    prepared.files.push(path);
    Ok(())
}

fn push(args: &mut Vec<String>, items: &[&str]) {
    args.extend(items.iter().map(|item| (*item).to_owned()));
}

/// Claude Code's own tools a task's run may use.
const CLAUDE_WORK_TOOLS: &[&str] = &["Bash", "Read", "Edit", "MultiEdit", "Write", "Glob", "Grep"];

fn claude_code(launch: &Launch, prepared: &mut Prepared) -> anyhow::Result<()> {
    let mcp = json!({
        "mcpServers": {
            "stemma": {
                "type": "stdio",
                "command": launch.mcp.command,
                "args": launch.mcp.args,
            }
        }
    });
    write(
        prepared,
        launch.files.join("mcp.json"),
        &serde_json::to_vec_pretty(&mcp)?,
    )?;
    let mcp_file = launch.files_seen.join("mcp.json").display().to_string();
    let stemma_tools = |allowed: &[String]| -> Vec<String> {
        allowed
            .iter()
            .map(|name| format!("mcp__stemma__{name}"))
            .collect()
    };
    let args = &mut prepared.args;
    match &launch.mode {
        Mode::Session { allowed } => {
            // `--mcp-config` and `--allowedTools` take several values: a
            // flag with one ends them, before anything the person adds.
            push(args, &["--mcp-config", &mcp_file]);
            if !allowed.is_empty() {
                push(args, &["--allowedTools", &stemma_tools(allowed).join(",")]);
            }
            push(args, &["--append-system-prompt", launch.instructions]);
        }
        Mode::Job {
            edits,
            max_turns,
            allowed,
            ..
        } => {
            push(args, &["-p", "--output-format", "stream-json", "--verbose"]);
            push(args, &["--no-session-persistence", "--strict-mcp-config"]);
            push(args, &["--mcp-config", &mcp_file]);
            let mut tools = stemma_tools(allowed);
            if *edits {
                tools.extend(CLAUDE_WORK_TOOLS.iter().map(|t| (*t).to_owned()));
            } else {
                push(args, &["--tools", ""]);
            }
            push(args, &["--allowedTools", &tools.join(",")]);
            push(args, &["--permission-mode", "dontAsk"]);
            push(args, &["--append-system-prompt", launch.instructions]);
            if let Some(turns) = max_turns {
                push(args, &["--max-turns", &turns.to_string()]);
            }
        }
    }
    if let Some(model) = launch.model {
        push(args, &["--model", model]);
    }
    args.extend(launch.args.iter().cloned());
    Ok(())
}

/// A TOML value, as `codex -c key=value` reads it.
fn toml_value(value: toml::Value) -> String {
    value.to_string()
}

fn codex(launch: &Launch, prepared: &mut Prepared) {
    let command = toml_value(toml::Value::String(
        launch.mcp.command.display().to_string(),
    ));
    let mcp_args = toml_value(toml::Value::Array(
        launch
            .mcp
            .args
            .iter()
            .cloned()
            .map(toml::Value::String)
            .collect(),
    ));
    let instructions = toml_value(toml::Value::String(launch.instructions.to_owned()));
    let configuration = [
        format!("mcp_servers.stemma.command={command}"),
        format!("mcp_servers.stemma.args={mcp_args}"),
        format!("developer_instructions={instructions}"),
    ];
    let mut answer = None;
    let args = &mut prepared.args;
    if let Mode::Job {
        dir,
        edits,
        contained,
        ..
    } = &launch.mode
    {
        // In a container of its own, the container is its sandbox: Codex's
        // own needs what a container does not allow.
        let sandbox = match (*edits, *contained) {
            (true, true) => "danger-full-access",
            (true, false) => "workspace-write",
            (false, _) => "read-only",
        };
        let file = launch.files_seen.join("answer.md");
        push(
            args,
            &["exec", "--skip-git-repo-check", "--ephemeral", "--json"],
        );
        push(args, &["--sandbox", sandbox, "--color", "never"]);
        push(args, &["--cd", &dir.display().to_string()]);
        push(
            args,
            &["--output-last-message", &file.display().to_string()],
        );
        push(args, &["-c", "approval_policy=\"never\""]);
        answer = Some(launch.files.join("answer.md"));
    }
    for value in configuration {
        push(args, &["-c", &value]);
    }
    if let Some(model) = launch.model {
        push(args, &["--model", model]);
    }
    args.extend(launch.args.iter().cloned());
    if matches!(launch.mode, Mode::Job { .. }) {
        // The prompt comes on standard input.
        args.push("-".into());
    }
    prepared.answer = answer;
}

fn opencode(launch: &Launch, prepared: &mut Prepared) -> anyhow::Result<()> {
    write(
        prepared,
        launch.files.join("instructions.md"),
        launch.instructions.as_bytes(),
    )?;
    let instructions = launch.files_seen.join("instructions.md");
    let mut command = vec![launch.mcp.command.display().to_string()];
    command.extend(launch.mcp.args.iter().cloned());
    let mut config = json!({
        "$schema": "https://opencode.ai/config.json",
        "mcp": {
            "stemma": {"type": "local", "command": command, "enabled": true}
        },
        "instructions": [instructions.display().to_string()],
    });
    if let Mode::Job { edits, .. } = &launch.mode {
        let allow = if *edits { "allow" } else { "deny" };
        config["autoupdate"] = json!(false);
        config["share"] = json!("disabled");
        config["permission"] = json!({"edit": allow, "bash": allow, "webfetch": "deny"});
        if !edits {
            config["tools"] = json!({"write": false, "edit": false, "patch": false, "bash": false});
        }
    }
    write(
        prepared,
        launch.files.join("opencode.json"),
        &serde_json::to_vec_pretty(&config)?,
    )?;
    let config_file = launch.files_seen.join("opencode.json");
    prepared
        .env
        .push(("OPENCODE_CONFIG".into(), config_file.display().to_string()));
    let args = &mut prepared.args;
    if matches!(launch.mode, Mode::Job { .. }) {
        // The prompt comes last, as the message.
        push(args, &["run", "--format", "json"]);
    }
    if let Some(model) = launch.model {
        push(args, &["--model", model]);
    }
    args.extend(launch.args.iter().cloned());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_json(path: &Path) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }

    fn mcp() -> Mcp {
        Mcp {
            command: "/bin/stemma".into(),
            args: vec!["mcp".into(), "--token-file".into(), "/t/token".into()],
        }
    }

    fn launch<'a>(runner: &'a str, files: &'a Path, mcp: &'a Mcp, mode: Mode<'a>) -> Launch<'a> {
        Launch {
            runner,
            program: None,
            model: Some("sonnet"),
            args: &[],
            instructions: "Be precise.",
            mcp,
            files,
            files_seen: files,
            mode,
        }
    }

    #[test]
    fn claude_code_in_a_session() {
        let files = tempfile::tempdir().unwrap();
        let mcp = mcp();
        let args = vec!["--effort".to_owned(), "high".to_owned()];
        let prepared = prepare(&Launch {
            args: &args,
            ..launch(
                runners::CLAUDE_CODE,
                files.path(),
                &mcp,
                Mode::Session {
                    allowed: vec!["read_thread".into(), "search".into()],
                },
            )
        })
        .unwrap();
        assert_eq!(prepared.program, "claude");
        let mcp_file = files.path().join("mcp.json").display().to_string();
        assert_eq!(
            prepared.args,
            [
                "--mcp-config",
                &mcp_file,
                "--allowedTools",
                "mcp__stemma__read_thread,mcp__stemma__search",
                "--append-system-prompt",
                "Be precise.",
                "--model",
                "sonnet",
                "--effort",
                "high"
            ]
        );
        assert!(!prepared.args.contains(&"-p".to_owned()));
        let config = read_json(&files.path().join("mcp.json")).unwrap();
        assert_eq!(config["mcpServers"]["stemma"]["command"], "/bin/stemma");
        assert_eq!(config["mcpServers"]["stemma"]["args"][2], "/t/token");
    }

    #[test]
    fn claude_code_for_a_job() {
        let files = tempfile::tempdir().unwrap();
        let mcp = mcp();
        let mode = Mode::Job {
            dir: files.path(),
            edits: false,
            contained: false,
            max_turns: None,
            allowed: vec!["read_thread".into()],
        };
        let prepared = prepare(&launch(runners::CLAUDE_CODE, files.path(), &mcp, mode)).unwrap();
        for flag in ["-p", "--strict-mcp-config", "dontAsk", "--tools"] {
            assert!(prepared.args.contains(&flag.to_owned()), "{flag}");
        }
    }

    #[test]
    fn codex_in_a_session_and_for_a_job() {
        let files = tempfile::tempdir().unwrap();
        let mcp = mcp();
        let prepared = prepare(&launch(
            runners::CODEX,
            files.path(),
            &mcp,
            Mode::Session { allowed: vec![] },
        ))
        .unwrap();
        assert_eq!(prepared.program, "codex");
        assert_eq!(
            prepared.args,
            [
                "-c",
                "mcp_servers.stemma.command=\"/bin/stemma\"",
                "-c",
                "mcp_servers.stemma.args=[\"mcp\", \"--token-file\", \"/t/token\"]",
                "-c",
                "developer_instructions=\"Be precise.\"",
                "--model",
                "sonnet"
            ]
        );
        assert!(prepared.files.is_empty());

        let mode = Mode::Job {
            dir: files.path(),
            edits: true,
            contained: false,
            max_turns: None,
            allowed: vec![],
        };
        let prepared = prepare(&launch(runners::CODEX, files.path(), &mcp, mode)).unwrap();
        assert_eq!(prepared.args[0], "exec");
        assert!(prepared.args.contains(&"workspace-write".to_owned()));
        assert_eq!(prepared.args.last().unwrap(), "-");
        assert_eq!(prepared.answer, Some(files.path().join("answer.md")));

        // In a container, the files are where it sees them, and the
        // container is its sandbox.
        let seen = Path::new("/stemma/jobs/7");
        let mode = Mode::Job {
            dir: Path::new("/stemma/tasks/w3"),
            edits: true,
            contained: true,
            max_turns: None,
            allowed: vec![],
        };
        let prepared = prepare(&Launch {
            files_seen: seen,
            ..launch(runners::CODEX, files.path(), &mcp, mode)
        })
        .unwrap();
        assert!(prepared.args.contains(&"danger-full-access".to_owned()));
        assert!(
            prepared
                .args
                .contains(&"/stemma/jobs/7/answer.md".to_owned())
        );
        assert!(prepared.args.contains(&"/stemma/tasks/w3".to_owned()));
        assert_eq!(prepared.answer, Some(files.path().join("answer.md")));
    }

    #[test]
    fn opencode_in_a_container_reads_its_files_where_it_sees_them() {
        let files = tempfile::tempdir().unwrap();
        let mcp = mcp();
        let mode = Mode::Job {
            dir: Path::new("/stemma/tasks/w3"),
            edits: true,
            contained: true,
            max_turns: None,
            allowed: vec![],
        };
        let prepared = prepare(&Launch {
            files_seen: Path::new("/stemma/jobs/7"),
            ..launch(runners::OPENCODE, files.path(), &mcp, mode)
        })
        .unwrap();
        assert_eq!(
            prepared.env,
            [(
                "OPENCODE_CONFIG".to_owned(),
                "/stemma/jobs/7/opencode.json".to_owned()
            )]
        );
        let config = read_json(&files.path().join("opencode.json")).unwrap();
        assert_eq!(config["instructions"][0], "/stemma/jobs/7/instructions.md");
        assert!(files.path().join("instructions.md").is_file());
    }

    #[test]
    fn opencode_in_a_session() {
        let files = tempfile::tempdir().unwrap();
        let mcp = mcp();
        let prepared = prepare(&launch(
            runners::OPENCODE,
            files.path(),
            &mcp,
            Mode::Session { allowed: vec![] },
        ))
        .unwrap();
        assert_eq!(prepared.program, "opencode");
        assert_eq!(prepared.args, ["--model", "sonnet"]);
        let config_file = files.path().join("opencode.json");
        assert_eq!(
            prepared.env,
            [(
                "OPENCODE_CONFIG".to_owned(),
                config_file.display().to_string()
            )]
        );
        let config = read_json(&config_file).unwrap();
        assert_eq!(
            config["mcp"]["stemma"]["command"],
            json!(["/bin/stemma", "mcp", "--token-file", "/t/token"])
        );
        // The person's own permissions stay theirs.
        assert!(config.get("permission").is_none());
        let instructions = config["instructions"][0].as_str().unwrap();
        assert_eq!(
            std::fs::read_to_string(instructions).unwrap(),
            "Be precise."
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&config_file)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
