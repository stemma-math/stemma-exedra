//! Where the workshop's jobs run: each in a container of its own.
//!
//! The workshop runs work that other people ask for, with an agent that
//! runs commands; so a machine runs each of its jobs (a task, or a build
//! of a library) in a container of its own, with Docker or Podman, and
//! the image `docker/workshop/Dockerfile` describes: Lean, the agents'
//! programs, git and `stemma`. The container is given only what the job
//! works on, at the same place under `/stemma` as under the machine's
//! work folder:
//!
//! - a task's worktree, read-write; the library's shared Lake packages
//!   (Mathlib), read-only; and the job's folder, with its token and the
//!   agent's configuration, read-write;
//! - for a build, the group's checkout, read-write, with its `.git`
//!   hidden.
//!
//! It runs as the machine's user (never root), with no capabilities and
//! no new privileges, with limits on memory, CPUs and processes, for as
//! long as the job may take; and is removed when the job ends, whatever
//! the end. The agent's sign-in enters it as its owner configured, per
//! runner: environment variables passed by name, so that their values are
//! on no command line, and files mounted read-only. Nothing of it reaches
//! the hub.
//!
//! Where containers are not available, the machine's owner may choose,
//! explicitly, to run the workshop's jobs on the machine itself
//! ([`Isolation::Host`]); and with neither, the machine runs none.
//! People's own jobs, and their sessions, run on the machine: they are
//! their own.
//!
//! The step runner ([`crate::workshop`]) and the builds
//! ([`crate::library`]) run every command through a [`Sandbox`]: the
//! machine itself ([`HostSandbox`]), or a [`Container`], whose command
//! line each runtime ([`Engine`]: [`Docker`], [`Podman`]) builds.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use stemma_core::api::{Isolation, runners};
use tokio::process::Command;

use crate::detect;

/// Where a container sees the machine's work folder.
pub const ROOT: &str = "/stemma";

/// The home of the container's user.
pub const HOME: &str = "/home/stemma";

/// The workshop's image, as `stemma machine workshop image` tags it.
pub fn default_image() -> String {
    format!("stemma-workshop:{}", env!("CARGO_PKG_VERSION"))
}

/// How long a runtime has to answer whether it works.
const RUNTIME_WAIT: Duration = Duration::from_secs(30);

// ── Configuration ─────────────────────────────────────────────────────

/// How a machine isolates the workshop's jobs, as its owner configures it
/// (`[isolation]` in its configuration).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IsolationConfig {
    /// `auto` (Docker, or else Podman), `docker`, `podman`, or `host`:
    /// on the machine itself, unisolated. `auto` if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<Runtime>,
    /// The runtime's program, if not `docker` or `podman` on the path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The image; `stemma-workshop:<this version>` if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// The most memory a job's container may use: `8g` if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
    /// How many CPUs it may use: 4 if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpus: Option<f64>,
    /// How many processes it may run at once: 4096 if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pids: Option<u32>,
    /// The runtime's network it joins (`--network`); the runtime's own
    /// if absent. Its agent needs to reach its model's provider and the
    /// hub: `none` leaves it with neither.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    /// The hub's address from inside a container, if not the one the
    /// machine uses: that, or, if it is on the machine itself
    /// (`localhost`), the name the runtime gives the machine.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hub_url: Option<String>,
    /// How each runner's agent signs in inside the container, by runner.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agents: BTreeMap<String, AgentSignIn>,
}

/// Which runtime runs the workshop's jobs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    /// Docker, or else Podman, whichever works.
    #[default]
    Auto,
    Docker,
    Podman,
    /// None: on the machine itself, unisolated. The owner's explicit
    /// choice, where containers are not available.
    Host,
}

impl Runtime {
    pub fn parse(text: &str) -> Option<Runtime> {
        match text {
            "auto" => Some(Runtime::Auto),
            "docker" => Some(Runtime::Docker),
            "podman" => Some(Runtime::Podman),
            "host" => Some(Runtime::Host),
            _ => None,
        }
    }
}

/// How an agent signs in inside a job's container: what of the machine
/// enters it. None of it leaves the machine.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSignIn {
    /// Environment variables passed from the machine's, by name:
    /// `ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN`, `OPENAI_API_KEY`,
    /// `OPENROUTER_API_KEY`…
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    /// Files of the machine, mounted read-only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<SignInFile>,
    /// The runner's program in the container, if not its usual name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

