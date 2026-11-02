//! A workshop process runs its recipe's `run.sh`, and nothing else, in a
//! folder of its own: in the machine's image or, if the machine runs its
//! jobs unisolated, on the machine itself.
//!
//! The folder holds `run.sh` and `files/`, with the request's inputs in
//! place; checkouts of the library and of each paper the inputs name, on
//! the process's branch; and `.stemma/`, what the `stemma` command reads
//! there. Everything a run needs of Stemma it does with that command:
//! `stemma agent run` launches the request's agent, and `stemma output`
//! delivers what the run produced. The script's exit says how it went.
use crate::{
    config::CliRunner,
    isolation::{self, HostSandbox, Isolator, Sandbox},
    job::{Context, JobDir, Progress, Prompt},
    library::{self, Log},
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use stemma_core::{
    api::{AgentProfile, ThreadKind, User},
    protocol::*,
    recipe::{self, Output},
    workshop::Delivered,
};

/// The variable that tells the `stemma` command where its process is.
pub const PROCESS: &str = "STEMMA_PROCESS";

/// What a process's own commands know of it, in `.stemma/process.json`,
/// with its paths as they see them.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessContext {
    pub process: i64,
    pub group: GroupRef,
    pub request: ThreadRef,
    pub requester: User,
    /// Whose machine runs it: its agents write for them.
    pub owner: User,
    pub hub: String,
    pub token_file: PathBuf,
    /// The process's folder.
    pub folder: PathBuf,
    /// The request's runner, by its handle, and how to run it.
    pub runner: String,
    pub runner_config: CliRunner,
    pub branch: Option<String>,
    /// The checkouts made for it, relative to its folder.
    pub checkouts: Vec<String>,
    pub outputs: Vec<Output>,
    pub permissions: Vec<String>,
    /// When it must end, in Unix milliseconds.
    pub deadline: i64,
}

impl ProcessContext {
    /// The process this command runs in, from [`PROCESS`].
    pub fn load() -> anyhow::Result<ProcessContext> {
        let path = std::env::var_os(PROCESS).ok_or_else(|| {
            anyhow::anyhow!("This runs only in a workshop process, from its recipe's run.sh")
        })?;
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }

    fn state(&self) -> PathBuf {
        self.folder.join(".stemma")
    }
}

pub fn runner(handle: &str) -> &str {
    if handle == "claude" {
        "claude-code"
    } else {
        handle
    }
}
pub fn accepts(context: &Context, offer: &ProcessOffer) -> anyhow::Result<bool> {
    let settings = stemma_client::config::Settings::load()?;
    let scope = crate::builds::capabilities(&settings, Some(&offer.group.slug))?;
    let Some(model) = scope.runners.get(&offer.runner) else {
        return Ok(false);
    };
    Ok(scope.enabled
        && scope.concurrency > 0
        && offer.model.as_ref().is_none_or(|m| m == model)
        && offer
            .recipe
            .recipe
            .as_ref()
            .is_some_and(|r| r.accepts_version(env!("CARGO_PKG_VERSION")))
        && offer.expires_at > crate::approvals::now()
        && !context.hub.is_empty())
}
struct Folder(PathBuf);
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What a run left: what it delivered and consumed, and why it failed.
struct Ran {
    error: Option<String>,
    outputs: Vec<Delivered>,
    usage: Usage,
}

pub async fn run(context: Arc<Context>, spec: ProcessSpec) -> ProcessResult {
    let timeout = Duration::from_millis((spec.deadline - crate::approvals::now()).max(1) as u64);
    let result = tokio::time::timeout(timeout, execute(&context, &spec)).await;
    let result = match result {
        Ok(Ok(ran)) => ProcessResult {
            error: ran.error,
            outputs: ran.outputs,
            usage: ran.usage,
        },
        Ok(Err(e)) => ProcessResult {
            error: Some(format!("{e:#}")),
            ..Default::default()
        },
        Err(_) => ProcessResult {
            error: Some("The process exceeded its deadline".into()),
            ..Default::default()
        },
    };
    let _ = crate::approvals::log(
        &context.config.paths.state,
        "finished",
        serde_json::json!({"hub":context.hub,"process":spec.offer.process,"error":result.error.as_deref().map(stemma_core::secrets::hide),"usage":result.usage}),
    );
    result
}

