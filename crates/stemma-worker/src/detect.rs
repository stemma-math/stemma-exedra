//! Which runners a machine has: those whose program is installed and
//! answers `--version`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use stemma_core::api::runners;

use crate::config::Runners;

/// How long a program has to say its version.
const VERSION_WAIT: Duration = Duration::from_secs(15);

/// Where `program` is: itself, if it is a path, or else the first of
/// that name on `search`, a list of folders as `PATH` has them.
pub fn find_program(program: &str, search: Option<&OsStr>) -> Option<PathBuf> {
    let program = Path::new(program);
    if program.components().count() > 1 {
        return is_executable(program).then(|| program.to_owned());
    }
    std::env::split_paths(search?)
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

/// Whether a program runs: `program --version` ends well, in a while.
pub async fn usable(program: &Path) -> bool {
    let mut command = tokio::process::Command::new(program);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    matches!(
        tokio::time::timeout(VERSION_WAIT, child.wait()).await,
        Ok(Ok(status)) if status.success()
    )
}

/// The runners installed and usable here, in [`runners::ALL`]'s order:
/// each with the program its configuration names, or its usual one,
/// found on `search`.
pub async fn installed(configured: &Runners, search: Option<&OsStr>) -> Vec<String> {
    let mut found = Vec::new();
    for runner in runners::ALL {
        let program = configured
            .get(runner)
            .and_then(|c| c.command.clone())
            .or_else(|| runners::program(runner).map(str::to_owned));
        let Some(program) = program else {
            continue;
        };
        if let Some(path) = find_program(&program, search)
            && usable(&path).await
        {
            found.push((*runner).to_owned());
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

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
    async fn finds_the_runners_that_answer() {
        let bin = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        fake(bin.path(), "claude", "echo '2.1.0 (Claude Code)'");
        // Installed, but broken.
        fake(bin.path(), "codex", "exit 1");
        // Not executable.
        std::fs::write(bin.path().join("opencode"), "").unwrap();
        let search = std::env::join_paths([other.path(), bin.path()]).unwrap();
        assert_eq!(
            installed(&Runners::default(), Some(&search)).await,
            ["claude-code"]
        );

        // A runner's configuration may name its program.
        let opencode = fake(other.path(), "my-opencode", "echo 1.0");
        let configured: Runners = toml::from_str(&format!(
            "[opencode]\ncommand = {:?}\n",
            opencode.display().to_string()
        ))
        .unwrap();
        assert_eq!(
            installed(&configured, Some(&search)).await,
            ["claude-code", "opencode"]
        );
        assert!(installed(&Runners::default(), None).await.is_empty());
    }
}
