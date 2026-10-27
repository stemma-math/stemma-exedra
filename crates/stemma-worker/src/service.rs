//! Per-user system service management, shared by the CLI and desktop.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::Context as _;
use serde::Serialize;

use stemma_client::config::{Credentials, Paths};

const LABEL: &str = "org.stemma.machine";
const UNIT: &str = "stemma-machine.service";

#[derive(Debug, Serialize)]
pub struct Status {
    pub installed: bool,
    pub running: bool,
    pub connected: Option<bool>,
    pub signed_out: bool,
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
    if let Some(root) = std::env::var_os("STEMMA_SERVICE_DIR") {
        return Ok(PathBuf::from(root).join(if cfg!(target_os = "macos") {
            "org.stemma.machine.plist"
        } else {
            UNIT
        }));
    }
    Ok(if cfg!(target_os = "macos") {
        home()?.join("Library/LaunchAgents/org.stemma.machine.plist")
    } else {
        home()?.join(".config/systemd/user/stemma-machine.service")
    })
}

fn log_path() -> anyhow::Result<PathBuf> {
    Ok(Paths::discover()?.state.join("logs/machine.log"))
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
    let config = std::path::absolute(Paths::discover()?.config)?;
    let search = std::env::var("PATH").unwrap_or_default();
    let path = path()?;
    std::fs::create_dir_all(path.parent().context("Service directory")?)?;
    if cfg!(target_os = "macos") {
        if loaded()? {
            stop()?;
        }
        let logs = log_path()?;
        std::fs::create_dir_all(logs.parent().context("Log directory")?)?;
        let paths = Paths::discover()?;
        let mut text = plist(stemma, &config, &search, &logs);
        let extra = environment(&paths)?
            .into_iter()
            .map(|(key, value)| format!("<key>{}</key><string>{}</string>", xml(&key), xml(&value)))
            .collect::<String>();
        text = text.replace(
            "</dict>\n<key>StandardOutPath",
            &format!("{extra}</dict>\n<key>StandardOutPath"),
        );
        std::fs::write(&path, text)?;
        command(
            "launchctl",
            &[
                "bootstrap",
                &domain()?,
                path.to_str().context("Service path is not UTF-8")?,
            ],
        )
    } else {
        let paths = Paths::discover()?;
        let logs = log_path()?;
        std::fs::create_dir_all(logs.parent().context("Log directory")?)?;
        let extra = environment(&paths)?
            .into_iter()
            .map(|(key, value)| format!("Environment={}\n", quoted(&format!("{key}={value}"))))
            .collect::<String>();
        let text = unit(stemma, &config, &search)
            .replace("Restart=on-failure", &format!("{extra}Restart=on-failure"));
        let text = text.replace(
            "Restart=on-failure",
            &format!(
                "StandardOutput=append:{}\nStandardError=append:{}\nRestart=on-failure",
                logs.display(),
                logs.display()
            ),
        );
        std::fs::write(&path, text)?;
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
        "Install the service first: `stemma machine start`"
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
    let config = std::path::absolute(Paths::discover()?.config)?;
    Ok(if cfg!(target_os = "macos") {
        text.contains(&format!(
            "<key>STEMMA_CONFIG_DIR</key><string>{}</string>",
            xml(&config.to_string_lossy())
        ))
    } else {
        text.lines().any(|line| {
            line == format!(
                "Environment={}",
                quoted(&format!("STEMMA_CONFIG_DIR={}", config.display()))
            )
        })
    })
}

/// Re-read changed settings if the service is active, including failure backoff.
pub fn reload() -> anyhow::Result<()> {
    if !manages_config()? {
        return Ok(());
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

pub async fn status() -> anyhow::Result<Status> {
    let installed = manages_config()?;
    let running = installed && running()?;
    let credentials = Credentials::load()?.with_env();
    let mut signed_out = credentials.token.is_none();
    let connected = if credentials.token.is_some() {
        match tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let hub = credentials.hub()?;
            let me = hub.me().await?;
            let token = hub.token_identity().await?;
            let _ = me;
            Ok::<_, anyhow::Error>(token.connected)
        })
        .await
        {
            Ok(Ok(online)) => Some(online),
            Ok(Err(error)) => {
                if error
                    .downcast_ref::<stemma_client::ApiError>()
                    .and_then(|e| e.status())
                    == Some(reqwest::StatusCode::UNAUTHORIZED)
                {
                    signed_out = true;
                }
                Some(false)
            }
            Err(_) => Some(false),
        }
    } else {
        Some(false)
    };
    Ok(Status {
        installed,
        running,
        connected,
        signed_out,
        logs: log_path()?.display().to_string(),
        stale_agents: Vec::new(),
    })
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
<key>EnvironmentVariables</key><dict><key>PATH</key><string>{search}</string><key>STEMMA_CONFIG_DIR</key><string>{config}</string></dict>
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
    let config = quoted(&format!("STEMMA_CONFIG_DIR={}", config.display()));
    let search = quoted(&format!("PATH={search}"));
    format!(
        "[Unit]\nDescription=Stemma machine\n\n[Service]\nExecStart={stemma} machine run\nEnvironment={config}\nEnvironment={search}\nRestart=on-failure\nRestartSec=10\n\n[Install]\nWantedBy=default.target\n"
    )
}

/// Save nonsecret settings overrides with the service; credentials stay in their own file.
fn environment(paths: &Paths) -> anyhow::Result<Vec<(String, String)>> {
    let mut variables = vec![
        ("STEMMA_STATE_DIR".into(), paths.state.display().to_string()),
        ("STEMMA_DATA_DIR".into(), paths.data.display().to_string()),
    ];
    for setting in stemma_client::config::Settings::load()?.explain(None, &Default::default())? {
        if let stemma_client::config::Source::Environment(name) = setting.source {
            variables.push((name.clone(), std::env::var(name)?));
        }
    }
    if let Ok(hub) = std::env::var("STEMMA_HUB") {
        variables.push(("STEMMA_HUB".into(), hub));
    }
    Ok(variables)
}
