//! Processes run in exclusive checkouts and containers, using the same runners
//! and sign-in mounts as answers, with a distinct leased process token.
use crate::{
    isolation::{self, Isolator, Sandbox},
    job::{Context, JobDir, Progress, Prompt},
    library::{self, Log},
};
use std::{path::PathBuf, sync::Arc, time::Duration};
use stemma_core::{
    api::{AgentProfile, ThreadKind},
    protocol::*,
};

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
struct Checkout(PathBuf);
impl Drop for Checkout {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
pub async fn run(context: Arc<Context>, spec: ProcessSpec) -> ProcessResult {
    let timeout = Duration::from_millis((spec.deadline - crate::approvals::now()).max(1) as u64);
    let result = tokio::time::timeout(timeout, execute(&context, &spec)).await;
    let result = match result {
        Ok(Ok(answer)) => ProcessResult {
            summary: answer.text,
            usage: answer.usage,
            ..Default::default()
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
async fn execute(context: &Context, spec: &ProcessSpec) -> anyhow::Result<crate::runners::Answer> {
    anyhow::ensure!(
        accepts(context, &spec.offer)?,
        "This machine no longer offers this runner and model"
    );
    let settings = stemma_client::config::Settings::load()?;
    let credentials = stemma_client::config::Credentials {
        hub: Some(context.hub.clone()),
        ..Default::default()
    };
    let mut config = crate::WorkerConfig::resolve(
        &settings,
        &context.config.paths,
        &credentials,
        Some(&spec.offer.group.slug),
    )?;
    let handle = &spec.offer.runner;
    let runner = runner(handle);
    let mut configured = config
        .runners
        .get(runner)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Runner is not configured"))?;
    if let Some(model) = &spec.offer.model {
        configured.model = Some(model.clone());
    }
    configured.timeout_minutes = spec
        .offer
        .recipe
        .recipe
        .as_ref()
        .map(|r| u64::from(r.front.timeout_minutes));
    config.runners.set(runner, configured);
    config.isolation.image = Some(
        settings
            .get("workshop.image", Some(&spec.offer.group.slug))?
            .value
            .as_str()
            .unwrap()
            .into(),
    );
    config.isolation.memory = Some(
        settings
            .get("workshop.memory", Some(&spec.offer.group.slug))?
            .value
            .as_str()
            .unwrap()
            .into(),
    );
    let cpus = settings
        .get("workshop.cpus", Some(&spec.offer.group.slug))?
        .value;
    config.isolation.cpus = cpus
        .as_float()
        .or_else(|| cpus.as_integer().map(|v| v as f64));
    let Isolator::Containers(containers) = isolation::resolve(
        &config.isolation,
        &context.hub,
        std::env::var_os("PATH").as_deref(),
    )
    .await
    .map_err(anyhow::Error::msg)?
    else {
        anyhow::bail!("The workshop requires isolation");
    };
    let agent =
        AgentProfile::default_for(handle).ok_or_else(|| anyhow::anyhow!("Unknown runner"))?;
    let job = JobSpec {
        process: true,
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
    let root = Checkout(
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
    std::fs::create_dir_all(&root.0)?;
    let log = Log::default();
    let seen_token = dir.token_file.clone();
    let mut repositories = Vec::new();
    for repo in &spec.repositories {
        let name = repo
            .path
            .rsplit('/')
            .next()
            .unwrap_or("")
            .trim_end_matches(".git");
        anyhow::ensure!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
            "Invalid process repository"
        );
        anyhow::ensure!(
            repo.branch
                == format!(
                    "workshop/{}-{}",
                    spec.offer.request.number, spec.offer.process
                ),
            "Invalid process branch"
        );
        let path = if name == "library" {
            root.0.join("library")
        } else {
            root.0
                .join("papers")
                .join(name.trim_start_matches("paper-"))
        };
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
        library::git_output(
            &path,
            &spec.token,
            &["checkout", "-B", &repo.branch, &repo.base],
        )
        .await?;
        library::git_output(
            &path,
            &spec.token,
            &[
                "config",
                "user.name",
                &format!("{}, for {}", agent.display_name, spec.owner.display_name),
            ],
        )
        .await?;
        library::git_output(
            &path,
            &spec.token,
            &["config", "user.email", "worker@stemma.invalid"],
        )
        .await?;
        // Only the token file's path is stored in git config, never its value.
        let helper = format!(
            "!f() {{ printf 'username=process\\npassword='; cat '{}'; printf '\\n'; }}; f",
            seen_token.display()
        );
        library::git_output(
            &path,
            &spec.token,
            &["config", "credential.helper", &helper],
        )
        .await?;
        library::git_output(
            &path,
            &spec.token,
            &[
                "remote",
                "set-url",
                "origin",
                &format!("{}{}", containers.hub, repo.path),
            ],
        )
        .await?;
        repositories.push((path, name == "library", repo));
    }
    let (env, mut files) = containers.sign_in(runner, dirs::home_dir().as_deref())?;
    let mut _held = None;
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
    let sandbox = containers.start(&container_spec, &root.0).await?;
    let mut contract = String::new();
    for (path, library, repo) in &repositories {
        if *library {
            crate::builds::seed_mathlib(&sandbox, path, &log).await?;
        }
        contract.push_str(&format!(
            "\n- {} at {}, branch {}, base {}",
            sandbox.path(path).display(),
            repo.path,
            repo.branch,
            repo.base
        ));
    }
    let prompt = Prompt {
        system: format!(
            "{}\n\nGroup: {}. Request: t{}, {}.\nCheckouts:{contract}",
            include_str!("../prompts/process.md"),
            spec.offer.group.slug,
            spec.offer.request.number,
            spec.offer.request.title
        ),
        user: spec.offer.prompt.clone(),
    };
    let work = crate::runners::Work {
        dir: &root.0,
        root: &root.0,
        sandbox: &sandbox,
        contained_program: containers.program(runner),
    };
    let (send, _receive) = tokio::sync::mpsc::unbounded_channel();
    let progress = Progress::new(spec.offer.process, send);
    let run_context = Context {
        hub: context.hub.clone(),
        stemma: context.stemma.clone(),
        work_dir: context.work_dir.clone(),
        config,
        isolator: None,
    };
    let result =
        crate::job::run_cli(&run_context, &job, &agent, &dir, &work, &prompt, &progress).await;
    let _ = crate::approvals::log(
        &context.config.paths.state,
        "checkout_log",
        serde_json::json!({"process":spec.offer.process,"log":stemma_core::secrets::hide(&log.text())}),
    );
    result
}