/// Writes the recipe's folder, with the inputs in place, into `root`.
fn write_recipe(root: &Path, offer: &ProcessOffer) -> anyhow::Result<()> {
    let recipe = offer
        .recipe
        .recipe
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("The recipe cannot run"))?;
    for (path, bytes) in recipe.render(&offer.recipe.name, &offer.inputs)? {
        let relative = Path::new(&path);
        anyhow::ensure!(
            relative
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
            "The recipe's file {path} is outside its folder"
        );
        let target = root.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, bytes)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            root.join(recipe::ENTRY),
            std::fs::Permissions::from_mode(0o755),
        )?;
    }
    Ok(())
}

async fn execute(context: &Context, spec: &ProcessSpec) -> anyhow::Result<Ran> {
    anyhow::ensure!(
        accepts(context, &spec.offer)?,
        "This machine no longer offers this runner and model"
    );
    let settings = stemma_client::config::Settings::load()?;
    let credentials = stemma_client::config::Credentials {
        hub: Some(context.hub.clone()),
        ..Default::default()
    };
    let config = crate::WorkerConfig::resolve(
        &settings,
        &context.config.paths,
        &credentials,
        Some(&spec.offer.group.slug),
    )?;
    let handle = &spec.offer.runner;
    let runner = runner(handle);
    let mut runner_config = config
        .runners
        .get(runner)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Runner is not configured"))?;
    if let Some(model) = &spec.offer.model {
        runner_config.model = Some(model.clone());
    }
    let isolator = isolation::resolve(
        &config.isolation,
        &context.hub,
        std::env::var_os("PATH").as_deref(),
    )
    .await
    .map_err(anyhow::Error::msg)?;
    let contained = matches!(isolator, Isolator::Containers(_));
    // The image runs its runners by their usual names.
    if contained {
        runner_config.command = None;
    }
    let host = context.host();
    let agent =
        AgentProfile::default_for(handle).ok_or_else(|| anyhow::anyhow!("Unknown runner"))?;
    let recipe = spec
        .offer
        .recipe
        .recipe
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("The recipe cannot run"))?;
    let job = JobSpec {
        process: true,
        permissions: recipe
            .manifest
            .permissions
            .iter()
            .map(|p| p.tool().to_owned())
            .collect(),
        id: spec.offer.process,
        group: spec.offer.group.clone(),
        token: spec.token.clone(),
        work: Work::Reply(Box::new(Conversation {
            agent: agent.clone(),
            thread: ThreadRef {
                kind: ThreadKind::Request,
                ..spec.offer.request.clone()
            },
            trigger: 0,
            requester: spec.offer.requester.clone(),
            context: vec![],
        })),
    };
    let dir = JobDir::create(&context.work_dir, &job)?;
    let root = Folder(
        settings
            .workspace(&spec.offer.group.slug)?
            .join("processes")
            .join(format!(
                "{}-{}-{:08x}",
                spec.offer.process,
                spec.lease.attempt,
                rand::random::<u32>()
            )),
    );
    std::fs::create_dir_all(root.0.join(".stemma/config"))?;
    write_recipe(&root.0, &spec.offer)?;
    let log = Log::default();
    let mut checkouts = Vec::new();
    for repo in &spec.checkouts {
        let name = repo.name.as_str();
        anyhow::ensure!(
            name == "library"
                || name.strip_prefix("paper-").is_some_and(|slug| {
                    !slug.is_empty()
                        && slug
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                }),
            "Invalid process repository"
        );
        let relative = match name.strip_prefix("paper-") {
            Some(slug) => format!("papers/{slug}"),
            None => "library".to_owned(),
        };
        let path = root.0.join(&relative);
        library::fetch(
            &path,
            &format!("{}{}", context.hub, repo.path),
            &spec.token,
            &log,
        )
        .await?;
        library::git_output(
            &path,
            &spec.token,
            &["fetch", "--no-tags", "origin", &repo.base],
        )
        .await?;
        match &spec.branch {
            Some(branch) => {
                anyhow::ensure!(
                    *branch
                        == format!(
                            "workshop/{}-{}",
                            spec.offer.request.number, spec.offer.process
                        ),
                    "Invalid process branch"
                );
                library::git_output(&path, &spec.token, &["checkout", "-B", branch, &repo.base])
                    .await?;
            }
            None => {
                library::git_output(&path, &spec.token, &["checkout", "--detach", &repo.base])
                    .await?;
            }
        }
        checkouts.push((path, name == "library", relative, repo.path.clone()));
    }
    let mut container = None;
    // The agent's sign-in for containers, held while it works.
    let mut _held = None;
    if let Isolator::Containers(containers) = &isolator {
        let (env, mut files) = containers.sign_in(runner, dirs::home_dir().as_deref())?;
        if let Some(handle) = crate::sign_in::handle_of(runner) {
            files.extend(crate::sign_in::mounts(&context.config.paths, handle));
            _held = crate::sign_in::hold(&context.config.paths, handle).await?;
        }
        let mut container_spec = containers.spec(
            spec.offer.process,
            isolation::owner_of(&root.0),
            crate::job::answer_mounts(&root.0, &dir.path, files),
            env,
            Duration::from_millis((spec.deadline - crate::approvals::now()).max(1) as u64)
                + Duration::from_secs(60),
        );
        container_spec.pass_values = crate::sign_in::values(&container_spec.pass_env);
        container = Some(containers.start(&container_spec, &root.0).await?);
    }
    let sandbox: &dyn Sandbox = match &container {
        Some(container) => container,
        None => &host,
    };
    for (path, library, _, hub_path) in &checkouts {
        if *library {
            crate::builds::seed_mathlib(sandbox, path, &log).await?;
        }
        // The hub, as the process reaches it.
        library::git_output(
            path,
            &spec.token,
            &[
                "remote",
                "set-url",
                "origin",
                &format!("{}{hub_path}", sandbox.hub()),
            ],
        )
        .await?;
    }
    let seen_folder = sandbox.path(&root.0);
    let seen_token = sandbox.path(&dir.token_file);
    let state = root.0.join(".stemma");
    // Git signs in to the hub with the process's token, for its checkouts
    // and any other of the group's repositories it clones.
    std::fs::write(
        state.join("gitconfig"),
        format!(
            "[user]\n\tname = {}, for {}\n\temail = worker@stemma.invalid\n\
             [credential \"{}\"]\n\thelper = \"!f() {{ printf 'username=process\\\\npassword='; cat '{}'; printf '\\\\n'; }}; f\"\n",
            agent.display_name,
            spec.owner.display_name,
            sandbox.hub(),
            seen_token.display()
        ),
    )?;
    let process_context = ProcessContext {
        process: spec.offer.process,
        group: spec.offer.group.clone(),
        request: spec.offer.request.clone(),
        requester: spec.offer.requester.clone(),
        owner: spec.owner.clone(),
        hub: sandbox.hub().to_owned(),
        token_file: seen_token.clone(),
        folder: seen_folder.clone(),
        runner: handle.clone(),
        runner_config,
        branch: spec.branch.clone(),
        checkouts: checkouts.iter().map(|(_, _, r, _)| r.clone()).collect(),
        outputs: recipe.outputs(),
        permissions: job.permissions.clone(),
        deadline: spec.deadline,
    };
    std::fs::write(
        state.join("process.json"),
        serde_json::to_vec_pretty(&process_context)?,
    )?;
    // The `stemma` command signs in as the process, and nobody else.
    stemma_client::config::Credentials {
        hub: Some(sandbox.hub().to_owned()),
        token: Some(spec.token.clone()),
        ..Default::default()
    }
    .save_to(&state.join("config/credentials.toml"))?;
    let seen_state = seen_folder.join(".stemma");
    let mut env = vec![
        (
            PROCESS.to_owned(),
            seen_state.join("process.json").display().to_string(),
        ),
        (
            "GIT_CONFIG_GLOBAL".to_owned(),
            seen_state.join("gitconfig").display().to_string(),
        ),
        (
            "STEMMA_CONFIG_DIR".to_owned(),
            seen_state.join("config").display().to_string(),
        ),
        (
            "STEMMA_STATE_DIR".to_owned(),
            seen_state.join("state").display().to_string(),
        ),
        ("STEMMA_GROUP".to_owned(), spec.offer.group.slug.clone()),
    ];
    // On the machine itself, its own `stemma` comes first.
    if !contained && let Some(bin) = context.stemma.parent() {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![bin.to_owned()];
        paths.extend(std::env::split_paths(&path));
        env.push((
            "PATH".to_owned(),
            std::env::join_paths(paths)?.to_string_lossy().into_owned(),
        ));
    }
    let command = sandbox.command(
        "sh",
        &["-c".to_owned(), format!("exec ./{}", recipe::ENTRY)],
        &env,
        &root.0,
    );
    let status = library::run_shown(command, recipe::ENTRY, &log, |_| {}).await;
    let _ = crate::approvals::log(
        &context.config.paths.state,
        "process_log",
        serde_json::json!({"process":spec.offer.process,"log":stemma_core::secrets::hide(&log.text())}),
    );
    let error = match status {
        Ok(status) if status.success() => None,
        Ok(status) => Some(format!(
            "run.sh ended with {status}. Its last lines:\n\n{}",
            last_lines(&log.text(), 20)
        )),
        Err(error) => Some(format!("{error:#}")),
    };
    Ok(Ran {
        error,
        outputs: read_lines(&state.join("outputs.jsonl"))?,
        usage: read_lines::<Usage>(&state.join("usage.jsonl"))?
            .into_iter()
            .fold(Usage::default(), add),
    })
}

