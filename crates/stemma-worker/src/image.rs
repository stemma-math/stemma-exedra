//! Stemma's image, which `stemma machine image` builds:
//! `docker/images/stemma/Dockerfile` in Stemma's repository, embedded in
//! the binary, with Lean, Mathlib, LaTeX, the agents' programs, git, and
//! `stemma` of this version, for the agents' tools inside. A machine runs
//! all its jobs in it: answers, and the workshop's processes and builds.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::isolation::Engine;

/// The image's description.
pub const DOCKERFILE: &str = include_str!("../../../docker/images/stemma/Dockerfile");

/// What the image runs first, as the job's user.
pub const ENTRYPOINT: &str = include_str!("../../../docker/images/stemma/entrypoint.sh");

/// The toolchain of a new group's library, which the image has unless told
/// otherwise.
pub const TOOLCHAIN: &str = include_str!("../../../lean/template/lean-toolchain");

/// Where the image's `stemma` comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The release's Linux binary, for x86_64.
    Release,
    /// Built from the release's tag, for any architecture.
    Git,
    /// Built from a checkout of Stemma, as it is: the development image.
    Local,
}

impl Source {
    pub fn parse(text: &str) -> Option<Source> {
        match text {
            "release" => Some(Source::Release),
            "git" => Some(Source::Git),
            "local" => Some(Source::Local),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Source::Release => "release",
            Source::Git => "git",
            Source::Local => "local",
        }
    }

    /// The release's binary where there is one for this machine's
    /// architecture, which its containers share; else built.
    pub fn for_this_machine() -> Source {
        if std::env::consts::ARCH == "x86_64" {
            Source::Release
        } else {
            Source::Git
        }
    }
}

/// How to build the image.
#[derive(Clone, Debug)]
pub struct Build {
    pub tag: String,
    pub source: Source,
    /// The Lean toolchain it has, as `lean-toolchain` names it.
    pub toolchain: String,
    /// Stemma's version in it: this one.
    pub version: String,
}

/// The arguments of `<runtime> build`, in the folder `context`.
pub fn build_args(build: &Build, context: &Path) -> Vec<String> {
    vec![
        "build".into(),
        "--file".into(),
        context
            .join("docker/images/stemma/Dockerfile")
            .display()
            .to_string(),
        "--tag".into(),
        build.tag.clone(),
        "--build-arg".into(),
        format!("STEMMA_VERSION={}", build.version),
        "--build-arg".into(),
        format!("STEMMA_SOURCE={}", build.source.as_str()),
        "--build-arg".into(),
        format!("LEAN_TOOLCHAIN={}", build.toolchain.trim()),
        context.display().to_string(),
    ]
}

/// Writes the build's folder: the image's description and what it copies.
pub fn write_context(dir: &Path) -> anyhow::Result<()> {
    let folder = dir.join("docker/images/stemma");
    std::fs::create_dir_all(&folder)?;
    std::fs::write(folder.join("Dockerfile"), DOCKERFILE)?;
    std::fs::write(folder.join("entrypoint.sh"), ENTRYPOINT)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            folder.join("entrypoint.sh"),
            std::fs::Permissions::from_mode(0o755),
        )?;
    }
    std::fs::create_dir_all(dir.join("lean/template"))?;
    for (name, text) in [
        ("lean-toolchain", TOOLCHAIN),
        (
            "lakefile.toml",
            include_str!("../../../lean/template/lakefile.toml"),
        ),
        (
            "lake-manifest.json",
            include_str!("../../../lean/template/lake-manifest.json"),
        ),
    ] {
        std::fs::write(dir.join("lean/template").join(name), text)?;
    }
    Ok(())
}

/// The runtime that builds the image: the machine's, or Docker if it runs
/// its jobs unisolated.
async fn engine(config: &crate::WorkerConfig) -> anyhow::Result<std::sync::Arc<dyn Engine>> {
    use crate::isolation::{Runtime, find_engine};
    let runtime = match config.isolation.runtime {
        Runtime::Podman => Runtime::Podman,
        _ => Runtime::Docker,
    };
    find_engine(runtime, std::env::var_os("PATH").as_deref())
        .await
        .ok_or_else(|| anyhow::anyhow!("Install and start Docker or Podman."))
}

/// Builds the machine's image, as the general or group settings name it.
pub async fn build_for(group: Option<&str>) -> anyhow::Result<()> {
    let config = crate::WorkerConfig::load(group)?;
    let engine = engine(&config).await?;
    build(
        engine.as_ref(),
        &Build {
            tag: config.isolation.image(),
            source: Source::for_this_machine(),
            toolchain: TOOLCHAIN.into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    )
    .await
}

/// Builds the image with `engine`, showing what it does.
pub async fn build(engine: &dyn Engine, build: &Build) -> anyhow::Result<()> {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("stemma-image-{:08x}", rand::random::<u32>()));
    write_context(&dir)?;
    let built = run_build(engine, build, &dir).await;
    let _ = std::fs::remove_dir_all(&dir);
    built
}

