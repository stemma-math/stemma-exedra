//! What a machine signed in to a hub keeps: the hub's address, a personal
//! token and the group to use when none is named. It lives in the user's
//! configuration directory (`stemma/client.toml`), or where
//! `STEMMA_CLIENT_CONFIG` says, readable only by its owner.
//!
//! The environment wins over the file: `STEMMA_HUB`, `STEMMA_TOKEN` and
//! `STEMMA_GROUP`.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::hub::Hub;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientConfig {
    /// The hub's address.
    pub hub: Option<String>,
    /// A personal API token.
    pub token: Option<String>,
    /// The token's id, to revoke it on logging out.
    pub token_id: Option<i64>,
    /// Who signed in.
    pub username: Option<String>,
    /// The group to use when none is named.
    pub group: Option<String>,
}

/// Where the configuration is.
pub fn path() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("STEMMA_CLIENT_CONFIG") {
        return Ok(path.into());
    }
    let dir = dirs::config_dir().context("this system has no configuration directory")?;
    Ok(dir.join("stemma").join("client.toml"))
}

impl ClientConfig {
    /// Reads the configuration, or an empty one if there is none.
    pub fn load_from(path: &Path) -> anyhow::Result<ClientConfig> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("reading {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ClientConfig::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn load() -> anyhow::Result<ClientConfig> {
        Self::load_from(&path()?)
    }

    /// Writes the configuration, readable only by its owner.
    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let text = toml::to_string(self)?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options
            .open(path)
            .with_context(|| format!("writing {}", path.display()))?;
        std::io::Write::write_all(&mut file, text.as_bytes())?;
        Ok(())
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&path()?)
    }

    /// The configuration with the environment's settings over it.
    pub fn with_env(mut self) -> ClientConfig {
        let var = |name| std::env::var(name).ok().filter(|v: &String| !v.is_empty());
        if let Some(hub) = var("STEMMA_HUB") {
            self.hub = Some(hub);
        }
        if let Some(token) = var("STEMMA_TOKEN") {
            self.token = Some(token);
        }
        if let Some(group) = var("STEMMA_GROUP") {
            self.group = Some(group);
        }
        self
    }

    /// The hub, as this configuration reaches it.
    pub fn hub(&self) -> anyhow::Result<Hub> {
        let url = self
            .hub
            .as_deref()
            .context("not signed in to a hub: run `stemma login <hub>`")?;
        let token = self
            .token
            .clone()
            .context("no token for the hub: run `stemma login <hub>`")?;
        Hub::new(url, Some(token))
    }
}

/// Reads a token kept in a file, as a job's runner gives it.
pub fn read_token_file(path: &Path) -> anyhow::Result<String> {
    let token = std::fs::read_to_string(path)
        .with_context(|| format!("reading the token in {}", path.display()))?;
    Ok(token.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_privately_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stemma/client.toml");
        assert!(ClientConfig::load_from(&path).unwrap().hub.is_none());
        let config = ClientConfig {
            hub: Some("https://stemma.example.org".into()),
            token: Some("stm_x".into()),
            token_id: Some(3),
            username: Some("ada".into()),
            group: Some("geometry".into()),
        };
        config.save_to(&path).unwrap();
        let read = ClientConfig::load_from(&path).unwrap();
        assert_eq!(read.token.as_deref(), Some("stm_x"));
        assert_eq!(read.group.as_deref(), Some("geometry"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
