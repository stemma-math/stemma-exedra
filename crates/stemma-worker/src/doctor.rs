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

/// Whether the person's own image has what an answer runs: `stemma`, for
/// the agents' tools, and the agents' programs, `commands`. It starts a
/// container of it to look, which only an image of one's own needs.
async fn own_image(runtime: &str, image: &str, commands: &[String]) -> Check {
    let wanted: Vec<&str> = std::iter::once("stemma")
        .chain(commands.iter().map(String::as_str))
        .collect();
    // Prints each program that is missing.
    let script = "for p in \"$@\"; do command -v \"$p\" >/dev/null || echo \"$p\"; done";
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        tokio::process::Command::new(runtime)
            .args([
                "run",
                "--rm",
                "--network",
                "none",
                "--entrypoint",
                "sh",
                image,
            ])
            .args(["-c", script, "sh"])
            .args(&wanted)
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await;
    let detail = format!("{image}, your own");
    match output {
        Ok(Ok(out)) if out.status.success() => {
            let missing: Vec<String> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect();
            if missing.is_empty() {
                check("Agents' image", true, detail, "")
            } else {
                let mut lacking = check(
                    "Agents' image",
                    false,
                    format!("{detail}: it lacks {}", missing.join(", ")),
                    "",
                );
                lacking.remedy = Some(format!(
                    "It lacks {}: build your image from Stemma's, which has them, or go back to \
                     Stemma's with stemma config unset agents.image.",
                    missing.join(", ")
                ));
                lacking
            }
        }
        _ => check(
            "Agents' image",
            false,
            format!("{detail}: it could not be started"),
            "Check that the image runs a shell, or go back to Stemma's with \
             stemma config unset agents.image.",
        ),
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
    let how = format!("Sign it in with stemma machine sign-in {handle}, or in This machine.");
    let how = how.as_str();
    // A sign-in of its own, made for the containers.
    if let Ok(paths) = Paths::discover()
        && crate::sign_in::folder(&paths, handle).is_some()
        && crate::sign_in::signed_in(&paths, handle)
    {
        return check(&name, true, "Signed in for containers", how);
    }
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
            let own = image != crate::isolation::default_image();
            checks.push(if !present {
                check(
                    "Agents' image",
                    false,
                    image,
                    if own {
                        "Build your image, or go back to Stemma's with stemma config unset agents.image."
                    } else {
                        "Build the image with stemma machine image."
                    },
                )
            } else if own {
                let commands: Vec<String> = found
                    .iter()
                    .filter_map(|handle| {
                        settings
                            .get(&format!("agents.{handle}.container.command"), group)
                            .ok()
                            .and_then(|v| v.value.as_str().map(str::to_owned))
                    })
                    .collect();
                own_image(runtime, &image, &commands).await
            } else {
                check("Agents' image", true, image, "")
            });
            for handle in &found {
                checks.push(container_sign_in(settings, group, handle));
            }
        }
        if let Ok(Some(why)) = settings.workshop_problem(group) {
            checks.push(check("Workshop", false, &why, &why));
        }
        if let Ok(value) = settings.get("agents.isolation", group)
            && let Some(runtime @ ("docker" | "podman")) = value.value.as_str()
        {
            let image = settings
                .get("workshop.image", group)
                .unwrap()
                .value
                .as_str()
                .unwrap()
                .to_owned();
            let out = tokio::process::Command::new(runtime)
                .args(["image", "inspect", &image])
                .output()
                .await;
            let present = out.as_ref().is_ok_and(|out| out.status.success());
            let mut result = if present && image != crate::image::workshop_image() {
                let mut result = own_image(
                    runtime,
                    &image,
                    &["git".into(), "lake".into(), "latexmk".into()],
                )
                .await;
                result.name = "Workshop image".into();
                result.remedy = result
                    .remedy
                    .map(|s| s.replace("agents.image", "workshop.image"));
                result
            } else {
                check(
                    "Workshop image",
                    present,
                    &image,
                    "Build both images with stemma machine image.",
                )
            };
            if let Ok(out) = out
                && !out.status.success()
            {
                result.detail = String::from_utf8_lossy(&out.stderr).into_owned();
            }
            checks.push(result);
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