/// Builds the machine's image from the checkout of Stemma at `root`, as it
/// is, under the same name: its `stemma` is that code's, uncommitted
/// changes included. Building it again without `--dev` brings back the
/// release's.
pub async fn build_dev(root: &Path, group: Option<&str>) -> anyhow::Result<String> {
    anyhow::ensure!(
        root.join("Cargo.toml").is_file()
            && root.join("crates/stemma").is_dir()
            && root.join("docker/images/stemma/Dockerfile").is_file(),
        "{} is not a checkout of Stemma: run this in one",
        root.display()
    );
    let config = crate::WorkerConfig::load(group)?;
    let engine = engine(&config).await?;
    let tag = config.isolation.image();
    run_build(
        engine.as_ref(),
        &Build {
            tag: tag.clone(),
            source: Source::Local,
            toolchain: TOOLCHAIN.into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        root,
    )
    .await?;
    Ok(tag)
}

/// Runs the runtime's build of `build` in the folder `dir`.
async fn run_build(engine: &dyn Engine, build: &Build, dir: &Path) -> anyhow::Result<()> {
    let mut command = tokio::process::Command::new(engine.program());
    command.args(build_args(build, dir));
    run_command(command).await
}

/// The lines of a failed build worth reading: the last ones, as a terminal
/// would show them (progress redrawn with carriage returns keeps its last
/// state), without blank lines.
fn last_lines(raw: &str, count: usize) -> String {
    let lines: Vec<&str> = raw
        .lines()
        .map(|line| line.rsplit('\r').next().unwrap_or("").trim_end())
        .filter(|line| !line.trim().is_empty())
        .collect();
    lines[lines.len().saturating_sub(count)..].join("\n")
}

/// The runtime's output goes to the terminal as it comes; a failure keeps
/// its whole log in the state's `logs/image.log`, and says its last lines.
async fn run_command(mut command: tokio::process::Command) -> anyhow::Result<()> {
    use std::io::Write;
    use tokio::io::AsyncReadExt;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut stderr = child.stderr.take().unwrap();
    let mut raw = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let count = stderr.read(&mut chunk).await?;
        if count == 0 {
            break;
        }
        std::io::stderr().write_all(&chunk[..count])?;
        raw.extend_from_slice(&chunk[..count]);
    }
    let status = child.wait().await?;
    if !status.success() {
        let text = String::from_utf8_lossy(&raw);
        let saved = stemma_client::config::Paths::discover()
            .ok()
            .and_then(|paths| {
                let path = paths.state.join("logs/image.log");
                std::fs::create_dir_all(path.parent()?).ok()?;
                std::fs::write(&path, text.as_bytes()).ok()?;
                Some(path)
            });
        let whole = saved
            .map(|path| format!("\n\nThe whole log is in {}.", path.display()))
            .unwrap_or_default();
        anyhow::bail!(
            "The image did not build. Its last lines:\n\n{}{whole}",
            last_lines(&text, 20)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_build_says_its_last_lines() {
        let raw = "#1 load\r\n#7 Updating files: 10%\rUpdating files: 100%, done.\n\n\
                   #7 ERROR: process did not complete\n";
        assert_eq!(
            last_lines(raw, 2),
            "Updating files: 100%, done.\n#7 ERROR: process did not complete"
        );
        assert_eq!(last_lines(raw, 9).lines().count(), 3);
    }
    #[test]
    fn mathlib_matches_the_library_template() {
        let template: toml::Value =
            toml::from_str(include_str!("../../../lean/template/lakefile.toml")).unwrap();
        let version = template["require"][0]["rev"].as_str().unwrap();
        assert!(DOCKERFILE.contains(&format!("ARG MATHLIB_VERSION={version}")));
        let dir = tempfile::tempdir().unwrap();
        write_context(dir.path()).unwrap();
        assert!(
            dir.path()
                .join("lean/template/lake-manifest.json")
                .is_file()
        );
    }
    #[test]
    fn builds_this_versions_image() {
        let build = Build {
            tag: "stemma/stemma:1.2.3".into(),
            source: Source::Git,
            toolchain: "leanprover/lean4:v4.35.0\n".into(),
            version: "1.2.3".into(),
        };
        let args = build_args(&build, Path::new("/tmp/x"));
        assert_eq!(
            args,
            [
                "build",
                "--file",
                "/tmp/x/docker/images/stemma/Dockerfile",
                "--tag",
                "stemma/stemma:1.2.3",
                "--build-arg",
                "STEMMA_VERSION=1.2.3",
                "--build-arg",
                "STEMMA_SOURCE=git",
                "--build-arg",
                "LEAN_TOOLCHAIN=leanprover/lean4:v4.35.0",
                "/tmp/x"
            ]
        );
        let dir = tempfile::tempdir().unwrap();
        write_context(dir.path()).unwrap();
        assert!(
            dir.path()
                .join("docker/images/stemma/entrypoint.sh")
                .is_file()
        );
        // The image has every runner, and `stemma`, whose versions are
        // explicit.
        for wanted in [
            "ARG CLAUDE_CODE_VERSION=",
            "ARG CODEX_VERSION=",
            "ARG OPENCODE_VERSION=",
            "ARG ELAN_VERSION=",
            "/opt/stemma/mathlib",
            "/usr/local/bin/stemma",
            "USER",
        ] {
            assert!(DOCKERFILE.contains(wanted), "{wanted}");
        }
        assert_eq!(TOOLCHAIN.trim(), "leanprover/lean4:v4.35.0");
    }
}
