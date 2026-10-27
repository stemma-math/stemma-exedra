//! Build execution has no access to model sign-ins or container credentials.
use crate::{
    isolation::{self, Containers, Isolator, Mount, Sandbox},
    job::{Context, Progress},
    library::{self, Log},
};
use std::{path::Path, sync::Arc, time::Duration};
use stemma_client::config::Settings;
use stemma_core::protocol::{BuildKind, BuildResult, BuildSpec, GroupWorkshop};

pub fn capabilities(settings: &Settings, group: Option<&str>) -> anyhow::Result<GroupWorkshop> {
    let isolated = settings.get("agents.isolation", group)?.value.as_str() != Some("none");
    let get = |key: &str| settings.get(key, group).map(|v| v.value);
    Ok(GroupWorkshop {
        enabled: isolated && get("workshop.enabled")?.as_bool().unwrap(),
        build: isolated && get("workshop.build")?.as_bool().unwrap(),
        build_concurrency: get("workshop.build_concurrency")?.as_integer().unwrap() as u32,
        concurrency: get("workshop.concurrency")?.as_integer().unwrap() as u32,
        runners: {
            let configured = get("workshop.runners")?;
            ["claude", "codex", "opencode"]
                .into_iter()
                .filter(|r| {
                    configured
                        .as_array()
                        .is_none_or(|a| a.is_empty() || a.iter().any(|v| v.as_str() == Some(r)))
                })
                .map(|r| {
                    Ok((
                        r.into(),
                        get(&format!("agents.{r}.model"))?.as_str().unwrap().into(),
                    ))
                })
                .collect::<anyhow::Result<_>>()?
        },
        others: if get("workshop.others")?.as_str() == Some("never") {
            stemma_core::protocol::Others::Never
        } else {
            stemma_core::protocol::Others::Ask
        },
    })
}

/// Builds, or declines with why when this machine cannot build now, so
/// that the hub offers the build to another machine.
pub async fn run(context: &Context, spec: &BuildSpec) -> Result<BuildResult, String> {
    let (settings, group, containers) = prepare(context, spec)
        .await
        .map_err(|error| format!("{error:#}"))?;
    let log = Log::default();
    let result = tokio::time::timeout(
        Duration::from_secs(90 * 60),
        execute(context, spec, &settings, &group, &containers, &log),
    )
    .await;
    let mut result = match result {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => library::failed(&format!("{error:#}")),
        Err(_) => library::failed("The build exceeded 90 minutes"),
    };
    result.log = log.text();
    Ok(result)
}

/// What a build needs before it starts: the group's settings, and containers
/// with the workshop image.
async fn prepare(
    context: &Context,
    spec: &BuildSpec,
) -> anyhow::Result<(Settings, String, Arc<Containers>)> {
    let settings = Settings::load()?;
    let group = settings_group(context, spec)?;
    let capabilities = capabilities(&settings, Some(&group))?;
    anyhow::ensure!(
        capabilities.enabled && capabilities.build,
        "This machine does not build for this group; the workshop requires isolation"
    );
    let credentials = stemma_client::config::Credentials {
        hub: Some(context.hub.clone()),
        ..Default::default()
    };
    let config =
        crate::WorkerConfig::resolve(&settings, &context.config.paths, &credentials, Some(&group))?;
    let mut isolated = config.isolation.clone();
    isolated.image = Some(
        settings
            .get("workshop.image", Some(&group))?
            .value
            .as_str()
            .unwrap()
            .into(),
    );
    isolated.memory = Some(
        settings
            .get("workshop.memory", Some(&group))?
            .value
            .as_str()
            .unwrap()
            .into(),
    );
    let cpus = settings.get("workshop.cpus", Some(&group))?.value;
    isolated.cpus = cpus
        .as_float()
        .or_else(|| cpus.as_integer().map(|n| n as f64));
    isolated.agents.clear();
    let Isolator::Containers(containers) =
        isolation::resolve(&isolated, &context.hub, std::env::var_os("PATH").as_deref())
            .await
            .map_err(anyhow::Error::msg)?
    else {
        anyhow::bail!("The workshop requires isolation");
    };
    Ok((settings, group, containers))
}

