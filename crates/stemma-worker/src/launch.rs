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
//! - [`Mode::Job`], headless: an answer in a thread. In a container of its
//!   own, the agent has no limits: the container holds only the group's
//!   workspace, its sign-in and the job's token. On the computer itself, it
//!   only answers: it reads, and runs no commands.
//!
//! For each runner:
//!
//! - Claude Code reads Stemma's tools from an MCP configuration
//!   (`--mcp-config`) and its instructions from `--append-system-prompt`.
//!   A job runs it with `-p` and only those MCP tools
//!   (`--strict-mcp-config`): contained, with every tool and no asking
//!   (`--dangerously-skip-permissions`); otherwise with none of its own.
//! - Codex gets Stemma's tools and instructions as configuration values on
//!   its command line (`-c mcp_servers.stemma.…`,
//!   `-c developer_instructions=…`). A job runs `codex exec`, contained
//!   with full access, otherwise in its read-only sandbox.
//! - OpenCode reads a configuration of its own from `OPENCODE_CONFIG`,
//!   which it merges with the person's: Stemma's tools, and its
//!   instructions in a file. A job runs `opencode run`: contained, allowed
//!   everything; otherwise, only reading.
//!
//! Stemma's skills go in the launch's folder too, as a Claude Code plugin
//! (`--plugin-dir`) whose `skills` folder OpenCode also reads (`skills` in
//! its configuration). Codex reads skills only from its working folders
//! and the person's own: its instructions say where Stemma's are.

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
        /// The folder it starts in, as it sees it.
        dir: &'a Path,
        /// The group's workspace, as it sees it, which it may read.
        root: &'a Path,
        /// Whether it runs in a container of its own, with no limits there;
        /// else on the computer, only answering.
        contained: bool,
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
        // The person's sign-in reaches no agent: Stemma's tools get theirs
        // by arguments.
        command
            .args(&self.args)
            .env_remove("STEMMA_TOKEN")
            .env_remove("STEMMA_HUB");
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
    }
}

/// Stemma's skills, by name: what each agent may load when a task calls for it.
const SKILLS: &[(&str, &str)] = &[(
    "workshop-recipes",
    include_str!("../skills/workshop-recipes/SKILL.md"),
)];

