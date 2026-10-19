//! Which runners a machine has: those whose program is installed and
//! answers `--version`.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use stemma_core::api::runners;

use crate::config::Runners;

/// How long a program has to say its version.
const VERSION_WAIT: Duration = Duration::from_secs(15);

/// How long the login shell has to say its environment.
const LOGIN_WAIT: Duration = Duration::from_secs(10);

/// The `PATH` the person's login shell sets up. An app opened from the
/// desktop does not inherit it (on macOS it gets `/usr/bin:/bin:…`), and
/// so would find neither the agents nor what they run on. `None` if there
/// is no shell to ask, or it does not answer in a while.
pub fn login_path() -> Option<OsString> {
    login_path_of(Path::new(&std::env::var_os("SHELL")?), LOGIN_WAIT)
}

fn login_path_of(shell: &Path, wait: Duration) -> Option<OsString> {
    // `env` prints the environment the same way whatever the shell: fish,
    // zsh or bash.
    let mut child = std::process::Command::new(shell)
        .args(["-l", "-c", "env"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + wait;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut output = String::new();
    child.stdout.take()?.read_to_string(&mut output).ok()?;
    output
        .lines()
        .find_map(|line| line.strip_prefix("PATH="))
        .filter(|path| !path.trim().is_empty())
        .map(OsString::from)
}

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

/// A runner's local sign-in state, without exposing credentials.
#[derive(Clone, Debug, serde::Serialize)]
pub struct AgentStatus {
    pub runner: String,
    pub name: String,
    pub installed: bool,
    pub sign_in: String,
}

pub async fn agents(configured: &Runners) -> Vec<AgentStatus> {
    let search = std::env::var_os("PATH");
    let mut agents = Vec::new();
    for runner in runners::ALL {
        let program = configured
            .get(runner)
            .and_then(|c| c.command.as_deref())
            .or_else(|| runners::program(runner))
            .unwrap_or(runner);
        let path = find_program(program, search.as_deref());
        let installed = match &path {
            Some(path) => usable(path).await,
            None => false,
        };
        let sign_in = if !installed {
            "Not installed".into()
        } else if *runner == runners::OPENCODE {
            // OpenCode has multiple providers: report credentials, never infer
            // that a particular model or subscription is usable.
            let path = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| dirs::home_dir().map(|p| p.join(".local/share")))
                .map(|p| p.join("opencode/auth.json"));
            let configured = path
                .and_then(|p| std::fs::read(p).ok())
                .and_then(|s| serde_json::from_slice::<serde_json::Value>(&s).ok())
                .and_then(|v| v.as_object().map(|o| !o.is_empty()))
                .unwrap_or(false);
            if configured {
                "Signed in (provider credentials found)".into()
            } else {
                "No saved sign-in; environment providers may be available".into()
            }
        } else {
            let args: &[&str] = if *runner == runners::CLAUDE_CODE {
                &["auth", "status", "--json"]
            } else {
                &["login", "status"]
            };
            let mut cmd = tokio::process::Command::new(path.as_ref().expect("installed"));
            cmd.args(args).stdin(Stdio::null()).kill_on_drop(true);
            match tokio::time::timeout(VERSION_WAIT, cmd.output()).await {
                Ok(Ok(output)) if *runner == runners::CLAUDE_CODE => {
                    match serde_json::from_slice::<serde_json::Value>(&output.stdout)
                        .ok()
                        .and_then(|v| v["loggedIn"].as_bool())
                    {
                        Some(true) => "Signed in".into(),
                        Some(false) => "Not signed in".into(),
                        None => "Sign-in status unavailable".into(),
                    }
                }
                Ok(Ok(output)) => {
                    if output.status.success() {
                        "Signed in".into()
                    } else {
                        "Not signed in".into()
                    }
                }
                _ => "Sign-in status unavailable".into(),
            }
        };
        agents.push(AgentStatus {
            runner: (*runner).into(),
            name: runners::name(runner).into(),
            installed,
            sign_in,
        });
    }
    agents
}

/// A tool's version, when installed and responsive.
pub async fn version(program: &str) -> Option<String> {
    let path = find_program(program, std::env::var_os("PATH").as_deref())?;
    let output = tokio::time::timeout(
        VERSION_WAIT,
        tokio::process::Command::new(path)
            .arg("--version")
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned()
    })
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
    #[test]
    fn reads_the_path_of_the_login_shell() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake(
            dir.path(),
            "shell",
            "[ \"$*\" = '-l -c env' ] || exit 2\nprintf 'HOME=/home/ada\\nPATH=/nix/bin:/usr/bin\\nTERM=dumb\\n'",
        );
        assert_eq!(
            login_path_of(&shell, LOGIN_WAIT),
            Some(OsString::from("/nix/bin:/usr/bin"))
        );
        let failing = fake(dir.path(), "failing", "exit 1");
        assert_eq!(login_path_of(&failing, LOGIN_WAIT), None);
        let silent = fake(dir.path(), "silent", "sleep 5");
        assert_eq!(login_path_of(&silent, Duration::from_millis(200)), None);
        assert_eq!(login_path_of(&dir.path().join("missing"), LOGIN_WAIT), None);
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
