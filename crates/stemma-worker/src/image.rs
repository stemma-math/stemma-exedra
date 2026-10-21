//! The agents' image, which `stemma machine image` builds:
//! `docker/agents/Dockerfile` in Stemma's repository, embedded in the
//! binary, with Lean, the agents' programs, git, and `stemma` of this
//! version, for the agents' tools inside.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::Context as _;

use crate::isolation::Engine;

/// The image's description.
pub const DOCKERFILE: &str = include_str!("../../../docker/agents/Dockerfile");

/// What the image runs first, as the job's user.
pub const ENTRYPOINT: &str = include_str!("../../../docker/agents/entrypoint.sh");

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
}

impl Source {
    pub fn parse(text: &str) -> Option<Source> {
        match text {
            "release" => Some(Source::Release),
            "git" => Some(Source::Git),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Source::Release => "release",
            Source::Git => "git",
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
            .join("docker/agents/Dockerfile")
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
    let folder = dir.join("docker/agents");
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
    Ok(())
}

/// Builds the image with `engine`, showing what it does.
pub async fn build(engine: &dyn Engine, build: &Build) -> anyhow::Result<()> {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("stemma-image-{:08x}", rand::random::<u32>()));
    write_context(&dir)?;
    let status = tokio::process::Command::new(engine.program())
        .args(build_args(build, &dir))
        .stdin(Stdio::null())
        .status()
        .await
        .with_context(|| format!("running {}", engine.program().display()));
    let _ = std::fs::remove_dir_all(&dir);
    anyhow::ensure!(status?.success(), "the image could not be built");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_this_versions_image() {
        let build = Build {
            tag: "stemma-agents:1.2.3".into(),
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
                "/tmp/x/docker/agents/Dockerfile",
                "--tag",
                "stemma-agents:1.2.3",
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
        assert!(dir.path().join("docker/agents/entrypoint.sh").is_file());
        // The image has every runner, and `stemma`, whose versions are
        // explicit.
        for wanted in [
            "ARG CLAUDE_CODE_VERSION=",
            "ARG CODEX_VERSION=",
            "ARG OPENCODE_VERSION=",
            "ARG ELAN_VERSION=",
            "/usr/local/bin/stemma",
            "USER",
        ] {
            assert!(DOCKERFILE.contains(wanted), "{wanted}");
        }
        assert_eq!(TOOLCHAIN.trim(), "leanprover/lean4:v4.35.0");
    }
}
