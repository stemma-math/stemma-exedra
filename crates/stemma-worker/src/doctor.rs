//! Diagnostics shared by the command line and This machine in the app.
use serde::Serialize;
use stemma_client::config::{Credentials, Paths, Settings};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Success,
    Warning,
    Danger,
}
#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub name: String,
    pub status: Status,
    pub detail: String,
    pub purpose: Option<String>,
    pub remedy: Option<String>,
}
fn check(name: &str, ok: bool, detail: impl Into<String>, remedy: &str) -> Check {
    Check {
        name: name.into(),
        status: if ok { Status::Success } else { Status::Warning },
        detail: detail.into(),
        purpose: None,
        remedy: (!ok).then(|| remedy.into()),
    }
}
async fn program(name: &str, command: &str, remedy: &str) -> Check {
    let path = crate::detect::find_program(command, std::env::var_os("PATH").as_deref());
    if let Some(path) = path {
        let version = crate::detect::version(&path.to_string_lossy()).await;
        check(
            name,
            version.is_some(),
            format!(
                "{} – {}",
                path.display(),
                version.as_deref().unwrap_or("could not read version")
            ),
            remedy,
        )
    } else {
        check(name, false, "Not found", remedy)
    }
}

/// Whether `handle`'s agent can sign in inside a container: the variables
/// and files it is given, set and present here.
fn container_sign_in(settings: &Settings, group: Option<&str>, handle: &str) -> Check {
    let list = |key: &str| {
        settings
            .get(&format!("agents.{handle}.container.{key}"), group)
            .ok()
            .and_then(|v| v.value.as_array().cloned())
            .unwrap_or_default()
    };
    let variables: Vec<String> = list("env")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let files: Vec<String> = list("files")
        .iter()
        .filter_map(|f| f.get("from").and_then(|v| v.as_str()).map(str::to_owned))
        .collect();
    let name = format!(
        "{} in a container",
        match handle {
            "claude" => "Claude Code",
            "codex" => "Codex",
            _ => "OpenCode",
        }
    );
    let how = match handle {
        "claude" => {
            "Run claude setup-token, keep what it prints with stemma config secret set \
             CLAUDE_CODE_OAUTH_TOKEN, and stemma config set agents.claude.container.env \
             CLAUDE_CODE_OAUTH_TOKEN."
        }
        "codex" => {
            "stemma config set agents.codex.container.files \"[{ from = '~/.codex/auth.json' }]\"."
        }
        _ => {
            "Keep its providers' keys with stemma config secret set <NAME>, and list their \
             names in agents.opencode.container.env."
        }
    };
    if variables.is_empty() && files.is_empty() {
        return check(&name, false, "No sign-in for its container", how);
    }
    let kept = Credentials::load().map(|c| c.secrets).unwrap_or_default();
    let missing: Vec<String> = variables
        .iter()
        .filter(|v| !kept.contains_key(*v) && std::env::var_os(v).is_none())
        .map(|v| format!("{v} has no value"))
        .chain(files.iter().filter_map(|f| {
            let path = stemma_client::config::expand_home(f).ok()?;
            (!path.is_file()).then(|| format!("{} is missing", path.display()))
        }))
        .collect();
    let given = variables
        .iter()
        .chain(&files)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if missing.is_empty() {
        check(&name, true, given, "")
    } else {
        check(&name, false, missing.join(", "), how)
    }
}

