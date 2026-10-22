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
    /// Built from the source this `stemma` was built from, still on this
    /// computer: a development build, whose code no release has.
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

    /// The folder of the source this binary was built from, if it is still
    /// there: then its containers run the same `stemma`, not a release's.
    pub fn checkout() -> Option<PathBuf> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        (root.join("Cargo.toml").is_file() && root.join("docker/agents/Dockerfile").is_file())
            .then(|| root.canonicalize().ok())
            .flatten()
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

/// Builds the image selected by the general or group settings.
pub async fn build_for(group: Option<&str>) -> anyhow::Result<()> {
    use crate::isolation;
    let config = crate::WorkerConfig::load(group)?;
    let mut isolation = config.isolation.clone();
    isolation.runtime = Some(match isolation.runtime() {
        isolation::Runtime::Podman => isolation::Runtime::Podman,
        _ => isolation::Runtime::Docker,
    });
    // Stemma builds only its own image. Another name is the person's own
    // image, which they build; building Stemma's under it would hide that.
    let tag = isolation.image();
    anyhow::ensure!(
        tag == isolation::default_image(),
        "agents.image names your own image, {tag}: Stemma builds only its own, {}. \
         Build yours yourself, or go back to Stemma's with: stemma config unset agents.image",
        isolation::default_image()
    );
    let engine = isolation::find_engine(&isolation, std::env::var_os("PATH").as_deref())
        .await
        .ok_or_else(|| anyhow::anyhow!("Install and start Docker or Podman."))?;
    let checkout = Source::checkout();
    build(
        engine.as_ref(),
        &Build {
            tag,
            source: if checkout.is_some() {
                Source::Local
            } else {
                Source::for_this_machine()
            },
            toolchain: TOOLCHAIN.into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    )
    .await
}

/// Builds the image with `engine`, showing what it does.
pub async fn build(engine: &dyn Engine, build: &Build) -> anyhow::Result<()> {
    // From the source itself, or from a folder of only the image's files.
    let (dir, made) = match (build.source, Source::checkout()) {
        (Source::Local, Some(root)) => (root, false),
        (Source::Local, None) => anyhow::bail!("the source this stemma was built from is gone"),
        _ => {
            let dir =
                std::env::temp_dir().join(format!("stemma-image-{:08x}", rand::random::<u32>()));
            write_context(&dir)?;
            (dir, true)
        }
    };
    let status = tokio::process::Command::new(engine.program())
        .args(build_args(build, &dir))
        .stdin(Stdio::null())
        .status()
        .await
        .with_context(|| format!("running {}", engine.program().display()));
    if made {
        let _ = std::fs::remove_dir_all(&dir);
    }
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