/// Writes Stemma's skills in the launch's folder, as a Claude Code plugin,
/// and gives that plugin's folder as the agent sees it.
fn skills(launch: &Launch, prepared: &mut Prepared) -> anyhow::Result<PathBuf> {
    let plugin = launch.files.join("plugin");
    std::fs::create_dir_all(plugin.join(".claude-plugin"))?;
    let manifest = json!({
        "name": "stemma",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Stemma's skills for the agents it launches",
    });
    write(
        prepared,
        plugin.join(".claude-plugin").join("plugin.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    for (name, text) in SKILLS {
        let dir = plugin.join("skills").join(name);
        std::fs::create_dir_all(&dir)?;
        write(prepared, dir.join("SKILL.md"), text.as_bytes())?;
    }
    Ok(launch.files_seen.join("plugin"))
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
    let plugin = skills(launch, &mut prepared)?;
    match launch.runner {
        runners::CLAUDE_CODE => claude_code(launch, &plugin, &mut prepared)?,
        runners::CODEX => codex(launch, &plugin, &mut prepared),
        runners::OPENCODE => opencode(launch, &plugin, &mut prepared)?,
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
fn claude_code(launch: &Launch, plugin: &Path, prepared: &mut Prepared) -> anyhow::Result<()> {
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
            // Stemma through its tools, never through the command.
            push(args, &["--disallowedTools", "Bash(stemma:*)"]);
            push(args, &["--append-system-prompt", launch.instructions]);
        }
        Mode::Job {
            contained, allowed, ..
        } => {
            push(args, &["-p", "--output-format", "stream-json", "--verbose"]);
            push(args, &["--no-session-persistence", "--strict-mcp-config"]);
            push(args, &["--mcp-config", &mcp_file]);
            if *contained {
                push(args, &["--dangerously-skip-permissions"]);
            } else {
                push(args, &["--tools", ""]);
                push(args, &["--allowedTools", &stemma_tools(allowed).join(",")]);
                push(args, &["--permission-mode", "dontAsk"]);
            }
            push(args, &["--append-system-prompt", launch.instructions]);
        }
    }
    push(args, &["--plugin-dir", &plugin.display().to_string()]);
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

fn codex(launch: &Launch, plugin: &Path, prepared: &mut Prepared) {
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
    let mut instructions = launch.instructions.to_owned();
    instructions.push_str("\n\nStemma's skills: read one when a task calls for it.");
    for (name, text) in SKILLS {
        let description = text
            .lines()
            .find_map(|line| line.strip_prefix("description: "))
            .unwrap_or_default();
        let path = plugin.join("skills").join(name).join("SKILL.md");
        instructions.push_str(&format!("\n- {name}, in {}: {description}", path.display()));
    }
    let instructions = toml_value(toml::Value::String(instructions));
    let allowed = match &launch.mode {
        Mode::Session { allowed } | Mode::Job { allowed, .. } => allowed,
    };
    let enabled = toml_value(toml::Value::Array(
        allowed.iter().cloned().map(toml::Value::String).collect(),
    ));
    // Codex 0.163.0: config/src/mcp_types.rs, McpServerConfig and AppToolApproval.
    // https://github.com/openai/codex/blob/rust-v0.163.0/codex-rs/config/src/mcp_types.rs
    let configuration = [
        format!("mcp_servers.stemma.command={command}"),
        format!("mcp_servers.stemma.args={mcp_args}"),
        "mcp_servers.stemma.default_tools_approval_mode=\"approve\"".into(),
        format!("mcp_servers.stemma.enabled_tools={enabled}"),
        format!("developer_instructions={instructions}"),
    ];
    let mut answer = None;
    let args = &mut prepared.args;
    if let Mode::Job { dir, contained, .. } = &launch.mode {
        // In a container of its own, the container is its sandbox: Codex's
        // own needs what a container does not allow.
        let sandbox = if *contained {
            "danger-full-access"
        } else {
            "read-only"
        };
        let file = dir.join("answer.md");
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
        answer = Some(file);
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

fn opencode(launch: &Launch, plugin: &Path, prepared: &mut Prepared) -> anyhow::Result<()> {
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
        "skills": [plugin.join("skills").display().to_string()],
    });
    let allowed = match &launch.mode {
        Mode::Session { allowed } | Mode::Job { allowed, .. } => allowed,
    };
    let mut permission = serde_json::Map::new();
    let unlimited = matches!(
        launch.mode,
        Mode::Job {
            contained: true,
            ..
        }
    );
    if let Mode::Job { root, .. } = &launch.mode {
        config["autoupdate"] = json!(false);
        config["share"] = json!("disabled");
        if unlimited {
            permission.insert("*".into(), json!("allow"));
        } else {
            permission.insert("*".into(), json!("deny"));
            for tool in ["read", "glob", "grep", "list"] {
                permission.insert(tool.into(), json!("allow"));
            }
            permission.insert(
                "external_directory".into(),
                json!({root.display().to_string(): "allow"}),
            );
        }
    }
    if !unlimited {
        // Scope automatic permission to the tools this Stemma session exposes.
        // https://opencode.ai/docs/permissions/ and /docs/mcp-servers/
        permission.insert("stemma_*".into(), json!("deny"));
        for tool in allowed {
            permission.insert(format!("stemma_{tool}"), json!("allow"));
        }
    }
    config["permission"] = json!(permission);
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
    fn no_agent_inherits_the_persons_sign_in() {
        let prepared = Prepared {
            program: "claude".into(),
            args: Vec::new(),
            env: vec![("STEMMA_CONFIG_DIR".into(), "/session".into())],
            files: Vec::new(),
            answer: None,
        };
        let command = prepared.command();
        let envs: Vec<_> = command.as_std().get_envs().collect();
        for name in ["STEMMA_TOKEN", "STEMMA_HUB"] {
            assert!(envs.contains(&(std::ffi::OsStr::new(name), None)), "{name}");
        }
        assert!(envs.contains(&(
            std::ffi::OsStr::new("STEMMA_CONFIG_DIR"),
            Some(std::ffi::OsStr::new("/session"))
        )));
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
        let plugin = files.path().join("plugin");
        assert_eq!(
            prepared.args,
            [
                "--mcp-config",
                &mcp_file,
                "--allowedTools",
                "mcp__stemma__read_thread,mcp__stemma__search",
                "--disallowedTools",
                "Bash(stemma:*)",
                "--append-system-prompt",
                "Be precise.",
                "--plugin-dir",
                &plugin.display().to_string(),
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
        // Stemma's skills, as a plugin of this session only.
        let manifest = read_json(&plugin.join(".claude-plugin/plugin.json")).unwrap();
        assert_eq!(manifest["name"], "stemma");
        assert!(plugin.join("skills/workshop-recipes/SKILL.md").is_file());
    }

    #[test]
    fn claude_code_for_a_job() {
        let files = tempfile::tempdir().unwrap();
        let mcp = mcp();
        let mode = Mode::Job {
            root: Path::new("/workspace"),
            dir: files.path(),
            contained: false,
            allowed: vec!["read_thread".into()],
        };
        let prepared = prepare(&launch(runners::CLAUDE_CODE, files.path(), &mcp, mode)).unwrap();
        for flag in ["-p", "--strict-mcp-config", "dontAsk", "--tools"] {
            assert!(prepared.args.contains(&flag.to_owned()), "{flag}");
        }
        assert!(!prepared.args.iter().any(|a| a.contains("skip-permissions")));

        // In a container, every tool, without asking.
        let mode = Mode::Job {
            root: Path::new("/stemma"),
            dir: Path::new("/stemma/scratch/7"),
            contained: true,
            allowed: vec!["read_thread".into()],
        };
        let prepared = prepare(&launch(runners::CLAUDE_CODE, files.path(), &mcp, mode)).unwrap();
        assert!(
            prepared
                .args
                .contains(&"--dangerously-skip-permissions".to_owned())
        );
        for flag in ["--tools", "--allowedTools", "dontAsk"] {
            assert!(!prepared.args.contains(&flag.to_owned()), "{flag}");
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
        // Codex reads Stemma's skills where its instructions say.
        let skill = files.path().join("plugin/skills/workshop-recipes/SKILL.md");
        let instructions = prepared
            .args
            .iter()
            .find(|a| a.starts_with("developer_instructions="))
            .unwrap()
            .clone();
        assert!(instructions.contains(&format!("- workshop-recipes, in {}: ", skill.display())));
        assert_eq!(
            prepared.args,
            [
                "-c",
                "mcp_servers.stemma.command=\"/bin/stemma\"",
                "-c",
                "mcp_servers.stemma.args=[\"mcp\", \"--token-file\", \"/t/token\"]",
                "-c",
                "mcp_servers.stemma.default_tools_approval_mode=\"approve\"",
                "-c",
                "mcp_servers.stemma.enabled_tools=[]",
                "-c",
                &instructions,
                "--model",
                "sonnet"
            ]
        );
        assert!(instructions.starts_with("developer_instructions=\"\"\"\nBe precise."));
        assert!(
            files
                .path()
                .join("plugin/skills/workshop-recipes/SKILL.md")
                .is_file()
        );

        let mode = Mode::Job {
            root: Path::new("/workspace"),
            dir: files.path(),
            contained: false,
            allowed: vec![],
        };
        let prepared = prepare(&launch(runners::CODEX, files.path(), &mcp, mode)).unwrap();
        assert_eq!(prepared.args[0], "exec");
        assert!(prepared.args.contains(&"read-only".to_owned()));
        assert_eq!(prepared.args.last().unwrap(), "-");
        assert_eq!(prepared.answer, Some(files.path().join("answer.md")));

        // In a container, the files are where it sees them, and the
        // container is its sandbox.
        let seen = Path::new("/stemma/jobs/7");
        let mode = Mode::Job {
            root: Path::new("/workspace"),
            dir: Path::new("/stemma/scratch/7"),
            contained: true,
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
                .iter()
                .any(|a| a.contains("/stemma/jobs/7/plugin/skills/workshop-recipes/SKILL.md"))
        );
        assert!(
            prepared
                .args
                .contains(&"/stemma/scratch/7/answer.md".to_owned())
        );
        assert!(prepared.args.contains(&"/stemma/scratch/7".to_owned()));
        assert_eq!(
            prepared.answer,
            Some(Path::new("/stemma/scratch/7/answer.md").into())
        );
    }

    #[test]
    fn opencode_in_a_container_reads_its_files_where_it_sees_them() {
        let files = tempfile::tempdir().unwrap();
        let mcp = mcp();
        let mode = Mode::Job {
            root: Path::new("/workspace"),
            dir: Path::new("/stemma/scratch/7"),
            contained: true,
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
        assert_eq!(config["skills"][0], "/stemma/jobs/7/plugin/skills");
        // Contained, it is allowed everything.
        assert_eq!(config["permission"], json!({"*": "allow"}));
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
        // Only Stemma's permissions change in an interactive session.
        assert_eq!(config["permission"], json!({"stemma_*": "deny"}));
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
    #[test]
    fn on_the_computer_an_answer_only_reads() {
        let files = tempfile::tempdir().unwrap();
        let mcp = mcp();
        for runner in runners::ALL {
            let prepared = prepare(&launch(
                runner,
                files.path(),
                &mcp,
                Mode::Job {
                    root: Path::new("/workspace"),
                    dir: files.path(),
                    contained: false,
                    allowed: vec!["read_thread".into(), "search".into()],
                },
            ))
            .unwrap();
            match *runner {
                runners::CODEX => {
                    let values: Vec<_> = prepared
                        .args
                        .windows(2)
                        .filter(|w| w[0] == "-c")
                        .map(|w| w[1].as_str())
                        .collect();
                    assert!(values.contains(&"approval_policy=\"never\""));
                    assert!(
                        values.contains(
                            &"mcp_servers.stemma.default_tools_approval_mode=\"approve\""
                        )
                    );
                    assert!(values.contains(
                        &"mcp_servers.stemma.enabled_tools=[\"read_thread\", \"search\"]"
                    ));
                    assert!(
                        !values.iter().any(|v| v.contains("approval_mode")
                            && !v.starts_with("mcp_servers.stemma."))
                    );
                }
                runners::CLAUDE_CODE => {
                    let tools = prepared
                        .args
                        .windows(2)
                        .find(|w| w[0] == "--allowedTools")
                        .unwrap();
                    assert_eq!(tools[1], "mcp__stemma__read_thread,mcp__stemma__search");
                    assert!(
                        !prepared
                            .args
                            .iter()
                            .any(|arg| arg.contains("skip-permissions"))
                    );
                }
                runners::OPENCODE => {
                    let config = read_json(&files.path().join("opencode.json")).unwrap();
                    assert_eq!(config["permission"]["stemma_search"], "allow");
                    assert_eq!(config["permission"]["stemma_read_thread"], "allow");
                    assert_eq!(config["permission"]["stemma_*"], "deny");
                    assert_eq!(config["permission"]["*"], "deny");
                    assert!(config["permission"]["bash"].is_null());
                    assert!(config["permission"]["edit"].is_null());
                }
                _ => unreachable!(),
            }
        }
    }
}
