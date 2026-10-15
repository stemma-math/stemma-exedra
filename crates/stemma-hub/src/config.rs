//! The hub's configuration: a TOML file, then environment variables.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

/// How the hub runs.
///
/// Every setting can be given in a TOML file and overridden by an
/// environment variable (`STEMMA_LISTEN`, `STEMMA_DATA_DIR`,
/// `STEMMA_PUBLIC_URL`, `STEMMA_MAX_UPLOAD_MB`).
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Where to listen for HTTP.
    pub listen: SocketAddr,
    /// Where the database, the attachments and the libraries live.
    pub data_dir: PathBuf,
    /// The address people reach the hub at, such as
    /// `https://stemma.example.org`, for the links the hub prints. Cookies
    /// are marked secure when it is HTTPS.
    pub public_url: Option<String>,
    /// The largest attachment accepted, in megabytes.
    pub max_upload_mb: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            listen: SocketAddr::from(([127, 0, 0, 1], 8080)),
            data_dir: PathBuf::from("data"),
            public_url: None,
            max_upload_mb: 50,
        }
    }
}

impl Config {
    /// Reads the configuration from `path`, if given, and the environment.
    pub fn load(path: Option<&Path>) -> anyhow::Result<Config> {
        let mut config = match path {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .with_context(|| format!("reading {}", path.display()))?;
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
            }
            None => Config::default(),
        };
        config.apply_env(|name| std::env::var(name).ok())?;
        Ok(config)
    }

    fn apply_env(&mut self, var: impl Fn(&str) -> Option<String>) -> anyhow::Result<()> {
        if let Some(listen) = var("STEMMA_LISTEN") {
            self.listen = listen.parse().context("STEMMA_LISTEN")?;
        }
        if let Some(dir) = var("STEMMA_DATA_DIR") {
            self.data_dir = dir.into();
        }
        if let Some(url) = var("STEMMA_PUBLIC_URL") {
            self.public_url = Some(url).filter(|u| !u.is_empty());
        }
        if let Some(mb) = var("STEMMA_MAX_UPLOAD_MB") {
            self.max_upload_mb = mb.parse().context("STEMMA_MAX_UPLOAD_MB")?;
        }
        Ok(())
    }

    /// The public address, without a trailing slash, or the local one.
    pub fn base_url(&self) -> String {
        match &self.public_url {
            Some(url) => url.trim_end_matches('/').to_owned(),
            None => format!("http://{}", self.listen),
        }
    }

    pub fn secure_cookies(&self) -> bool {
        self.public_url
            .as_deref()
            .is_some_and(|url| url.starts_with("https://"))
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("stemma.db")
    }

    pub fn attachments_dir(&self) -> PathBuf {
        self.data_dir.join("attachments")
    }

    /// Where the groups' libraries are, as bare git repositories.
    pub fn libraries_dir(&self) -> PathBuf {
        self.data_dir.join("libraries")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_toml_and_the_environment_wins() {
        let mut config: Config = toml::from_str(
            r#"
            listen = "0.0.0.0:9000"
            data_dir = "/var/lib/stemma"
            public_url = "https://stemma.example.org/"
            "#,
        )
        .unwrap();
        assert_eq!(config.max_upload_mb, 50);
        assert!(config.secure_cookies());
        assert_eq!(config.base_url(), "https://stemma.example.org");

        config
            .apply_env(|name| match name {
                "STEMMA_DATA_DIR" => Some("/srv/stemma".into()),
                "STEMMA_MAX_UPLOAD_MB" => Some("10".into()),
                _ => None,
            })
            .unwrap();
        assert_eq!(config.data_dir, PathBuf::from("/srv/stemma"));
        assert_eq!(config.max_upload_mb, 10);
        assert_eq!(config.listen.port(), 9000);
    }

    #[test]
    fn rejects_unknown_settings() {
        assert!(toml::from_str::<Config>("lisen = \"0.0.0.0:1\"").is_err());
    }
}