/// A file of the machine that an agent signs in with.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SignInFile {
    /// Where it is on the machine; `~/` is the user's home.
    pub from: String,
    /// Where it is in the container; if absent, the same place under the
    /// container's home as `from` under the machine's (`~/…`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

impl SignInFile {
    /// Where it is on the machine.
    pub fn source(&self, home: Option<&Path>) -> anyhow::Result<PathBuf> {
        match self.from.strip_prefix("~/") {
            Some(rest) => Ok(home
                .context("this machine's user has no home folder, for `~/`")?
                .join(rest)),
            None => {
                let path = PathBuf::from(&self.from);
                anyhow::ensure!(
                    path.is_absolute(),
                    "a sign-in file is an absolute path, or starts with `~/`: {}",
                    self.from
                );
                Ok(path)
            }
        }
    }

    /// Where it is in the container.
    pub fn target(&self) -> anyhow::Result<PathBuf> {
        if let Some(to) = &self.to {
            let path = PathBuf::from(to);
            anyhow::ensure!(
                path.is_absolute(),
                "where a sign-in file goes in the container is an absolute path: {to}"
            );
            anyhow::ensure!(
                !path.starts_with(ROOT),
                "a sign-in file does not go under {ROOT}, which is the job's: {to}"
            );
            return Ok(path);
        }
        match self.from.strip_prefix("~/") {
            Some(rest) => Ok(Path::new(HOME).join(rest)),
            None => anyhow::bail!(
                "say where {} goes in the container (`to`), or give it from `~/`",
                self.from
            ),
        }
    }
}

/// Whether `name` is an environment variable's name.
fn env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Variables the container sets itself, which a sign-in does not pass.
const KEPT: &[&str] = &["HOME", "PATH", "USER", "ELAN_HOME"];

impl IsolationConfig {
    /// The runtime chosen.
    pub fn runtime(&self) -> Runtime {
        self.runtime.unwrap_or_default()
    }

    pub fn image(&self) -> String {
        self.image.clone().unwrap_or_else(default_image)
    }

    pub fn limits(&self) -> Limits {
        Limits {
            memory: self.memory.clone().unwrap_or_else(|| "8g".into()),
            cpus: self.cpus.unwrap_or(4.0),
            pids: self.pids.unwrap_or(4096),
        }
    }

    /// Checks what can be checked before running anything.
    pub fn validate(&self) -> anyhow::Result<()> {
        for (runner, sign_in) in &self.agents {
            anyhow::ensure!(
                runners::ALL.contains(&runner.as_str()),
                "there is no runner {runner} (in [isolation.agents])"
            );
            for name in &sign_in.env {
                anyhow::ensure!(
                    env_name(name),
                    "{name} is not an environment variable's name (for {runner})"
                );
                anyhow::ensure!(
                    !KEPT.contains(&name.as_str()),
                    "the container sets {name} itself (for {runner})"
                );
            }
            for file in &sign_in.files {
                file.target()?;
            }
        }
        if let Some(memory) = &self.memory {
            anyhow::ensure!(
                memory
                    .trim_end_matches(['b', 'k', 'm', 'g', 'B', 'K', 'M', 'G'])
                    .parse::<u64>()
                    .is_ok_and(|n| n > 0),
                "the memory is a number with a unit, such as 8g: {memory}"
            );
        }
        anyhow::ensure!(
            self.cpus.is_none_or(|c| c > 0.0),
            "the CPUs are more than 0"
        );
        anyhow::ensure!(
            self.pids.is_none_or(|p| p >= 64),
            "the processes are at least 64"
        );
        Ok(())
    }
}

/// How much a job's container may use.
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    pub memory: String,
    pub cpus: f64,
    pub pids: u32,
}

// ── The runtimes ──────────────────────────────────────────────────────

/// A container runtime's command line, where it differs between them.
pub trait Engine: Send + Sync + std::fmt::Debug {
    /// Which it is.
    fn isolation(&self) -> Isolation;
    /// Its program.
    fn program(&self) -> &Path;
    /// The arguments that run the container as `user`, so that what it
    /// writes in the folders it is given is the machine's user's.
    fn user_args(&self, user: (u32, u32)) -> Vec<String>;
    /// The name containers reach the machine by.
    fn host_alias(&self) -> &'static str;
    /// The arguments that make that name reach the machine, where the
    /// runtime does not by itself.
    fn host_alias_args(&self) -> Vec<String>;
}

#[derive(Debug)]
pub struct Docker {
    pub program: PathBuf,
}

