//! Builds of a group's library, and their inspection.
//!
//! A worker with Lean keeps a checkout of each group's library in its work
//! folder (`libraries/<group>`), with its `.lake` between builds, so that
//! dependencies are fetched and built once. To build a commit it fetches
//! `main` from the hub with the job's token, checks the commit out, gets
//! Mathlib's cache if the library depends on Mathlib, runs `lake build`,
//! and then the inspection, `lean/Inspect.lean` in Stemma's repository,
//! with the library's own toolchain: `lake env lean --run`.

use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use stemma_core::api::LibraryReport;
use stemma_core::protocol::{Built, JobSpec, LibraryBuild, Outcome};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;

use crate::job::{Context, Progress, within};

/// The inspection program.
pub const INSPECT: &str = include_str!("../../../lean/Inspect.lean");

/// The most of a build's log sent to the hub, in bytes: its end.
const MAX_LOG: usize = 200_000;

/// What a build wrote, kept as it goes.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<String>>);

impl Log {
    fn line(&self, text: &str) {
        let mut log = self.0.lock().expect("not poisoned");
        log.push_str(text);
        log.push('\n');
        if log.len() > 2 * MAX_LOG {
            let mut cut = log.len() - MAX_LOG;
            while !log.is_char_boundary(cut) {
                cut += 1;
            }
            log.replace_range(..cut, "[…]\n");
        }
    }

    fn text(&self) -> String {
        let log = self.0.lock().expect("not poisoned");
        if log.len() <= MAX_LOG {
            return log.clone();
        }
        let mut cut = log.len() - MAX_LOG;
        while !log.is_char_boundary(cut) {
            cut += 1;
        }
        format!("[…]\n{}", &log[cut..])
    }
}

/// Reads lines from `reader` into the log, handing each to `each`.
async fn pump(reader: impl AsyncRead + Unpin, log: Log, each: impl Fn(&str)) {
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        log.line(&line);
        each(&line);
    }
}

/// Runs a command, writing what it says to the log; `each` sees every line
/// of its output.
async fn run(
    mut command: Command,
    log: &Log,
    each: impl Fn(&str) + Send + Sync + 'static,
) -> anyhow::Result<ExitStatus> {
    // The program and its arguments only: the environment holds the token.
    let std = command.as_std();
    let mut shown = std.get_program().to_string_lossy().into_owned();
    for arg in std.get_args() {
        shown.push(' ');
        shown.push_str(&arg.to_string_lossy());
    }
    log.line(&format!("$ {shown}"));
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let program = command
        .as_std()
        .get_program()
        .to_string_lossy()
        .into_owned();
    let mut child = command
        .spawn()
        .with_context(|| format!("running {program}"))?;
    let each = Arc::new(each);
    let out = {
        let (log, each) = (log.clone(), each.clone());
        tokio::spawn(pump(child.stdout.take().expect("piped"), log, move |l| {
            each(l)
        }))
    };
    let err = tokio::spawn(pump(
        child.stderr.take().expect("piped"),
        log.clone(),
        move |l| each(l),
    ));
    let status = child.wait().await?;
    let _ = out.await;
    let _ = err.await;
    Ok(status)
}

/// Git, with the job's token for the hub, and nothing asked of anyone.
fn git(dir: &Path, token: &str) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.extraHeader")
        .env(
            "GIT_CONFIG_VALUE_0",
            format!("Authorization: Bearer {token}"),
        );
    command
}

/// The modules a build made, from its `.olean` files.
pub fn built_modules(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, prefix: &str, found: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let module = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}.{name}")
            };
            if path.is_dir() {
                walk(&path, &module, found);
            } else if let Some(module) = module.strip_suffix(".olean") {
                found.push(module.to_owned());
            }
        }
    }
    let mut found = Vec::new();
    walk(&root.join(".lake/build/lib/lean"), "", &mut found);
    found.sort();
    found
}

/// Whether the library depends on Mathlib, as its Lake configuration says.
fn uses_mathlib(dir: &Path) -> bool {
    ["lakefile.toml", "lakefile.lean"].iter().any(|file| {
        std::fs::read_to_string(dir.join(file))
            .is_ok_and(|text| text.to_lowercase().contains("mathlib"))
    })
}

/// A short form of `lake build`'s progress: `[12/40] Geometry.Basic`.
fn build_progress(line: &str) -> Option<String> {
    let start = line.find('[')?;
    let rest = &line[start..];
    let close = rest.find(']')?;
    let step = &rest[..=close];
    let what = rest[close + 1..].trim().trim_start_matches("Built").trim();
    let what = what.split_whitespace().next().unwrap_or_default();
    Some(format!("Building {step} {what}").trim().to_owned())
}

/// Builds and inspects a commit of the library: a build that does not pass
/// is an outcome too, with its log.
pub async fn build(
    context: &Context,
    job: &JobSpec,
    build: &LibraryBuild,
    progress: &Progress,
) -> anyhow::Result<Outcome> {
    let config = context.config.lean.clone().unwrap_or_default();
    let minutes = config.timeout_minutes.unwrap_or(90);
    let log = Log::default();
    let dir = context.work_dir.join("libraries").join(&job.group.slug);
    let work = run_build(
        context,
        job,
        build,
        progress,
        &dir,
        &log,
        config.cache.unwrap_or(true),
    );
    let built = match within(minutes, "The build", work).await {
        Ok(built) => built,
        Err(error) => {
            log.line(&format!("{error:#}"));
            Built {
                succeeded: false,
                error: Some(format!("{error:#}")),
                log: String::new(),
                report: None,
            }
        }
    };
    Ok(Outcome::Built(Box::new(Built {
        log: log.text(),
        ..built
    })))
}

