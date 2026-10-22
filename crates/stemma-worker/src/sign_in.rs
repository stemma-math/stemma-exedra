//! How each agent signs in inside an answer's container, without touching
//! the person's own sign-in on this computer.
//!
//! - Claude Code takes a long-lived token, from `claude setup-token`, kept
//!   with the person's credentials as `CLAUDE_CODE_OAUTH_TOKEN`.
//! - Codex and OpenCode keep a sign-in of their own, made for the
//!   containers in Stemma's data folder (`sign-ins/<agent>`), and mounted
//!   read-write: they renew it as they work. One answer at a time uses it,
//!   so two never renew it at once; the person's own sign-in, in their
//!   home, is never read or written.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

use stemma_client::config::{Credentials, Paths};

use crate::isolation::{HOME, Mount};

/// The variable Claude Code's token is kept and passed as.
pub const CLAUDE_TOKEN: &str = "CLAUDE_CODE_OAUTH_TOKEN";

/// The agents, by the handle their settings sit under.
pub const AGENTS: [&str; 3] = ["claude", "codex", "opencode"];

/// The folder of `handle`'s sign-in for containers, if it keeps one.
pub fn folder(paths: &Paths, handle: &str) -> Option<PathBuf> {
    matches!(handle, "codex" | "opencode").then(|| paths.data.join("sign-ins").join(handle))
}

/// The file whose presence says that `handle`'s folder holds a sign-in.
fn marker(folder: &Path, handle: &str) -> PathBuf {
    match handle {
        "opencode" => folder.join("opencode").join("auth.json"),
        _ => folder.join("auth.json"),
    }
}

/// Whether `handle` has a sign-in for containers: its token kept, or in the
/// environment, or its folder signed in.
pub fn signed_in(paths: &Paths, handle: &str) -> bool {
    match folder(paths, handle) {
        Some(folder) => marker(&folder, handle).is_file(),
        None => {
            let kept = Credentials::load()
                .map(|c| c.secrets.contains_key(CLAUDE_TOKEN))
                .unwrap_or(false);
            kept || std::env::var_os(CLAUDE_TOKEN).is_some()
        }
    }
}

/// The folder mounted for `handle`, read-write, where it signs in inside:
/// none until it has signed in.
pub fn mounts(paths: &Paths, handle: &str) -> Vec<Mount> {
    let Some(folder) = folder(paths, handle) else {
        return Vec::new();
    };
    if !marker(&folder, handle).is_file() {
        return Vec::new();
    }
    let (source, target) = match handle {
        "opencode" => (folder.join("opencode"), ".local/share/opencode"),
        _ => (folder, ".codex"),
    };
    vec![Mount::Bind {
        source,
        target: Path::new(HOME).join(target),
        read_only: false,
    }]
}

/// Holds `handle`'s sign-in folder for one answer, waiting while another
/// holds it. `None` for an agent without a folder.
pub async fn hold(paths: &Paths, handle: &str) -> anyhow::Result<Option<File>> {
    let Some(folder) = folder(paths, handle) else {
        return Ok(None);
    };
    std::fs::create_dir_all(&folder)?;
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(folder.join(".lock"))?;
    Ok(Some(
        tokio::task::spawn_blocking(move || file.lock().map(|()| file)).await??,
    ))
}

/// The command that signs `handle` in for containers, run on this
/// computer with `program`, the agent's program here: it signs in to the
/// folder, never to the person's own sign-in. Claude Code's prints a token
/// to keep.
pub fn command(
    paths: &Paths,
    handle: &str,
    program: &str,
) -> anyhow::Result<std::process::Command> {
    let mut command = std::process::Command::new(program);
    match handle {
        "claude" => {
            command.arg("setup-token");
        }
        "codex" => {
            let folder = folder(paths, handle).expect("Codex keeps a folder");
            std::fs::create_dir_all(&folder)?;
            command.arg("login").env("CODEX_HOME", folder);
        }
        "opencode" => {
            let folder = folder(paths, handle).expect("OpenCode keeps a folder");
            std::fs::create_dir_all(&folder)?;
            command.args(["auth", "login"]).env("XDG_DATA_HOME", folder);
        }
        other => anyhow::bail!("There is no agent {other}"),
    }
    Ok(command)
}

/// Forgets `handle`'s sign-in for containers.
pub fn forget(paths: &Paths, handle: &str) -> anyhow::Result<()> {
    match folder(paths, handle) {
        Some(folder) => match std::fs::remove_dir_all(&folder) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        },
        None => {
            let path = stemma_client::config::path()?;
            let mut credentials = Credentials::load_from(&path)?;
            credentials.secrets.remove(CLAUDE_TOKEN);
            credentials.save_to(&path)
        }
    }
}

/// The values of `names`, for a container's environment: kept with the
/// person's credentials, or else in this computer's environment.
pub fn values(names: &[String]) -> Vec<(String, String)> {
    let kept = Credentials::load().map(|c| c.secrets).unwrap_or_default();
    names
        .iter()
        .filter_map(|name| {
            kept.get(name)
                .cloned()
                .or_else(|| std::env::var(name).ok())
                .map(|value| (name.clone(), value))
        })
        .collect()
}

