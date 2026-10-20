//! Signing in and keeping the same credentials for the CLI and desktop.

use crate::{Hub, config::ClientConfig, hub::normalize_url};
use anyhow::Context as _;

/// Check credentials, replace this device's previous token, and save them.
pub async fn save(
    hub: &str,
    token: String,
    token_id: Option<i64>,
    group: Option<String>,
) -> anyhow::Result<ClientConfig> {
    let url = normalize_url(hub)?;
    let mut previous = ClientConfig::load()?;
    let client = Hub::new(&url, Some(token.clone()))?;
    let me = client.me().await.context("checking the token")?;
    if let Some(group) = &group {
        anyhow::ensure!(
            me.groups.iter().any(|g| &g.slug == group),
            "you are not in a group {group}"
        );
    }
    if previous.hub.as_deref() == Some(&url)
        && let (Some(old), Some(id)) = (previous.token.take(), previous.token_id)
        && old != token
    {
        let _ = Hub::new(&url, Some(old))?.revoke_token(id).await;
    }
    let config = ClientConfig {
        hub: Some(url),
        token: Some(token),
        token_id,
        username: Some(me.user.username),
        group: group.or_else(|| me.groups.first().map(|g| g.slug.clone())),
    };
    config.save()?;
    Ok(config)
}

/// Sign in with the owner's account, creating a revocable device token.
pub async fn password(
    hub: &str,
    username: &str,
    password: &str,
    device: &str,
    group: Option<String>,
) -> anyhow::Result<ClientConfig> {
    let mut client = Hub::new(&normalize_url(hub)?, None)?;
    client
        .sign_in(username, password)
        .await
        .context("signing in")?;
    let created = client.create_token(&format!("stemma on {device}")).await?;
    let _ = client.sign_out().await;
    save(hub, created.secret, Some(created.token.id), group).await
}

/// Forget the local sign-in, revoking its device token when the hub is reachable.
/// Returns a warning if the token must be revoked later from the web app.
pub async fn logout() -> anyhow::Result<Option<String>> {
    let path = crate::config::path()?;
    let config = ClientConfig::load_from(&path)?;
    let mut warning = None;
    if let (Some(url), Some(token), Some(id)) = (config.hub, config.token, config.token_id)
        && let Err(error) = Hub::new(&url, Some(token))?.revoke_token(id).await
    {
        warning = Some(format!(
            "Could not revoke the token ({error}); revoke it in the web app."
        ));
    }
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).with_context(|| format!("removing {}", path.display())),
    }
    Ok(warning)
}