/// The last `count` lines of a log.
fn last_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(count)..].join("\n")
}

/// Each line of a JSON-lines file; none if there is no file.
fn read_lines<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<Vec<T>> {
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| Ok(serde_json::from_str(l)?))
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}

/// Appends `value` to a JSON-lines file.
fn append_line(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", serde_json::to_string(value)?)?;
    Ok(())
}

fn add(total: Usage, more: Usage) -> Usage {
    let sum = |a: Option<u64>, b: Option<u64>| match (a, b) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
    };
    Usage {
        input_tokens: sum(total.input_tokens, more.input_tokens),
        output_tokens: sum(total.output_tokens, more.output_tokens),
        cache_read_tokens: sum(total.cache_read_tokens, more.cache_read_tokens),
        cache_write_tokens: sum(total.cache_write_tokens, more.cache_write_tokens),
        cost_usd: match (total.cost_usd, more.cost_usd) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
        },
    }
}

/// Stemma's instructions to a process's agent: what it may do, and how it
/// delivers.
fn contract(context: &ProcessContext) -> String {
    let outputs = if context.outputs.is_empty() {
        "nothing: this recipe delivers no outputs".to_owned()
    } else {
        context
            .outputs
            .iter()
            .map(|o| o.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let checkouts = if context.checkouts.is_empty() {
        "none".to_owned()
    } else {
        context.checkouts.join(", ")
    };
    include_str!("../prompts/process.md")
        .replace("{group}", &context.group.slug)
        .replace("{request}", &context.request.number.to_string())
        .replace("{title}", &context.request.title)
        .replace("{folder}", &context.folder.display().to_string())
        .replace("{checkouts}", &checkouts)
        .replace(
            "{branch}",
            context
                .branch
                .as_deref()
                .unwrap_or("none: this recipe opens no pull requests"),
        )
        .replace("{outputs}", &outputs)
}

/// `stemma agent run`: the request's agent works on the prompt in `file`,
/// in the process's folder; what it answered.
pub async fn agent_run(file: &Path, model: Option<String>) -> anyhow::Result<String> {
    let context = ProcessContext::load()?;
    let user = std::fs::read_to_string(file)
        .map_err(|e| anyhow::anyhow!("Reading the prompt {}: {e}", file.display()))?;
    let agent = AgentProfile::default_for(&context.runner)
        .ok_or_else(|| anyhow::anyhow!("Unknown runner {}", context.runner))?;
    let mut config = context.runner_config.clone();
    if model.is_some() {
        config.model = model;
    }
    let token = std::fs::read_to_string(&context.token_file)?
        .trim()
        .to_owned();
    let job = JobSpec {
        process: true,
        permissions: context.permissions.clone(),
        id: context.process,
        group: context.group.clone(),
        token,
        work: Work::Reply(Box::new(Conversation {
            agent: agent.clone(),
            thread: context.request.clone(),
            trigger: 0,
            requester: context.requester.clone(),
            context: vec![],
        })),
    };
    let path = context
        .state()
        .join(format!("agent-{:08x}", rand::random::<u32>()));
    std::fs::create_dir_all(&path)?;
    let dir = JobDir {
        path,
        token_file: context.token_file.clone(),
    };
    let sandbox = HostSandbox {
        hub: context.hub.clone(),
        stemma: std::env::current_exe()?,
    };
    let cwd = std::env::current_dir()?;
    let work = crate::runners::Work {
        dir: &cwd,
        sandbox: &sandbox,
    };
    let prompt = Prompt {
        system: contract(&context),
        user,
    };
    let (send, mut receive) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some((_, text)) = receive.recv().await {
            eprintln!("· {text}");
        }
    });
    let progress = Progress::new(context.process, send);
    let minutes = ((context.deadline - crate::approvals::now()) / 60_000).max(1) as u64;
    let answer = crate::job::within(
        minutes,
        &agent.display_name,
        crate::job::run_agent(&config, &job, &agent, &dir, &work, &prompt, &progress),
    )
    .await?;
    append_line(&context.state().join("usage.jsonl"), &answer.usage)?;
    Ok(answer.text)
}