async fn execute(
    context: &Context,
    spec: &BuildSpec,
    settings: &Settings,
    group: &str,
    containers: &Containers,
    log: &Log,
) -> anyhow::Result<BuildResult> {
    let name = spec
        .repository
        .rsplit('/')
        .next()
        .unwrap()
        .trim_end_matches(".git");
    anyhow::ensure!(
        !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)),
        "Invalid build repository"
    );
    let dir = settings.workspace(group)?.join("builds").join(name);
    let locks = context.config.paths.state.join("build-locks");
    std::fs::create_dir_all(&locks)?;
    let held = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(locks.join(format!("{}-{name}", spec.group.id)))?;
    loop {
        match held.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) => {
                tokio::time::sleep(Duration::from_millis(100)).await
            }
            Err(error) => return Err(error.into()),
        }
    }
    library::fetch(
        &dir,
        &format!("{}{}", context.hub, spec.repository),
        &spec.token,
        log,
    )
    .await?;
    // Fetch exactly the requested commit too: future PR builds need not be main.
    let mut fetch = library::git(&dir, &spec.token);
    fetch.args(["fetch", "--no-tags", "origin", &spec.commit]);
    anyhow::ensure!(
        library::quietly(fetch, log).await?,
        "Fetching the build commit failed"
    );
    library::git_output(
        &dir,
        &spec.token,
        &["checkout", "--detach", "--force", &spec.commit],
    )
    .await?;
    library::git_output(&dir, &spec.token, &["clean", "-ffd"]).await?;
    let inspect = library::inspection_root(context, spec.group.id);
    std::fs::create_dir_all(&inspect)?;
    let seen = |path: &Path| isolation::container_path(&context.work_dir, path);
    let mut mounts = vec![
        Mount::Bind {
            source: dir.clone(),
            target: seen(&dir),
            read_only: false,
        },
        Mount::Hidden {
            target: seen(&dir.join(".git")),
        },
    ];
    if spec.kind == BuildKind::Library {
        mounts.push(Mount::Bind {
            source: inspect.clone(),
            target: seen(&inspect),
            read_only: false,
        });
    }
    // No sign-in mounts, pass_env or pass_values: only the workspace and inspection cache.
    let container_spec = containers.spec(
        spec.lease.id,
        isolation::owner_of(&context.work_dir),
        mounts,
        vec![],
        Duration::from_secs(90 * 60 + 300),
    );
    let sandbox = containers.start(&container_spec, &context.work_dir).await?;
    let (send, _receive) = tokio::sync::mpsc::unbounded_channel();
    let progress = Progress::new(spec.lease.id, send);
    match spec.kind {
        BuildKind::Library => {
            let seeded = seed_mathlib(&sandbox, &dir, log).await?;
            library::build_and_inspect(&inspect, &sandbox, &dir, log, &progress, !seeded).await
        }
        BuildKind::Paper => {
            if !crate::papers::latexmk(&sandbox, &dir, "main.tex", log, &progress).await? {
                return Ok(library::failed(&crate::papers::errors_of(&log.text())));
            }
            let pdf = crate::papers::read_pdf(&dir, "main.tex")?;
            let client = reqwest::Client::new();
            client
                .post(format!(
                    "{}/api/v1/builds/{}/pdf",
                    context.hub, spec.lease.id
                ))
                .bearer_auth(&spec.token)
                .header("content-type", "application/pdf")
                .body(pdf)
                .send()
                .await?
                .error_for_status()?;
            Ok(BuildResult {
                succeeded: true,
                ..Default::default()
            })
        }
    }
}
fn settings_group(context: &Context, spec: &BuildSpec) -> anyhow::Result<String> {
    context
        .config
        .paths
        .group_slug(&context.hub, spec.group.id, &spec.group.slug)
}

/// Seed the template's pinned Mathlib and its dependencies from the image.
pub(crate) async fn seed_mathlib(
    sandbox: &dyn Sandbox,
    dir: &Path,
    log: &Log,
) -> anyhow::Result<bool> {
    // The build folder outlives the image: links an earlier build made to
    // its Mathlib go first, so that another Mathlib is never built on it.
    let unlink = "for p in .lake/packages/*; do if [ -L \"$p\" ]; then case \"$(readlink \"$p\")\" in /opt/stemma/mathlib*) rm \"$p\" ;; esac; fi; done; true";
    anyhow::ensure!(
        library::run_in(sandbox, "sh", &["-c", unlink], dir, log, |_| {})
            .await?
            .success(),
        "Removing an earlier Mathlib's links failed"
    );
    let manifest = std::fs::read(dir.join("lake-manifest.json")).unwrap_or_default();
    let requested: serde_json::Value = serde_json::from_slice(&manifest).unwrap_or_default();
    let Some(rev) = requested["packages"]
        .as_array()
        .and_then(|p| p.iter().find(|p| p["name"] == "mathlib"))
        .and_then(|p| p["rev"].as_str())
    else {
        return Ok(false);
    };
    if !library::run_in(
        sandbox,
        "sh",
        &[
            "-c",
            "test \"$(git -C /opt/stemma/mathlib rev-parse HEAD)\" = \"$1\"",
            "sh",
            rev,
        ],
        dir,
        log,
        |_| {},
    )
    .await?
    .success()
    {
        return Ok(false);
    }
    let script = "mkdir -p .lake/packages; for p in /opt/stemma/mathlib/.lake/packages/*; do name=${p##*/}; if [ ! -e \".lake/packages/$name\" ] && [ ! -L \".lake/packages/$name\" ]; then ln -s \"$p\" \".lake/packages/$name\"; fi; done; if [ ! -e .lake/packages/mathlib ] && [ ! -L .lake/packages/mathlib ]; then ln -s /opt/stemma/mathlib .lake/packages/mathlib; fi";
    anyhow::ensure!(
        library::run_in(sandbox, "sh", &["-c", script], dir, log, |_| {})
            .await?
            .success(),
        "Seeding Mathlib's cache failed"
    );
    Ok(true)
}