/// Asks `handle`'s agent, in a container as an answer's, for one word:
/// whether it signs in there, and answers. Returns what it said.
pub async fn test(handle: &str, group: Option<&str>) -> anyhow::Result<String> {
    use crate::isolation::{Isolator, Sandbox, owner_of};
    let config = crate::WorkerConfig::load(group)?;
    let runner = runner_of(handle)?;
    let Isolator::Containers(containers) = crate::isolation::resolve(
        &config.isolation,
        &config.hub,
        std::env::var_os("PATH").as_deref(),
    )
    .await
    .map_err(|why| anyhow::anyhow!("Containers do not work here: {why}"))?
    else {
        anyhow::bail!("Answers do not run in containers: choose Docker or Podman first");
    };
    // In Stemma's own folder, as an answer's: the runtime may not see the
    // system's temporary folder (Docker Desktop on macOS does not).
    let tests = config.paths.state.join("tests");
    std::fs::create_dir_all(&tests)?;
    let work = tempfile::tempdir_in(&tests)?;
    let (env, mut mounts) = containers.sign_in(runner, dirs::home_dir().as_deref())?;
    mounts.extend(self::mounts(&config.paths, handle));
    mounts.insert(
        0,
        Mount::Bind {
            source: work.path().to_owned(),
            target: crate::isolation::ROOT.into(),
            read_only: false,
        },
    );
    let _held = hold(&config.paths, handle).await?;
    let mut spec = containers.spec(
        0,
        owner_of(work.path()),
        mounts,
        env,
        Duration::from_secs(300),
    );
    spec.pass_values = values(&spec.pass_env);
    let container = containers.start(&spec, work.path()).await?;
    let program = containers
        .program(runner)
        .unwrap_or_else(|| handle.to_owned());
    const ASK: &str = "Reply with the single word: ready";
    let args: Vec<String> = match handle {
        "claude" => vec!["-p".into(), ASK.into(), "--max-turns".into(), "1".into()],
        "codex" => vec!["exec".into(), "--skip-git-repo-check".into(), ASK.into()],
        _ => vec!["run".into(), ASK.into()],
    };
    let mut command = container.command(&program, &args, &[], work.path());
    command
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(180), command.output())
        .await
        .map_err(|_| anyhow::anyhow!("It did not answer within three minutes"))??;
    let said = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if output.status.success() && !said.is_empty() {
        Ok(said.lines().last().unwrap_or_default().to_owned())
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        let last = error
            .lines()
            .chain(said.lines())
            .rfind(|l| !l.trim().is_empty())
            .unwrap_or("it gave no answer");
        anyhow::bail!(
            "It could not answer: {}",
            stemma_core::secrets::hide(last.trim())
        )
    }
}

/// The handle of `runner`.
pub fn handle_of(runner: &str) -> Option<&'static str> {
    AGENTS
        .into_iter()
        .find(|handle| runner_of(handle).is_ok_and(|r| r == runner))
}

/// The runner of `handle`.
pub fn runner_of(handle: &str) -> anyhow::Result<&'static str> {
    use stemma_core::api::runners;
    Ok(match handle {
        "claude" => runners::CLAUDE_CODE,
        "codex" => runners::CODEX,
        "opencode" => runners::OPENCODE,
        other => anyhow::bail!("There is no agent {other}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(dir: &Path) -> Paths {
        Paths {
            config: dir.join("config"),
            state: dir.join("state"),
            data: dir.join("data"),
        }
    }

    #[test]
    fn a_folder_is_mounted_read_write_once_it_holds_a_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        assert!(folder(&paths, "claude").is_none());
        assert!(mounts(&paths, "codex").is_empty());

        let codex = folder(&paths, "codex").unwrap();
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(codex.join("auth.json"), "{}").unwrap();
        let opencode = folder(&paths, "opencode").unwrap();
        std::fs::create_dir_all(opencode.join("opencode")).unwrap();
        std::fs::write(opencode.join("opencode/auth.json"), "{}").unwrap();

        assert!(signed_in(&paths, "codex") && signed_in(&paths, "opencode"));
        let target = |handle: &str| match &mounts(&paths, handle)[..] {
            [
                Mount::Bind {
                    source,
                    target,
                    read_only: false,
                },
            ] => (source.clone(), target.clone()),
            other => panic!("{other:?}"),
        };
        assert_eq!(target("codex"), (codex, "/home/stemma/.codex".into()));
        assert_eq!(
            target("opencode"),
            (
                opencode.join("opencode"),
                "/home/stemma/.local/share/opencode".into()
            )
        );

        forget(&paths, "codex").unwrap();
        assert!(!signed_in(&paths, "codex"));
    }

    #[test]
    fn the_sign_in_command_never_touches_ones_own() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let env = |handle: &str| {
            command(&paths, handle, handle)
                .unwrap()
                .get_envs()
                .map(|(k, v)| (k.to_owned(), v.map(ToOwned::to_owned)))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            env("codex"),
            [(
                "CODEX_HOME".into(),
                Some(paths.data.join("sign-ins/codex").into_os_string())
            )]
        );
        assert_eq!(
            env("opencode"),
            [(
                "XDG_DATA_HOME".into(),
                Some(paths.data.join("sign-ins/opencode").into_os_string())
            )]
        );
        assert!(env("claude").is_empty());
        assert!(command(&paths, "gpt", "gpt").is_err());
    }

    #[test]
    fn handles_and_runners_go_both_ways() {
        for handle in AGENTS {
            assert_eq!(handle_of(runner_of(handle).unwrap()), Some(handle));
        }
        assert_eq!(handle_of("api"), None);
    }
}