impl Engine for Docker {
    fn isolation(&self) -> Isolation {
        Isolation::Docker
    }

    fn program(&self) -> &Path {
        &self.program
    }

    fn user_args(&self, (uid, gid): (u32, u32)) -> Vec<String> {
        vec!["--user".into(), format!("{uid}:{gid}")]
    }

    fn host_alias(&self) -> &'static str {
        "host.docker.internal"
    }

    fn host_alias_args(&self) -> Vec<String> {
        // Docker Desktop has the name; Docker Engine, on Linux, is told.
        vec!["--add-host=host.docker.internal:host-gateway".into()]
    }
}

#[derive(Debug)]
pub struct Podman {
    pub program: PathBuf,
}

impl Engine for Podman {
    fn isolation(&self) -> Isolation {
        Isolation::Podman
    }

    fn program(&self) -> &Path {
        &self.program
    }

    fn user_args(&self, (uid, gid): (u32, u32)) -> Vec<String> {
        // Rootless, the user keeps its own id inside, rather than root's
        // mapped to it.
        vec![
            "--userns=keep-id".into(),
            "--user".into(),
            format!("{uid}:{gid}"),
        ]
    }

    fn host_alias(&self) -> &'static str {
        "host.containers.internal"
    }

    fn host_alias_args(&self) -> Vec<String> {
        Vec::new()
    }
}

/// The user a job's container runs as: the machine's, unless that is
/// root, whose place an unprivileged user takes.
pub fn container_user(owner: (u32, u32)) -> (u32, u32) {
    if owner.0 == 0 { (1000, 1000) } else { owner }
}

/// A folder or file a container is given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mount {
    /// A folder or file of the machine.
    Bind {
        source: PathBuf,
        target: PathBuf,
        read_only: bool,
    },
    /// An empty folder, read-only, hiding what is beneath it.
    Hidden { target: PathBuf },
}

/// What a job's container is given, and how far it may go.
#[derive(Clone, Debug)]
pub struct ContainerSpec {
    pub name: String,
    pub image: String,
    /// The user it runs as.
    pub user: (u32, u32),
    pub mounts: Vec<Mount>,
    /// Variables set, with their values: none of them secret.
    pub env: Vec<(String, String)>,
    /// Variables passed from the machine's environment, by name: the
    /// agent's sign-in.
    pub pass_env: Vec<String>,
    pub limits: Limits,
    pub network: Option<String>,
    /// Whether it reaches the machine by the runtime's name for it.
    pub host_alias: bool,
    /// How long it lives at most, even if this machine stops.
    pub lifetime: Duration,
    /// The job, for the container's label.
    pub job: i64,
}

/// The arguments of `<runtime> run` that start a job's container, which
/// then waits for its commands.
pub fn run_args(engine: &dyn Engine, spec: &ContainerSpec) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "run".into(),
        "--detach".into(),
        "--rm".into(),
        "--init".into(),
        "--name".into(),
        spec.name.clone(),
        "--label".into(),
        format!("org.stemma.job={}", spec.job),
    ];
    args.extend(engine.user_args(spec.user));
    args.extend([
        "--cap-drop=ALL".into(),
        "--security-opt=no-new-privileges".into(),
        format!("--memory={}", spec.limits.memory),
        format!("--memory-swap={}", spec.limits.memory),
        format!("--cpus={}", spec.limits.cpus),
        format!("--pids-limit={}", spec.limits.pids),
    ]);
    if let Some(network) = &spec.network {
        args.push(format!("--network={network}"));
    }
    if spec.host_alias {
        args.extend(engine.host_alias_args());
    }
    for (key, value) in &spec.env {
        args.push("--env".into());
        args.push(format!("{key}={value}"));
    }
    for name in &spec.pass_env {
        // The runtime takes the value from its own environment.
        args.push("--env".into());
        args.push(name.clone());
    }
    for mount in &spec.mounts {
        args.push("--mount".into());
        args.push(match mount {
            Mount::Bind {
                source,
                target,
                read_only,
            } => {
                let mut mount = format!(
                    "type=bind,source={},target={}",
                    source.display(),
                    target.display()
                );
                if *read_only {
                    mount.push_str(",readonly");
                }
                mount
            }
            Mount::Hidden { target } => {
                format!("type=tmpfs,target={},readonly", target.display())
            }
        });
    }
    args.extend([
        "--workdir".into(),
        ROOT.into(),
        spec.image.clone(),
        "sleep".into(),
        spec.lifetime.as_secs().max(1).to_string(),
    ]);
    args
}