/// `stemma output`: delivers `output` when the process ends, if its recipe
/// admits it.
pub fn deliver(output: Delivered) -> anyhow::Result<()> {
    let context = ProcessContext::load()?;
    let kind = match &output {
        Delivered::Message { text } => {
            anyhow::ensure!(!text.trim().is_empty(), "The message is empty");
            Output::Message
        }
        Delivered::PullRequest { repository, .. } => {
            anyhow::ensure!(
                repository == "library"
                    || repository
                        .strip_prefix("paper-")
                        .is_some_and(|s| !s.is_empty()),
                "A pull request goes to library, or to paper-<slug>"
            );
            Output::PullRequest
        }
    };
    anyhow::ensure!(
        context.outputs.contains(&kind),
        "This recipe does not deliver a {}",
        kind.as_str().replace('_', " ")
    );
    append_line(&context.state().join("outputs.jsonl"), &output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_adds_up_across_agents() {
        let a = Usage {
            input_tokens: Some(10),
            cost_usd: Some(0.5),
            ..Default::default()
        };
        let b = Usage {
            input_tokens: Some(5),
            output_tokens: Some(3),
            ..Default::default()
        };
        let total = add(add(Usage::default(), a), b);
        assert_eq!(total.input_tokens, Some(15));
        assert_eq!(total.output_tokens, Some(3));
        assert_eq!(total.cost_usd, Some(0.5));
        assert_eq!(total.cache_read_tokens, None);
    }

    #[test]
    fn writes_the_recipe_with_its_inputs_in_place() {
        let folder = tempfile::tempdir().unwrap();
        let files = std::collections::BTreeMap::from([
            (
                recipe::MANIFEST.to_owned(),
                b"title = \"T\"\nsummary = \"S\"\ntimeout_minutes = 5\n[inputs.task]\ndescription = \"d\"\nkind = \"text\"\n".to_vec(),
            ),
            (recipe::ENTRY.to_owned(), b"echo {{task}}\n".to_vec()),
            ("files/a/b.md".to_owned(), b"Do {{task}}.".to_vec()),
        ]);
        let recipe = stemma_core::recipe::Recipe::parse("t", files).unwrap();
        let offer = ProcessOffer {
            process: 1,
            group: GroupRef {
                id: 1,
                slug: "g".into(),
                name: "G".into(),
            },
            request: ThreadRef {
                number: 2,
                title: "R".into(),
                kind: ThreadKind::Request,
            },
            requester: User {
                id: 1,
                username: "ada".into(),
                display_name: "Ada".into(),
            },
            recipe: stemma_core::workshop::RecipeFile {
                name: "t".into(),
                version: "v".into(),
                commit: "c".into(),
                url: String::new(),
                recipe: Some(recipe),
                error: None,
            },
            inputs: [("task".into(), "it; ls".into())].into(),
            runner: "codex".into(),
            model: None,
            assigned_by: User {
                id: 1,
                username: "ada".into(),
                display_name: "Ada".into(),
            },
            recurring: false,
            expires_at: 0,
        };
        write_recipe(folder.path(), &offer).unwrap();
        assert_eq!(
            std::fs::read_to_string(folder.path().join("run.sh")).unwrap(),
            "echo it; ls\n"
        );
        assert_eq!(
            std::fs::read_to_string(folder.path().join("files/a/b.md")).unwrap(),
            "Do it; ls."
        );
        assert!(!folder.path().join("recipe.toml").exists());
    }
}
