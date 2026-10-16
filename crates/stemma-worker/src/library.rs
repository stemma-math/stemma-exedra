//! Builds of a group's library, and their inspection.
//!
//! A worker with Lean keeps a warm checkout of each group's library in its
//! work folder (`libraries/<group>`), with its `.lake` between builds. To
//! build a commit it fetches `main` from the hub with the job's token and
//! checks the commit out; gets Mathlib's cache, if the library depends on
//! Mathlib and its dependencies changed since the last time; and runs
//! `lake build`, which builds again only the modules that changed.
//!
//! Then it runs the inspection, `lean/Inspect.lean` in Stemma's repository,
//! with the library's own toolchain: compiled once per toolchain into a
//! program (`inspect/<toolchain>` in the work folder), or interpreted if it
//! cannot be compiled. It inspects only the modules whose build changed and
//! those that import them, and keeps what it found of the others from the
//! last time (`.lake/stemma/inspection.json`): a commit that changes no
//! module is not inspected at all, and Mathlib is imported once, when one
//! is.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use stemma_core::api::{Declaration, LibraryReport};
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
pub struct Log(Arc<Mutex<String>>);

impl Log {
    pub fn line(&self, text: &str) {
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

    pub fn text(&self) -> String {
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

    /// What it wrote since `start`, a length of its text before.
    pub fn since(&self, start: usize) -> String {
        let log = self.0.lock().expect("not poisoned");
        let mut start = start.min(log.len());
        while !log.is_char_boundary(start) {
            start += 1;
        }
        log[start..].to_owned()
    }

    pub fn len(&self) -> usize {
        self.0.lock().expect("not poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
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
pub async fn run(
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

/// Runs a command quietly into the log; whether it succeeded.
pub async fn quietly(command: Command, log: &Log) -> anyhow::Result<bool> {
    Ok(run(command, log, |_| {}).await?.success())
}

/// Git, with the job's token for the hub, and nothing asked of anyone.
pub fn git(dir: &Path, token: &str) -> Command {
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

/// What git says, trimmed, if it succeeds.
pub async fn git_output(dir: &Path, token: &str, args: &[&str]) -> anyhow::Result<String> {
    let output = git(dir, token)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .context("running git")?;
    anyhow::ensure!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
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

/// The library's modules: those built whose source is in the checkout, so
/// that a module whose file is gone is not taken for one of the library.
/// A library whose sources are elsewhere (`srcDir`) keeps all those built.
pub fn library_modules(root: &Path) -> Vec<String> {
    let built = built_modules(root);
    let with_source: Vec<String> = built
        .iter()
        .filter(|module| {
            let path = module.replace('.', "/");
            root.join(format!("{path}.lean")).is_file()
        })
        .cloned()
        .collect();
    let elsewhere = ["lakefile.toml", "lakefile.lean"].iter().any(|file| {
        std::fs::read_to_string(root.join(file)).is_ok_and(|text| text.contains("srcDir"))
    });
    if with_source.is_empty() && elsewhere {
        built
    } else {
        with_source
    }
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

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Where Stemma keeps what it knows of a checkout: in its `.lake`, which
/// git leaves alone.
fn stemma_dir(dir: &Path) -> PathBuf {
    dir.join(".lake").join("stemma")
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
    let dir = checkout_dir(context, &job.group.slug);
    let lock = context.checkout_lock(&job.group.slug);
    let work = async {
        let _held = lock.lock().await;
        progress.say(format!(
            "Fetching {}",
            &build.commit[..build.commit.len().min(10)]
        ));
        let url = format!("{}{}", context.hub, build.repository);
        if let Err(error) = fetch(&dir, &url, &job.token, &log).await {
            return Ok(failed(&format!("{error:#}")));
        }
        if !checkout(&dir, &build.commit, &job.token, &log).await? {
            return Ok(failed("The commit is not on main"));
        }
        build_and_inspect(context, &dir, &log, progress, config.cache.unwrap_or(true)).await
    };
    let built = match within(minutes, "The build", work).await {
        Ok(built) => built,
        Err(error) => {
            log.line(&format!("{error:#}"));
            failed(&format!("{error:#}"))
        }
    };
    Ok(Outcome::Built(Box::new(Built {
        log: log.text(),
        ..built
    })))
}

/// A build that did not pass, for `error`.
pub fn failed(error: &str) -> Built {
    Built {
        succeeded: false,
        error: Some(error.to_owned()),
        log: String::new(),
        report: None,
    }
}

/// The warm checkout of a group's library.
pub fn checkout_dir(context: &Context, group: &str) -> PathBuf {
    context.work_dir.join("libraries").join(group)
}

/// Brings the warm checkout's `origin/main` to the hub's, making the
/// checkout if there is none.
pub async fn fetch(dir: &Path, url: &str, token: &str, log: &Log) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    if !dir.join(".git").exists() {
        let mut init = git(dir, token);
        init.args(["init", "-q"]);
        anyhow::ensure!(quietly(init, log).await?, "git init failed");
    }
    let mut remove = git(dir, token);
    remove.args(["remote", "remove", "origin"]);
    let _ = quietly(remove, log).await;
    let mut add = git(dir, token);
    add.args(["remote", "add", "origin", url]);
    quietly(add, log).await?;
    let mut fetch = git(dir, token);
    fetch.args([
        "fetch",
        "-q",
        "--force",
        "origin",
        "+refs/heads/main:refs/remotes/origin/main",
    ]);
    anyhow::ensure!(
        quietly(fetch, log).await?,
        "Could not fetch the library from the hub"
    );
    Ok(())
}

/// Checks `commit` out in the warm checkout, leaving `.lake` as it is;
/// whether it could.
pub async fn checkout(dir: &Path, commit: &str, token: &str, log: &Log) -> anyhow::Result<bool> {
    let mut checkout = git(dir, token);
    checkout.args(["checkout", "-q", "--force", "--detach", commit]);
    if !quietly(checkout, log).await? {
        return Ok(false);
    }
    let mut clean = git(dir, token);
    clean.args(["clean", "-q", "-fdx", "-e", ".lake"]);
    quietly(clean, log).await?;
    Ok(true)
}

/// Gets Mathlib's cache, if the library depends on Mathlib and its
/// dependencies or toolchain changed since the cache was last got here.
pub async fn get_cache(dir: &Path, log: &Log, progress: &Progress) -> anyhow::Result<()> {
    if !uses_mathlib(dir) {
        return Ok(());
    }
    let mut seen = Vec::new();
    for file in [
        "lean-toolchain",
        "lake-manifest.json",
        "lakefile.toml",
        "lakefile.lean",
    ] {
        seen.extend(std::fs::read(dir.join(file)).unwrap_or_default());
        seen.push(0);
    }
    let stamp = sha256(&seen);
    let stamp_file = stemma_dir(dir).join("cache");
    let got = std::fs::read_to_string(&stamp_file).unwrap_or_default();
    if got.trim() == stamp && dir.join(".lake/packages/mathlib").is_dir() {
        log.line("Mathlib's cache is as the dependencies need it.");
        return Ok(());
    }
    progress.say("Getting Mathlib's cache");
    let mut get = Command::new("lake");
    get.current_dir(dir).args(["exe", "cache", "get"]);
    if quietly(get, log).await? {
        std::fs::create_dir_all(stemma_dir(dir))?;
        std::fs::write(&stamp_file, stamp)?;
    } else {
        log.line("Mathlib's cache could not be got: building without it.");
    }
    Ok(())
}

/// Runs `lake build`; whether it passed.
pub async fn lake_build(dir: &Path, log: &Log, progress: &Progress) -> anyhow::Result<bool> {
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
    Ok(built.success())
}

/// Gets the cache if needed, builds, and inspects the library checked out
/// in `dir`.
pub async fn build_and_inspect(
    context: &Context,
    dir: &Path,
    log: &Log,
    progress: &Progress,
    cache: bool,
) -> anyhow::Result<Built> {
    if cache {
        get_cache(dir, log, progress).await?;
    }
    let started = std::time::Instant::now();
    if !lake_build(dir, log, progress).await? {
        return Ok(failed("lake build failed: see the log"));
    }
    log.line(&format!(
        "Built in {:.1}s.",
        started.elapsed().as_secs_f64()
    ));
    let started = std::time::Instant::now();
    let report = inspect(&context.work_dir, dir, log, progress).await?;
    log.line(&format!(
        "Inspected in {:.1}s.",
        started.elapsed().as_secs_f64()
    ));
    Ok(match report {
        Ok(report) => Built {
            succeeded: true,
            error: None,
            log: String::new(),
            report: Some(report),
        },
        Err(error) => failed(&error),
    })
}

// ── The inspection ────────────────────────────────────────────────────

/// What the inspection found of a module, kept for the next time.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Inspected {
    /// The hash of the module's `.olean` when it was inspected.
    hash: String,
    /// The library's modules it imports.
    imports: Vec<String>,
    declarations: Vec<Declaration>,
}

/// What the inspection found of a checkout, by module, with the Lean and the
/// inspection that found it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Kept {
    toolchain: String,
    inspection: String,
    lean: String,
    modules: BTreeMap<String, Inspected>,
}

/// What the inspection program prints.
#[derive(Deserialize)]
struct Printed {
    lean: String,
    #[serde(default)]
    imports: BTreeMap<String, Vec<String>>,
    declarations: Vec<Declaration>,
}

/// The modules to inspect again: those whose build changed, and those that
/// import them, as far as is known of what imports what.
fn to_inspect(
    modules: &BTreeMap<String, String>,
    kept: &BTreeMap<String, Inspected>,
) -> BTreeSet<String> {
    let mut dirty: BTreeSet<String> = modules
        .iter()
        .filter(|(module, hash)| kept.get(*module).is_none_or(|k| &k.hash != *hash))
        .map(|(module, _)| module.clone())
        .collect();
    // Upwards, until nothing more imports what is dirty.
    loop {
        let more: Vec<String> = modules
            .keys()
            .filter(|module| !dirty.contains(*module))
            .filter(|module| {
                kept.get(*module)
                    .is_some_and(|k| k.imports.iter().any(|i| dirty.contains(i)))
            })
            .cloned()
            .collect();
        if more.is_empty() {
            return dirty;
        }
        dirty.extend(more);
    }
}

/// The library's toolchain, as its `lean-toolchain` names it.
fn toolchain(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("lean-toolchain"))
        .unwrap_or_default()
        .trim()
        .to_owned()
}

/// The inspection, compiled with the library's toolchain into a program in
/// the work folder, once per toolchain and version of the inspection; none
/// if it cannot be compiled.
async fn inspection_program(work_dir: &Path, dir: &Path, log: &Log) -> Option<PathBuf> {
    let toolchain = toolchain(dir);
    if toolchain.is_empty() {
        return None;
    }
    let name: String = toolchain
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let package = work_dir.join("inspect").join(name);
    let program = package.join(".lake/build/bin/inspect");
    let stamp = package.join("inspection");
    let version = sha256(INSPECT.as_bytes());
    if program.is_file() && std::fs::read_to_string(&stamp).is_ok_and(|s| s == version) {
        return Some(program);
    }
    let made = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(&package)?;
        std::fs::write(package.join("lean-toolchain"), format!("{toolchain}\n"))?;
        std::fs::write(
            package.join("lakefile.toml"),
            "name = \"stemma-inspect\"\n\n[[lean_exe]]\nname = \"inspect\"\nroot = \"Inspect\"\n\
             supportInterpreter = true\n",
        )?;
        std::fs::write(package.join("Inspect.lean"), INSPECT)
    })();
    if made.is_err() {
        return None;
    }
    let mut lake = Command::new("lake");
    lake.current_dir(&package).args(["build", "inspect"]);
    log.line("Compiling the inspection for this toolchain, once.");
    match quietly(lake, log).await {
        Ok(true) if program.is_file() => {
            let _ = std::fs::write(&stamp, version);
            Some(program)
        }
        _ => {
            log.line("The inspection could not be compiled: it is interpreted.");
            None
        }
    }
}

/// Where the modules of the library built in `dir` and of its dependencies
/// are, as Lake lays them out.
fn lean_path(dir: &Path) -> anyhow::Result<std::ffi::OsString> {
    let mut paths = vec![dir.join(".lake/build/lib/lean")];
    if let Ok(packages) = std::fs::read_dir(dir.join(".lake/packages")) {
        let mut packages: Vec<PathBuf> = packages.flatten().map(|e| e.path()).collect();
        packages.sort();
        paths.extend(packages.into_iter().map(|p| p.join(".lake/build/lib/lean")));
    }
    Ok(std::env::join_paths(paths)?)
}

/// Runs the inspection on the library built in `dir`, again only on what
/// changed since the last time; its report, or why there is none.
pub async fn inspect(
    work_dir: &Path,
    dir: &Path,
    log: &Log,
    progress: &Progress,
) -> anyhow::Result<Result<LibraryReport, String>> {
    let modules = library_modules(dir);
    if modules.is_empty() {
        return Ok(Err("The build made no modules".into()));
    }
    let mut hashes = BTreeMap::new();
    for module in &modules {
        let olean = dir
            .join(".lake/build/lib/lean")
            .join(format!("{}.olean", module.replace('.', "/")));
        hashes.insert(module.clone(), sha256(&std::fs::read(&olean)?));
    }

    let kept_file = stemma_dir(dir).join("inspection.json");
    let toolchain = toolchain(dir);
    let version = sha256(INSPECT.as_bytes());
    let mut kept: Kept = std::fs::read(&kept_file)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .filter(|k: &Kept| k.toolchain == toolchain && k.inspection == version)
        .unwrap_or_default();
    kept.modules.retain(|module, _| hashes.contains_key(module));
    let dirty = to_inspect(&hashes, &kept.modules);

    if dirty.is_empty() {
        log.line("No module changed since the last inspection: its report stands.");
    } else {
        progress.say("Inspecting");
        log.line(&format!(
            "Inspecting {} of {} modules: {}",
            dirty.len(),
            modules.len(),
            dirty.iter().cloned().collect::<Vec<_>>().join(" ")
        ));
        // The library's modules, those to inspect marked.
        let args: Vec<String> = modules
            .iter()
            .map(|m| {
                if dirty.contains(m) {
                    format!("+{m}")
                } else {
                    m.clone()
                }
            })
            .collect();
        // The compiled inspection is given the library's and its
        // dependencies' modules itself: `lake env` would load the whole
        // workspace again, which takes a while.
        let mut command = match inspection_program(work_dir, dir, log).await {
            Some(program) => {
                let mut command = Command::new(program);
                command.env("LEAN_PATH", lean_path(dir)?);
                command
            }
            None => {
                let source = stemma_dir(dir).join("Inspect.lean");
                std::fs::create_dir_all(stemma_dir(dir))?;
                std::fs::write(&source, INSPECT)?;
                let mut command = Command::new("lake");
                command.args(["env", "lean", "--run"]).arg(source);
                command
            }
        };
        command.current_dir(dir);
        command
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        log.line(&format!("$ inspect {}", args.join(" ")));
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
        let printed: Printed = match serde_json::from_slice(&output.stdout) {
            Ok(printed) => printed,
            Err(error) => {
                return Ok(Err(format!(
                    "The inspection's report is unreadable: {error}"
                )));
            }
        };
        kept.lean = printed.lean;
        for module in &dirty {
            kept.modules.insert(
                module.clone(),
                Inspected {
                    hash: hashes[module].clone(),
                    imports: printed.imports.get(module).cloned().unwrap_or_default(),
                    declarations: Vec::new(),
                },
            );
        }
        for declaration in printed.declarations {
            if let Some(inspected) = kept.modules.get_mut(&declaration.module) {
                inspected.declarations.push(declaration);
            }
        }
        kept.toolchain = toolchain;
        kept.inspection = version;
        std::fs::create_dir_all(stemma_dir(dir))?;
        std::fs::write(&kept_file, serde_json::to_vec(&kept)?)?;
    }

    let report = LibraryReport {
        lean: kept.lean.clone(),
        modules: modules.clone(),
        declarations: kept
            .modules
            .values()
            .flat_map(|inspected| inspected.declarations.iter().cloned())
            .collect(),
    };
    log.line(&format!(
        "Inspected {} declarations in {} modules.",
        report.declarations.len(),
        report.modules.len()
    ));
    Ok(Ok(report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_lakes_progress() {
        assert_eq!(
            build_progress("✔ [12/40] Built Geometry.Basic (1.2s)").as_deref(),
            Some("Building [12/40] Geometry.Basic")
        );
        assert_eq!(build_progress("Build completed successfully."), None);
    }

    #[test]
    fn inspects_again_what_changed_and_what_imports_it() {
        let inspected = |hash: &str, imports: &[&str]| Inspected {
            hash: hash.into(),
            imports: imports.iter().map(|i| (*i).to_owned()).collect(),
            declarations: Vec::new(),
        };
        let kept: BTreeMap<String, Inspected> = [
            ("A", inspected("a", &[])),
            ("B", inspected("b", &["A"])),
            ("C", inspected("c", &["B"])),
            ("D", inspected("d", &[])),
        ]
        .into_iter()
        .map(|(m, i)| (m.to_owned(), i))
        .collect();
        let hashes = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
            pairs
                .iter()
                .map(|(m, h)| ((*m).to_owned(), (*h).to_owned()))
                .collect()
        };
        let same = hashes(&[("A", "a"), ("B", "b"), ("C", "c"), ("D", "d")]);
        assert!(to_inspect(&same, &kept).is_empty());
        let a_changed = hashes(&[("A", "a2"), ("B", "b"), ("C", "c"), ("D", "d")]);
        assert_eq!(
            to_inspect(&a_changed, &kept)
                .into_iter()
                .collect::<Vec<_>>(),
            ["A", "B", "C"]
        );
        let new_module = hashes(&[("A", "a"), ("B", "b"), ("C", "c"), ("D", "d"), ("E", "e")]);
        assert_eq!(
            to_inspect(&new_module, &kept)
                .into_iter()
                .collect::<Vec<_>>(),
            ["E"]
        );
    }
}