/// The arguments of `<runtime> exec` that run `program` in a container,
/// in `dir`, with `env` (none of it secret), on standard input.
pub fn exec_args(
    name: &str,
    program: &str,
    args: &[String],
    env: &[(String, String)],
    dir: &Path,
) -> Vec<String> {
    let mut exec: Vec<String> = vec![
        "exec".into(),
        "--interactive".into(),
        "--workdir".into(),
        dir.display().to_string(),
    ];
    for (key, value) in env {
        exec.push("--env".into());
        exec.push(format!("{key}={value}"));
    }
    exec.push(name.to_owned());
    exec.push(program.to_owned());
    exec.extend(args.iter().cloned());
    exec
}

// ── Sandboxes ─────────────────────────────────────────────────────────

/// Where a job's commands run: the machine itself, or a container.
pub trait Sandbox: Send + Sync {
    /// How it isolates them.
    fn isolation(&self) -> Isolation;
    /// Where the job's commands see `path`, a path of the machine in its
    /// work folder.
    fn path(&self, path: &Path) -> PathBuf;
    /// The command that runs `program`, with `args` and `env` (none of it
    /// secret), in `dir`, a folder of the machine.
    fn command(
        &self,
        program: &str,
        args: &[String],
        env: &[(String, String)],
        dir: &Path,
    ) -> Command;
    /// The hub, as the job's commands reach it.
    fn hub(&self) -> &str;
    /// The `stemma` binary, as the job's commands run it.
    fn stemma(&self) -> PathBuf;
}

/// The machine itself.
pub struct HostSandbox {
    pub hub: String,
    pub stemma: PathBuf,
}

impl Sandbox for HostSandbox {
    fn isolation(&self) -> Isolation {
        Isolation::Host
    }

    fn path(&self, path: &Path) -> PathBuf {
        path.to_owned()
    }

    fn command(
        &self,
        program: &str,
        args: &[String],
        env: &[(String, String)],
        dir: &Path,
    ) -> Command {
        let mut command = Command::new(program);
        command.args(args).current_dir(dir);
        for (key, value) in env {
            command.env(key, value);
        }
        command
    }

    fn hub(&self) -> &str {
        &self.hub
    }

    fn stemma(&self) -> PathBuf {
        self.stemma.clone()
    }
}

/// A job's container, removed when dropped.
pub struct Container {
    engine: Arc<dyn Engine>,
    name: String,
    /// The machine's work folder, which it sees as [`ROOT`].
    work_dir: PathBuf,
    hub: String,
}

