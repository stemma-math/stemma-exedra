//! Per-user system service management, shared by the CLI and desktop.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::Context as _;
use serde::Serialize;

use crate::machine::{self, MachineConfig};

const LABEL: &str = "org.stemma.machine";
const UNIT: &str = "stemma-machine.service";

#[derive(Debug, Serialize)]
pub struct Status {
    pub installed: bool,
    pub running: bool,
    pub personal: bool,
    pub workshop: bool,
    pub connected: Option<bool>,
    pub stopped: Option<machine::Stopped>,
    pub logs: String,
    /// Installed now, but unavailable with the service's saved search path.
    pub stale_agents: Vec<String>,
}

fn supported() -> anyhow::Result<()> {
    anyhow::ensure!(
        cfg!(any(target_os = "macos", target_os = "linux")),
        "System services are not supported here; run `stemma machine run`"
    );
    Ok(())
}

fn home() -> anyhow::Result<PathBuf> {
    dirs::home_dir().context("This system has no home directory")
}

fn path() -> anyhow::Result<PathBuf> {
    supported()?;
    Ok(if cfg!(target_os = "macos") {
        home()?.join("Library/LaunchAgents/org.stemma.machine.plist")
    } else {
        home()?.join(".config/systemd/user/stemma-machine.service")
    })
}

fn output(program: &str, args: &[&str]) -> anyhow::Result<Output> {
    Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("Running {program}"))
}

fn command(program: &str, args: &[&str]) -> anyhow::Result<()> {
    let result = output(program, args)?;
    anyhow::ensure!(
        result.status.success(),
        "{program} {} failed: {}{}",
        args.join(" "),
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}

fn domain() -> anyhow::Result<String> {
    let uid = output("id", &["-u"])?;
    anyhow::ensure!(uid.status.success(), "Cannot find this user's uid");
    Ok(format!("gui/{}", String::from_utf8(uid.stdout)?.trim()))
}

fn target() -> anyhow::Result<String> {
    Ok(format!("{}/{LABEL}", domain()?))
}

fn loaded() -> anyhow::Result<bool> {
    Ok(output("launchctl", &["print", &target()?])?
        .status
        .success())
}

fn running() -> anyhow::Result<bool> {
    if cfg!(target_os = "macos") {
        let result = output("launchctl", &["print", &target()?])?;
        Ok(result.status.success()
            && String::from_utf8_lossy(&result.stdout)
                .lines()
                .any(|line| line.trim() == "state = running"))
    } else {
        Ok(output("systemctl", &["--user", "is-active", UNIT])?
            .status
            .success())
    }
}

/// Install for login and start now. Only a CLI executable belongs in the service.
pub fn install(stemma: &Path) -> anyhow::Result<()> {
    supported()?;
    anyhow::ensure!(
        stemma.is_absolute() && stemma.is_file(),
        "Use an absolute path to the stemma CLI"
    );
    let config = std::path::absolute(machine::path()?)?;
    let search = std::env::var("PATH").unwrap_or_default();
    let path = path()?;
    std::fs::create_dir_all(path.parent().context("Service directory")?)?;
    if cfg!(target_os = "macos") {
        if loaded()? {
            stop()?;
        }
        let logs = home()?.join("Library/Logs/Stemma/machine.log");
        std::fs::create_dir_all(logs.parent().context("Log directory")?)?;
        std::fs::write(&path, plist(stemma, &config, &search, &logs))?;
        command(
            "launchctl",
            &[
                "bootstrap",
                &domain()?,
                path.to_str().context("Service path is not UTF-8")?,
            ],
        )
    } else {
        std::fs::write(&path, unit(stemma, &config, &search))?;
        command("systemctl", &["--user", "daemon-reload"])?;
        command("systemctl", &["--user", "enable", UNIT])?;
        command("systemctl", &["--user", "restart", UNIT])
    }
}

pub fn uninstall() -> anyhow::Result<()> {
    let path = path()?;
    stop()?;
    if !path.exists() {
        return Ok(());
    }
    if cfg!(target_os = "linux") {
        command("systemctl", &["--user", "disable", UNIT])?;
    }
    std::fs::remove_file(path)?;
    if cfg!(target_os = "linux") {
        command("systemctl", &["--user", "daemon-reload"])?;
    }
    Ok(())
}

pub fn start() -> anyhow::Result<()> {
    let path = path()?;
    anyhow::ensure!(
        path.is_file(),
        "Install the service first: `stemma machine service install`"
    );
    if cfg!(target_os = "macos") {
        if !loaded()? {
            command(
                "launchctl",
                &[
                    "bootstrap",
                    &domain()?,
                    path.to_str().context("Service path is not UTF-8")?,
                ],
            )?;
        }
        command("launchctl", &["kickstart", &target()?])
    } else {
        command("systemctl", &["--user", "start", UNIT])
    }
}

pub fn stop() -> anyhow::Result<()> {
    supported()?;
    if cfg!(target_os = "macos") {
        if loaded()? {
            command("launchctl", &["bootout", &target()?])?;
        }
        Ok(())
    } else if path()?.exists() {
        command("systemctl", &["--user", "stop", UNIT])
    } else {
        Ok(())
    }
}

/// Whether this service serves the configuration being edited. Alternate
/// configurations, including tests, must never restart another machine's service.
fn manages_config() -> anyhow::Result<bool> {
    if !cfg!(any(target_os = "macos", target_os = "linux")) {
        return Ok(false);
    }
    let text = match std::fs::read_to_string(path()?) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let config = std::path::absolute(machine::path()?)?;
    Ok(if cfg!(target_os = "macos") {
        text.contains(&format!(
            "<key>STEMMA_MACHINE_CONFIG</key><string>{}</string>",
            xml(&config.to_string_lossy())
        ))
    } else {
        text.lines().any(|line| {
            line == format!(
                "Environment={}",
                quoted(&format!("STEMMA_MACHINE_CONFIG={}", config.display()))
            )
        })
    })
}

/// Re-read changed settings if the service is active, including failure backoff.
pub fn reload() -> anyhow::Result<()> {
    if !manages_config()? {
        return Ok(());
    }
    let config = crate::control::load()?;
    if !config.personal && !config.workshop_enabled {
        return stop();
    }
    if cfg!(target_os = "macos") {
        if loaded()? {
            command("launchctl", &["kickstart", "-k", &target()?])?;
        }
    } else {
        command("systemctl", &["--user", "try-restart", UNIT])?;
    }
    Ok(())
}

/// Removing a registration also removes its service, if installed.
pub fn remove_machine() -> anyhow::Result<()> {
    if manages_config()? {
        uninstall()?;
    }
    Ok(())
}

pub async fn status() -> anyhow::Result<Status> {
    let installed = manages_config()?;
    let running = installed && running()?;
    let config = MachineConfig::load_from(&machine::path()?)?;
    let mut connected = None;
    let mut stopped = config
        .as_ref()
        .map(|c| machine::stopped(&machine::path()?, c))
        .transpose()?
        .flatten();
    if let Some(config) = &config {
        match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            crate::control::identity(config),
        )
        .await
        {
            Ok(Ok(machine)) => connected = Some(machine.online),
            Ok(Err(error))
                if error.downcast_ref::<machine::Stopped>()
                    == Some(&machine::Stopped::NotKnown) =>
            {
                stopped = Some(machine::Stopped::NotKnown);
                connected = Some(false);
                machine::record_stopped(&machine::path()?, config, &machine::Stopped::NotKnown)?;
            }
            _ => {}
        }
    }
    let stale_agents = if installed {
        let text = std::fs::read_to_string(path()?)?;
        let stored = stored_path(&text, cfg!(target_os = "macos"))
            .context("Cannot read the service's stored PATH; reinstall the service")?;
        let runners = config
            .as_ref()
            .map(|c| c.runners.clone())
            .unwrap_or_default();
        stale_agents(&runners, &stored, std::env::var_os("PATH").as_deref()).await
    } else {
        Vec::new()
    };
    Ok(Status {
        stale_agents,
        installed,
        running,
        personal: config.as_ref().is_some_and(|c| c.personal),
        workshop: config.as_ref().is_some_and(|c| c.workshop_enabled),
        connected,
        stopped,
        logs: if cfg!(target_os = "macos") {
            home()?
                .join("Library/Logs/Stemma/machine.log")
                .display()
                .to_string()
        } else {
            "journalctl --user -u stemma-machine".into()
        },
    })
}

