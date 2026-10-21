//! Disposable group workspaces, shared by jobs and interactive sessions.

use anyhow::Context as _;
use std::path::{Path, PathBuf};
use stemma_client::{
    Hub,
    config::{Paths, Settings},
};

/// A shared lease prevents resets while any job or session uses the group.
pub struct Workspace {
    pub path: PathBuf,
    _lease: std::fs::File,
}

fn lock_file(path: &Path) -> anyhow::Result<std::fs::File> {
    Ok(std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?)
}

impl Workspace {
    pub async fn open(
        settings: &Settings,
        paths: &Paths,
        hub: &Hub,
        group: &str,
        token: &str,
    ) -> anyhow::Result<Self> {
        let path = settings.workspace(group)?;
        let locks = paths.state.join("groups");
        std::fs::create_dir_all(&locks)?;
        let update = lock_file(&locks.join(format!("{group}.update.lock")))?;
        let lease = lock_file(&locks.join(format!("{group}.lock")))?;
        let (update, lease, refresh) = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            update.lock()?;
            let refresh = match lease.try_lock() {
                Ok(()) => true,
                Err(std::fs::TryLockError::WouldBlock) => {
                    lease.lock_shared()?;
                    false
                }
                Err(error) => return Err(error.into()),
            };
            Ok((update, lease, refresh))
        })
        .await??;
        if refresh {
            std::fs::create_dir_all(&path)?;
            checkout(
                hub.base(),
                token,
                &format!("/git/{group}/library.git"),
                &path.join("library"),
            )
            .await?;
            for paper in hub.papers(group).await? {
                checkout(
                    hub.base(),
                    token,
                    &format!("/git/{group}/paper-{}.git", paper.slug),
                    &path.join("papers").join(&paper.slug),
                )
                .await?;
            }
            lease.lock_shared()?;
        }
        drop(update);
        std::fs::create_dir_all(path.join("scratch"))?;
        Ok(Self {
            path,
            _lease: lease,
        })
    }
}

async fn git(dir: &Path, token: &str, args: &[&str]) -> anyhow::Result<()> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.extraHeader")
        .env(
            "GIT_CONFIG_VALUE_0",
            format!("Authorization: Bearer {token}"),
        )
        .kill_on_drop(true)
        .output()
        .await?;
    anyhow::ensure!(
        output.status.success(),
        "could not update {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr).replace(token, "[token]")
    );
    Ok(())
}

async fn checkout(hub: &str, token: &str, repository: &str, dir: &Path) -> anyhow::Result<()> {
    if !dir.join(".git").exists() {
        std::fs::create_dir_all(dir)?;
        git(dir, token, &["init", "--initial-branch=main"]).await?;
        git(
            dir,
            token,
            &["remote", "add", "origin", &format!("{hub}{repository}")],
        )
        .await?;
    }
    git(dir, token, &["fetch", "--depth=1", "origin", "main"]).await?;
    git(dir, token, &["checkout", "-B", "main", "FETCH_HEAD"]).await?;
    git(dir, token, &["reset", "--hard", "FETCH_HEAD"]).await?;
    // Ignored files stay: Lake's `.lake`, with the packages and the build,
    // so that the next job neither fetches Mathlib nor builds it again.
    git(dir, token, &["clean", "-ffd"]).await?;
    Ok(())
}

/// Job scratch is outside launch files and is removed even on cancellation.
pub struct Scratch(pub PathBuf);
impl Scratch {
    pub fn create(workspace: &Path, id: i64) -> anyhow::Result<Self> {
        let path = workspace
            .join("scratch")
            .join(format!("{id}-{:08x}", rand::random::<u32>()));
        std::fs::create_dir_all(&path).with_context(|| format!("creating {}", path.display()))?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "Ada")
            .env("GIT_AUTHOR_EMAIL", "ada@example.org")
            .env("GIT_COMMITTER_NAME", "Ada")
            .env("GIT_COMMITTER_EMAIL", "ada@example.org")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    #[tokio::test]
    async fn an_update_keeps_lakes_build_and_drops_the_rest() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        run(&source, &["init", "-q", "--initial-branch=main"]);
        std::fs::write(source.join(".gitignore"), ".lake\n").unwrap();
        std::fs::write(source.join("Basic.lean"), "theorem t : True := trivial\n").unwrap();
        run(&source, &["add", "."]);
        run(&source, &["commit", "-qm", "Start"]);
        run(
            temp.path(),
            &["clone", "-q", "--bare", "source", "library.git"],
        );

        let hub = format!("file://{}", temp.path().display());
        let dir = temp.path().join("workspace/library");
        checkout(&hub, "stj_x", "/library.git", &dir).await.unwrap();
        assert!(dir.join("Basic.lean").is_file());
        std::fs::create_dir_all(dir.join(".lake/packages/mathlib")).unwrap();
        std::fs::write(dir.join("Basic.lean"), "changed\n").unwrap();
        std::fs::write(dir.join("scratch.lean"), "left over\n").unwrap();

        checkout(&hub, "stj_x", "/library.git", &dir).await.unwrap();
        assert!(dir.join(".lake/packages/mathlib").is_dir());
        assert!(!dir.join("scratch.lean").exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("Basic.lean")).unwrap(),
            "theorem t : True := trivial\n"
        );
    }
}