impl Container {
    /// Starts a job's container, which then runs its commands.
    pub async fn start(
        engine: Arc<dyn Engine>,
        spec: &ContainerSpec,
        work_dir: &Path,
        hub: String,
    ) -> anyhow::Result<Container> {
        let container = Container {
            engine,
            name: spec.name.clone(),
            work_dir: work_dir.to_owned(),
            hub,
        };
        let output = Command::new(container.engine.program())
            .args(run_args(container.engine.as_ref(), spec))
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .with_context(|| format!("running {}", container.engine.program().display()))?;
        anyhow::ensure!(
            output.status.success(),
            "Could not start the job's container: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Ok(container)
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        let _ = std::process::Command::new(self.engine.program())
            .args(["rm", "--force", &self.name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Where a container sees `path` of a machine whose work folder is
/// `work_dir`.
pub fn container_path(work_dir: &Path, path: &Path) -> PathBuf {
    match path.strip_prefix(work_dir) {
        Ok(rest) => Path::new(ROOT).join(rest),
        Err(_) => path.to_owned(),
    }
}

impl Sandbox for Container {
    fn isolation(&self) -> Isolation {
        self.engine.isolation()
    }

    fn path(&self, path: &Path) -> PathBuf {
        container_path(&self.work_dir, path)
    }

    fn command(
        &self,
        program: &str,
        args: &[String],
        env: &[(String, String)],
        dir: &Path,
    ) -> Command {
        let mut command = Command::new(self.engine.program());
        command.args(exec_args(&self.name, program, args, env, &self.path(dir)));
        command
    }

    fn hub(&self) -> &str {
        &self.hub
    }

    fn stemma(&self) -> PathBuf {
        "stemma".into()
    }
}

// ── What a machine isolates with ──────────────────────────────────────

/// How a machine runs the workshop's jobs, as found when it starts.
#[derive(Clone, Debug)]
pub enum Isolator {
    /// On the machine itself, unisolated: its owner's choice.
    Host,
    /// Each in a container.
    Containers(Arc<Containers>),
}

/// A machine's containers for the workshop: its runtime, image, limits and
/// how agents sign in inside.
#[derive(Debug)]
pub struct Containers {
    pub engine: Arc<dyn Engine>,
    pub image: String,
    pub limits: Limits,
    pub network: Option<String>,
    /// The hub, as containers reach it, and whether by the runtime's name
    /// for the machine.
    pub hub: String,
    pub host_alias: bool,
    pub agents: BTreeMap<String, AgentSignIn>,
}

impl Isolator {
    pub fn isolation(&self) -> Isolation {
        match self {
            Isolator::Host => Isolation::Host,
            Isolator::Containers(containers) => containers.engine.isolation(),
        }
    }
}

/// The hub as containers reach it: as the machine does, unless it is on
/// the machine itself (a loopback address), which containers reach by
/// `alias`; and whether it was.
pub fn hub_in_container(hub: &str, alias: &str) -> (String, bool) {
    let (scheme, rest) = match hub.split_once("://") {
        Some(split) => split,
        None => return (hub.to_owned(), false),
    };
    let (authority, path) = match rest.find('/') {
        Some(at) => rest.split_at(at),
        None => (rest, ""),
    };
    let (host, port) = if let Some(after) = authority.strip_prefix('[') {
        match after.split_once(']') {
            Some((host, port)) => (host, port),
            None => return (hub.to_owned(), false),
        }
    } else {
        match authority.rfind(':') {
            Some(at) => (&authority[..at], &authority[at..]),
            None => (authority, ""),
        }
    };
    let loopback = host == "localhost"
        || host == "::1"
        || host == "0.0.0.0"
        || host.starts_with("127.")
        || host.ends_with(".localhost");
    if loopback {
        (format!("{scheme}://{alias}{port}{path}"), true)
    } else {
        (hub.to_owned(), false)
    }
}

/// Whether a runtime's program works: `<program> info` ends well, in a
/// while.
async fn works(program: &Path) -> bool {
    let mut command = Command::new(program);
    command
        .arg("info")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    matches!(
        tokio::time::timeout(RUNTIME_WAIT, child.wait()).await,
        Ok(Ok(status)) if status.success()
    )
}

/// Whether the runtime has `image`.
async fn has_image(program: &Path, image: &str) -> bool {
    Command::new(program)
        .args(["image", "inspect", image])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
        .is_ok_and(|status| status.success())
}

/// The engine of `kind` whose program is found on `search` and works.
async fn engine(
    kind: Runtime,
    command: Option<&str>,
    search: Option<&OsStr>,
) -> Option<Arc<dyn Engine>> {
    let program = command.unwrap_or(match kind {
        Runtime::Docker => "docker",
        Runtime::Podman => "podman",
        _ => return None,
    });
    let program = detect::find_program(program, search)?;
    if !works(&program).await {
        return None;
    }
    Some(match kind {
        Runtime::Podman => Arc::new(Podman { program }),
        _ => Arc::new(Docker { program }),
    })
}

/// The runtime a machine would use, as `config` says, found on `search`:
/// Docker or Podman, and whether its program works; none for the host, or
/// if none works.
pub async fn find_engine(
    config: &IsolationConfig,
    search: Option<&OsStr>,
) -> Option<Arc<dyn Engine>> {
    let command = config.command.as_deref();
    match config.runtime() {
        Runtime::Host => None,
        Runtime::Auto => match engine(Runtime::Docker, command, search).await {
            Some(found) => Some(found),
            None => engine(Runtime::Podman, command, search).await,
        },
        kind => engine(kind, command, search).await,
    }
}

/// How a machine runs the workshop's jobs, as `config` says and what it
/// finds on `search` (a list of folders, as `PATH` has them): on itself,
/// if its owner chose so; else in containers, with a runtime that works
/// and has the image. If it can do neither, why.
pub async fn resolve(
    config: &IsolationConfig,
    hub: &str,
    search: Option<&OsStr>,
) -> Result<Isolator, String> {
    if let Err(error) = config.validate() {
        return Err(format!("its isolation is misconfigured: {error:#}"));
    }
    if config.runtime() == Runtime::Host {
        return Ok(Isolator::Host);
    }
    let Some(engine) = find_engine(config, search).await else {
        let wanted = match config.runtime() {
            Runtime::Docker => "Docker",
            Runtime::Podman => "Podman",
            _ => "Docker or Podman",
        };
        return Err(format!(
            "{wanted} is not installed or does not work here (`info` fails), and the machine's \
             owner did not choose to run the workshop's jobs on it unisolated"
        ));
    };
    let image = config.image();
    if !has_image(engine.program(), &image).await {
        return Err(format!(
            "{} has no image {image}: build it with `stemma machine workshop image`",
            engine.program().display()
        ));
    }
    let (hub, host_alias) = match &config.hub_url {
        Some(url) => (url.trim_end_matches('/').to_owned(), false),
        None => hub_in_container(hub, engine.host_alias()),
    };
    Ok(Isolator::Containers(Arc::new(Containers {
        engine,
        image,
        limits: config.limits(),
        network: config.network.clone(),
        hub,
        host_alias,
        agents: config.agents.clone(),
    })))
}

impl Containers {
    /// How `runner`'s agent signs in inside: the variables passed by name,
    /// and the files mounted, as `(machine, container)`.
    pub fn sign_in(
        &self,
        runner: &str,
        home: Option<&Path>,
    ) -> anyhow::Result<(Vec<String>, Vec<Mount>)> {
        let Some(sign_in) = self.agents.get(runner) else {
            return Ok((Vec::new(), Vec::new()));
        };
        let mut mounts = Vec::new();
        for file in &sign_in.files {
            let source = file.source(home)?;
            anyhow::ensure!(
                source.is_file(),
                "{} signs in with {}, which is not a file on this machine",
                runners::name(runner),
                source.display()
            );
            mounts.push(Mount::Bind {
                source,
                target: file.target()?,
                read_only: true,
            });
        }
        Ok((sign_in.env.clone(), mounts))
    }

    /// The program of `runner` in the container.
    pub fn program(&self, runner: &str) -> Option<String> {
        self.agents.get(runner).and_then(|a| a.command.clone())
    }

    /// A job's container, as `spec` asks, with this machine's image,
    /// limits and network.
    pub fn spec(
        &self,
        job: i64,
        user: (u32, u32),
        mounts: Vec<Mount>,
        pass_env: Vec<String>,
        lifetime: Duration,
    ) -> ContainerSpec {
        ContainerSpec {
            name: format!("stemma-job-{job}-{:08x}", rand::random::<u32>()),
            image: self.image.clone(),
            user: container_user(user),
            mounts,
            env: vec![("HOME".into(), HOME.into())],
            pass_env,
            limits: self.limits.clone(),
            network: self.network.clone(),
            host_alias: self.host_alias,
            lifetime,
            job,
        }
    }

    /// Starts a job's container.
    pub async fn start(&self, spec: &ContainerSpec, work_dir: &Path) -> anyhow::Result<Container> {
        Container::start(self.engine.clone(), spec, work_dir, self.hub.clone()).await
    }
}

/// The owner of `path`, as user and group ids: the machine's user, for its
/// work folder.
pub fn owner_of(path: &Path) -> (u32, u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(metadata) = std::fs::metadata(path) {
            return (metadata.uid(), metadata.gid());
        }
    }
    let _ = path;
    (1000, 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ContainerSpec {
        ContainerSpec {
            name: "stemma-job-7-0000abcd".into(),
            image: "stemma-workshop:1.0.0".into(),
            user: (501, 20),
            mounts: vec![
                Mount::Bind {
                    source: "/w/tasks/geometry-w3".into(),
                    target: "/stemma/tasks/geometry-w3".into(),
                    read_only: false,
                },
                Mount::Bind {
                    source: "/w/libraries/geometry/.lake/packages".into(),
                    target: "/stemma/libraries/geometry/.lake/packages".into(),
                    read_only: true,
                },
                Mount::Hidden {
                    target: "/stemma/libraries/geometry/.git".into(),
                },
            ],
            env: vec![("HOME".into(), HOME.into())],
            pass_env: vec!["CLAUDE_CODE_OAUTH_TOKEN".into()],
            limits: Limits {
                memory: "8g".into(),
                cpus: 4.0,
                pids: 4096,
            },
            network: None,
            host_alias: true,
            lifetime: Duration::from_secs(600),
            job: 7,
        }
    }

    fn position(args: &[String], item: &str) -> usize {
        args.iter()
            .position(|a| a == item)
            .unwrap_or_else(|| panic!("{item} in {args:?}"))
    }

    #[test]
    fn docker_runs_a_jobs_container_confined() {
        // The token's value is in the machine's environment, never here.
        let docker = Docker {
            program: "docker".into(),
        };
        let args = run_args(&docker, &spec());
        assert_eq!(args[..3], ["run", "--detach", "--rm"]);
        for flag in [
            "--init",
            "--cap-drop=ALL",
            "--security-opt=no-new-privileges",
            "--memory=8g",
            "--memory-swap=8g",
            "--cpus=4",
            "--pids-limit=4096",
            "--add-host=host.docker.internal:host-gateway",
        ] {
            position(&args, flag);
        }
        assert!(!args.iter().any(|a| a.contains("privileged")));
        assert!(!args.iter().any(|a| a.starts_with("--network")));
        let user = position(&args, "--user");
        assert_eq!(args[user + 1], "501:20");
        assert!(!args.contains(&"--userns=keep-id".to_owned()));
        // The sign-in, by name only.
        let pass = position(&args, "CLAUDE_CODE_OAUTH_TOKEN");
        assert_eq!(args[pass - 1], "--env");
        position(&args, "HOME=/home/stemma");
        position(
            &args,
            "type=bind,source=/w/tasks/geometry-w3,target=/stemma/tasks/geometry-w3",
        );
        position(
            &args,
            "type=bind,source=/w/libraries/geometry/.lake/packages,\
             target=/stemma/libraries/geometry/.lake/packages,readonly",
        );
        position(
            &args,
            "type=tmpfs,target=/stemma/libraries/geometry/.git,readonly",
        );
        assert_eq!(
            args[args.len() - 5..],
            [
                "--workdir",
                "/stemma",
                "stemma-workshop:1.0.0",
                "sleep",
                "600"
            ]
        );
        // Nothing of the machine's home but what was given.
        assert!(
            !args
                .iter()
                .any(|a| a.contains("/Users/") || a.contains("~"))
        );
    }

    #[test]
    fn podman_keeps_the_users_id_and_needs_no_alias() {
        let podman = Podman {
            program: "podman".into(),
        };
        let mut spec = spec();
        spec.network = Some("none".into());
        let args = run_args(&podman, &spec);
        position(&args, "--userns=keep-id");
        let user = position(&args, "--user");
        assert_eq!(args[user + 1], "501:20");
        position(&args, "--network=none");
        assert!(!args.iter().any(|a| a.starts_with("--add-host")));
        assert_eq!(podman.host_alias(), "host.containers.internal");
    }

    #[test]
    fn root_is_never_the_containers_user() {
        assert_eq!(container_user((0, 0)), (1000, 1000));
        assert_eq!(container_user((501, 20)), (501, 20));
    }

    #[test]
    fn commands_run_in_the_container_where_it_sees_the_work_folder() {
        let args = exec_args(
            "stemma-job-7",
            "claude",
            &["-p".into()],
            &[(
                "OPENCODE_CONFIG".into(),
                "/stemma/jobs/7/opencode.json".into(),
            )],
            Path::new("/stemma/tasks/geometry-w3"),
        );
        assert_eq!(
            args,
            [
                "exec",
                "--interactive",
                "--workdir",
                "/stemma/tasks/geometry-w3",
                "--env",
                "OPENCODE_CONFIG=/stemma/jobs/7/opencode.json",
                "stemma-job-7",
                "claude",
                "-p"
            ]
        );
        assert_eq!(
            container_path(Path::new("/w"), Path::new("/w/jobs/7/token")),
            Path::new("/stemma/jobs/7/token")
        );
    }

    #[test]
    fn the_hub_on_the_machine_is_reached_by_its_alias() {
        let alias = "host.docker.internal";
        assert_eq!(
            hub_in_container("http://127.0.0.1:8080", alias),
            ("http://host.docker.internal:8080".to_owned(), true)
        );
        assert_eq!(
            hub_in_container("http://localhost/stemma", alias),
            ("http://host.docker.internal/stemma".to_owned(), true)
        );
        assert_eq!(
            hub_in_container("http://[::1]:8080", alias),
            ("http://host.docker.internal:8080".to_owned(), true)
        );
        assert_eq!(
            hub_in_container("https://stemma.example.org", alias),
            ("https://stemma.example.org".to_owned(), false)
        );
    }

    #[test]
    fn reads_and_checks_how_agents_sign_in() {
        let config: IsolationConfig = toml::from_str(
            r#"
            runtime = "podman"
            memory = "12g"
            [agents.claude-code]
            env = ["CLAUDE_CODE_OAUTH_TOKEN"]
            [agents.codex]
            files = [{ from = "~/.codex/auth.json" }]
            [agents.opencode]
            env = ["OPENROUTER_API_KEY"]
            files = [{ from = "/srv/opencode/auth.json", to = "/home/stemma/.local/share/opencode/auth.json" }]
            "#,
        )
        .unwrap();
        config.validate().unwrap();
        assert_eq!(config.runtime(), Runtime::Podman);
        assert_eq!(config.limits().memory, "12g");
        assert_eq!(config.limits().cpus, 4.0);
        let codex = &config.agents["codex"].files[0];
        assert_eq!(
            codex.source(Some(Path::new("/home/ada"))).unwrap(),
            Path::new("/home/ada/.codex/auth.json")
        );
        assert_eq!(
            codex.target().unwrap(),
            Path::new("/home/stemma/.codex/auth.json")
        );
        assert_eq!(config.image(), default_image());

        for wrong in [
            "[agents.api]\nenv = [\"KEY\"]",
            "[agents.codex]\nenv = [\"OPENAI_API_KEY=sk-…\"]",
            "[agents.codex]\nenv = [\"PATH\"]",
            "[agents.codex]\nfiles = [{ from = \"auth.json\", to = \"/home/stemma/auth.json\" }]",
            "[agents.codex]\nfiles = [{ from = \"/etc/auth.json\" }]",
            "[agents.codex]\nfiles = [{ from = \"~/a\", to = \"/stemma/jobs/a\" }]",
            "memory = \"lots\"",
            "pids = 2",
        ] {
            let config: IsolationConfig = toml::from_str(wrong).unwrap();
            let checked = config.validate().and_then(|()| {
                for sign_in in config.agents.values() {
                    for file in &sign_in.files {
                        file.source(Some(Path::new("/home/ada")))?;
                    }
                }
                Ok(())
            });
            assert!(checked.is_err(), "{wrong}");
        }
        assert!(toml::from_str::<IsolationConfig>("runtime = \"chroot\"").is_err());
    }

    fn fake(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn finds_a_runtime_that_works_with_the_image() {
        let bin = tempfile::tempdir().unwrap();
        let search = std::env::join_paths([bin.path()]).unwrap();
        let hub = "http://127.0.0.1:8080";
        let auto = IsolationConfig::default();

        // Nothing installed.
        let error = resolve(&auto, hub, Some(&search)).await.unwrap_err();
        assert!(error.contains("Docker or Podman"), "{error}");
        // The owner's choice needs nothing.
        let host = IsolationConfig {
            runtime: Some(Runtime::Host),
            ..Default::default()
        };
        assert!(matches!(
            resolve(&host, hub, Some(&search)).await,
            Ok(Isolator::Host)
        ));

        // Docker is installed but its daemon is not running; Podman works,
        // but has no image yet.
        fake(bin.path(), "docker", "exit 1");
        fake(
            bin.path(),
            "podman",
            "case \"$1\" in info) exit 0;; image) [ -f \"$(dirname \"$0\")/image\" ];; esac",
        );
        let error = resolve(&auto, hub, Some(&search)).await.unwrap_err();
        assert!(error.contains("stemma machine workshop image"), "{error}");
        std::fs::write(bin.path().join("image"), "").unwrap();
        let Ok(Isolator::Containers(found)) = resolve(&auto, hub, Some(&search)).await else {
            panic!("Podman, with its image");
        };
        assert_eq!(found.engine.isolation(), Isolation::Podman);
        assert_eq!(found.hub, "http://host.containers.internal:8080");
        assert!(found.host_alias);
        // Docker, asked for by name, does not work.
        let docker = IsolationConfig {
            runtime: Some(Runtime::Docker),
            ..Default::default()
        };
        let error = resolve(&docker, hub, Some(&search)).await.unwrap_err();
        assert!(error.starts_with("Docker is not"), "{error}");

        // Once it works, Docker comes first.
        fake(bin.path(), "docker", "exit 0");
        let Ok(Isolator::Containers(found)) = resolve(&auto, hub, Some(&search)).await else {
            panic!("Docker");
        };
        assert_eq!(found.engine.isolation(), Isolation::Docker);
        assert_eq!(found.hub, "http://host.docker.internal:8080");
    }
}