async fn stale_agents(
    configured: &crate::config::Runners,
    stored: &str,
    current: Option<&std::ffi::OsStr>,
) -> Vec<String> {
    let (service, now) = tokio::join!(
        crate::detect::installed(configured, Some(std::ffi::OsStr::new(stored))),
        crate::detect::installed(configured, current),
    );
    now.into_iter()
        .filter(|agent| !service.contains(agent))
        .collect()
}

/// Read the environment entry written by our installer, including escaped paths.
fn stored_path(text: &str, macos: bool) -> Option<String> {
    if macos {
        let value = text
            .split_once("<key>PATH</key><string>")?
            .1
            .split_once("</string>")?
            .0;
        Some(
            value
                .replace("&apos;", "'")
                .replace("&quot;", "\"")
                .replace("&gt;", ">")
                .replace("&lt;", "<")
                .replace("&amp;", "&"),
        )
    } else {
        let value = text
            .lines()
            .find_map(|line| line.strip_prefix("Environment=\"PATH="))?
            .strip_suffix('"')?;
        let mut result = String::new();
        let mut chars = value.chars();
        while let Some(c) = chars.next() {
            result.push(if c == '\\' {
                match chars.next()? {
                    'n' => '\n',
                    'r' => '\r',
                    c => c,
                }
            } else {
                c
            });
        }
        Some(result.replace("%%", "%"))
    }
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// No shell interprets ProgramArguments, even for paths containing spaces.
pub fn plist(stemma: &Path, config: &Path, search: &str, logs: &Path) -> String {
    let stemma = xml(&stemma.to_string_lossy());
    let config = xml(&config.to_string_lossy());
    let search = xml(search);
    let logs = xml(&logs.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{LABEL}</string>
<key>ProgramArguments</key><array><string>{stemma}</string><string>machine</string><string>run</string></array>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
<key>ThrottleInterval</key><integer>10</integer>
<key>EnvironmentVariables</key><dict><key>PATH</key><string>{search}</string><key>STEMMA_MACHINE_CONFIG</key><string>{config}</string></dict>
<key>StandardOutPath</key><string>{logs}</string>
<key>StandardErrorPath</key><string>{logs}</string>
</dict></plist>
"#
    )
}

fn quoted(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
}

pub fn unit(stemma: &Path, config: &Path, search: &str) -> String {
    // ExecStart also expands dollar variables; $$ is a literal dollar there.
    let stemma = quoted(&stemma.to_string_lossy()).replace('$', "$$");
    let config = quoted(&format!("STEMMA_MACHINE_CONFIG={}", config.display()));
    let search = quoted(&format!("PATH={search}"));
    format!(
        "[Unit]\nDescription=Stemma machine\n\n[Service]\nExecStart={stemma} machine run\nEnvironment={config}\nEnvironment={search}\nRestart=on-failure\nRestartSec=10\n\n[Install]\nWantedBy=default.target\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_saved_paths_without_losing_escaping() {
        let search = "/a & b:<bin>:'quoted':\"double\":/100%/back\\slash";
        assert_eq!(
            stored_path(
                &plist(
                    Path::new("/stemma"),
                    Path::new("/config"),
                    search,
                    Path::new("/logs")
                ),
                true
            )
            .as_deref(),
            Some(search)
        );
        assert_eq!(
            stored_path(
                &unit(Path::new("/stemma"), Path::new("/config"), search),
                false
            )
            .as_deref(),
            Some(search)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn warns_only_for_agents_missing_from_the_saved_path() {
        use std::os::unix::fs::PermissionsExt;
        let old = tempfile::tempdir().unwrap();
        let new = tempfile::tempdir().unwrap();
        for (dir, name) in [(old.path(), "claude"), (new.path(), "codex")] {
            let path = dir.join(name);
            std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let current = std::env::join_paths([old.path(), new.path()]).unwrap();
        let configured = crate::config::Runners::default();
        assert_eq!(
            stale_agents(&configured, old.path().to_str().unwrap(), Some(&current)).await,
            ["codex"]
        );
        assert!(
            stale_agents(&configured, current.to_str().unwrap(), Some(&current))
                .await
                .is_empty()
        );
        assert!(
            stale_agents(
                &configured,
                old.path().to_str().unwrap(),
                Some(old.path().as_os_str())
            )
            .await
            .is_empty()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn checks_the_agents_interpreter_with_each_path() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let agent = dir.path().join("agent");
        let interpreter = dir.path().join("agent-interpreter");
        std::fs::write(&agent, "#!/usr/bin/env agent-interpreter\n").unwrap();
        std::fs::write(&interpreter, "#!/bin/sh\nexit 0\n").unwrap();
        for path in [&agent, &interpreter] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let configured =
            toml::from_str(&format!("[codex]\ncommand = {:?}", agent.to_str().unwrap())).unwrap();
        assert_eq!(
            stale_agents(&configured, "/nonexistent", Some(dir.path().as_os_str())).await,
            ["codex"]
        );
    }

    #[test]
    fn launch_agent_preserves_arguments_and_escapes_xml() {
        let text = plist(
            Path::new("/Applications/Stemma tools/stemma"),
            Path::new("/Users/A & B/machine.toml"),
            "/bin:/my tools",
            Path::new("/my logs/machine.log"),
        );
        assert!(text.contains("<string>/Applications/Stemma tools/stemma</string><string>machine</string><string>run</string>"));
        assert!(text.contains("/Users/A &amp; B/machine.toml"));
        assert!(text.contains("<key>RunAtLoad</key><true/>"));
        assert!(text.contains("<key>SuccessfulExit</key><false/>"));
        assert!(text.contains("<string>/my logs/machine.log</string>"));
    }

    #[test]
    fn systemd_unit_quotes_paths_and_escapes_expansions() {
        let text = unit(
            Path::new("/my tools/100%/$stemma"),
            Path::new("/my config/machine.toml"),
            "/bin:/my tools",
        );
        assert!(text.contains("ExecStart=\"/my tools/100%%/$$stemma\" machine run"));
        assert!(text.contains("Environment=\"STEMMA_MACHINE_CONFIG=/my config/machine.toml\""));
        assert!(text.contains("Environment=\"PATH=/bin:/my tools\""));
        assert!(text.contains("Restart=on-failure"));
        assert!(text.contains("WantedBy=default.target"));
        assert_eq!(quoted("/a\"b\\c\n"), "\"/a\\\"b\\\\c\\n\"");
    }
}