async fn run_build(
    context: &Context,
    job: &JobSpec,
    build: &LibraryBuild,
    progress: &Progress,
    dir: &Path,
    log: &Log,
    cache: bool,
) -> anyhow::Result<Built> {
    let failed = |error: &str| Built {
        succeeded: false,
        error: Some(error.to_owned()),
        log: String::new(),
        report: None,
    };
    let quiet = |_: &str| {};

    progress.say(format!(
        "Fetching {}",
        &build.commit[..build.commit.len().min(10)]
    ));
    std::fs::create_dir_all(dir)?;
    let url = format!("{}{}", context.hub, build.repository);
    if !dir.join(".git").exists() {
        let mut init = git(dir, &job.token);
        init.args(["init", "-q"]);
        anyhow::ensure!(run(init, log, quiet).await?.success(), "git init failed");
    }
    let mut remote = git(dir, &job.token);
    remote.args(["remote", "remove", "origin"]);
    let _ = run(remote, log, quiet).await;
    let mut remote = git(dir, &job.token);
    remote.args(["remote", "add", "origin", &url]);
    run(remote, log, quiet).await?;
    let mut fetch = git(dir, &job.token);
    fetch.args([
        "fetch",
        "-q",
        "--force",
        "origin",
        "+refs/heads/main:refs/remotes/origin/main",
    ]);
    if !run(fetch, log, quiet).await?.success() {
        return Ok(failed("Could not fetch the library from the hub"));
    }
    let mut checkout = git(dir, &job.token);
    checkout.args(["checkout", "-q", "--force", "--detach", &build.commit]);
    if !run(checkout, log, quiet).await?.success() {
        return Ok(failed("The commit is not on main"));
    }
    let mut clean = git(dir, &job.token);
    clean.args(["clean", "-q", "-fdx", "-e", ".lake"]);
    run(clean, log, quiet).await?;
    // The modules of an earlier build must not be taken for this one's.
    let _ = std::fs::remove_dir_all(dir.join(".lake/build/lib/lean"));

    if cache && uses_mathlib(dir) {
        progress.say("Getting Mathlib's cache");
        let mut get = Command::new("lake");
        get.current_dir(dir).args(["exe", "cache", "get"]);
        if !run(get, log, quiet).await?.success() {
            log.line("Mathlib's cache could not be got: building without it.");
        }
    }

    progress.say("Building");
    let mut lake = Command::new("lake");
    lake.current_dir(dir).arg("build");
    let progress_lines = progress.clone();
    let built = run(lake, log, move |line| {
        if let Some(step) = build_progress(line) {
            progress_lines.say(step);
        }
    })
    .await?;
    if !built.success() {
        return Ok(failed("lake build failed: see the log"));
    }

    progress.say("Inspecting");
    let modules = built_modules(dir);
    if modules.is_empty() {
        return Ok(failed("The build made no modules"));
    }
    let report = inspect(dir, &modules, log).await?;
    match report {
        Ok(report) => Ok(Built {
            succeeded: true,
            error: None,
            log: String::new(),
            report: Some(report),
        }),
        Err(error) => Ok(failed(&error)),
    }
}

/// Runs the inspection on the built library; its report, or why there is
/// none.
async fn inspect(
    dir: &Path,
    modules: &[String],
    log: &Log,
) -> anyhow::Result<Result<LibraryReport, String>> {
    let program: PathBuf = dir.join(".lake").join("stemma").join("Inspect.lean");
    std::fs::create_dir_all(program.parent().expect("in a folder"))?;
    std::fs::write(&program, INSPECT)?;
    let mut command = Command::new("lake");
    command
        .current_dir(dir)
        .args(["env", "lean", "--run"])
        .arg(&program)
        .args(modules)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    log.line(&format!(
        "$ lake env lean --run Inspect.lean {}",
        modules.join(" ")
    ));
    let output = command.output().await.context("running the inspection")?;
    for line in String::from_utf8_lossy(&output.stderr).lines() {
        log.line(line);
    }
    if !output.status.success() {
        return Ok(Err(format!(
            "The inspection failed ({}): see the log",
            output.status
        )));
    }
    match serde_json::from_slice::<LibraryReport>(&output.stdout) {
        Ok(report) => {
            log.line(&format!(
                "Inspected {} declarations in {} modules.",
                report.declarations.len(),
                report.modules.len()
            ));
            Ok(Ok(report))
        }
        Err(error) => Ok(Err(format!(
            "The inspection's report is unreadable: {error}"
        ))),
    }
}

/// Runs the inspection on a library already built in `dir`, for tests and
/// for people trying it.
pub async fn inspect_built(dir: &Path) -> anyhow::Result<LibraryReport> {
    let modules = built_modules(dir);
    anyhow::ensure!(!modules.is_empty(), "nothing is built in {}", dir.display());
    let log = Log::default();
    inspect(dir, &modules, &log)
        .await?
        .map_err(|error| anyhow::anyhow!("{error}\n{}", log.text()))
}

#[cfg(test)]
mod tests {
    use super::build_progress;

    #[test]
    fn reads_lakes_progress() {
        assert_eq!(
            build_progress("✔ [12/40] Built Geometry.Basic (1.2s)").as_deref(),
            Some("Building [12/40] Geometry.Basic")
        );
        assert_eq!(build_progress("Build completed successfully."), None);
    }
}