pub async fn run(group: Option<&str>) -> Vec<Check> {
    let mut checks = Vec::new();
    let settings = Settings::load();
    checks.push(match &settings {
        Ok(settings) => check(
            "Configuration",
            true,
            settings.path.display().to_string(),
            "",
        ),
        Err(error) => Check {
            name: "Configuration".into(),
            status: Status::Danger,
            detail: format!("{error:#}"),
            purpose: None,
            remedy: Some("Fix the named setting with stemma config edit.".into()),
        },
    });
    let hub = async {
        let credentials = Credentials::load()?.with_env();
        let hub = credentials.hub()?;
        let me = hub.me().await?;
        Ok::<_, anyhow::Error>(format!(
            "{} – signed in as {}",
            hub.base(),
            me.user.display_name
        ))
    };
    checks.push(
        match tokio::time::timeout(std::time::Duration::from_secs(5), hub).await {
            Ok(Ok(detail)) => check("Hub and token", true, detail, ""),
            Ok(Err(error)) => check(
                "Hub and token",
                false,
                format!("{error:#}"),
                "Check the hub address and network; sign in with stemma login if signed out.",
            ),
            Err(_) => check(
                "Hub and token",
                false,
                "Hub did not respond",
                "Check the hub address and network.",
            ),
        },
    );
    // The runners this computer has, which its containers need a sign-in for.
    let mut found = Vec::new();
    for handle in ["claude", "codex", "opencode"] {
        let command = settings
            .as_ref()
            .ok()
            .and_then(|s| s.get(&format!("agents.{handle}.command"), group).ok())
            .and_then(|v| v.value.as_str().map(str::to_owned))
            .unwrap_or_else(|| handle.into());
        let checked = program(
            stemma_core::api::runners::name(if handle == "claude" {
                "claude-code"
            } else {
                handle
            }),
            &command,
            &format!("Install {handle}, sign in to it, or set agents.{handle}.command."),
        )
        .await;
        if matches!(checked.status, Status::Success) {
            found.push(handle);
        }
        checks.push(checked);
    }
    for (tool, purpose) in [
        ("elan", "Lean toolchains"),
        ("lake", "checking Lean proofs"),
        ("latexmk", "checking LaTeX papers"),
    ] {
        let mut result = program(
            tool,
            tool,
            &format!("Install {tool} so agents can check their work."),
        )
        .await;
        result.purpose = Some(format!("Agents can use it for {purpose}."));
        checks.push(result);
    }
    if let Ok(settings) = &settings {
        if let Ok(value) = settings.get("agents.isolation", group)
            && let Some(runtime @ ("docker" | "podman")) = value.value.as_str()
        {
            checks.push(program(runtime, runtime, &format!("Install and start {runtime}.")).await);
            let image = settings
                .get("agents.image", group)
                .ok()
                .and_then(|v| v.value.as_str().map(str::to_owned))
                .unwrap_or_default();
            let present = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                tokio::process::Command::new(runtime)
                    .args(["image", "inspect", &image])
                    .output(),
            )
            .await
            .is_ok_and(|r| r.is_ok_and(|o| o.status.success()));
            checks.push(check(
                "Agents' image",
                present,
                image,
                "Build the image with stemma machine image.",
            ));
            for handle in &found {
                checks.push(container_sign_in(settings, group, handle));
            }
        }
        let folder = group.map(|g| settings.workspace(g)).unwrap_or_else(|| {
            settings
                .get("agents.workspace", None)
                .and_then(|v| stemma_client::config::expand_home(v.value.as_str().unwrap()))
        });
        if let Ok(folder) = folder {
            let mut parent = folder.as_path();
            while !parent.exists() {
                if let Some(next) = parent.parent() {
                    parent = next;
                } else {
                    break;
                }
            }
            let writable = tempfile::NamedTempFile::new_in(parent).is_ok();
            checks.push(check(
                "Workspace writable",
                writable,
                folder.display().to_string(),
                "Choose a writable workspace with stemma config set agents.workspace <folder>.",
            ));
        }
    }
    checks.push(match crate::service::status().await {
        Ok(service) => check(
            "Service",
            service.installed && service.running,
            if service.running {
                "Installed and running"
            } else if service.installed {
                "Installed, stopped"
            } else {
                "Not installed"
            },
            "Run stemma machine start.",
        ),
        Err(error) => check(
            "Service",
            false,
            error.to_string(),
            "Run stemma machine start, or stemma machine run in a terminal.",
        ),
    });
    let _ = Paths::discover();
    checks
}
