//! The hub's configuration: a TOML file, then environment variables.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

/// How the hub runs.
///
/// Every setting can be given in a TOML file, and the main ones overridden
/// by an environment variable (`STEMMA_LISTEN`, `STEMMA_DATA_DIR`,
/// `STEMMA_PUBLIC_URL`, `STEMMA_MAX_UPLOAD_MB`, and the forge's
/// `STEMMA_FORGE_URL`, which sets a forge up, `STEMMA_FORGE_PUBLIC_URL`
/// and `STEMMA_FORGE_HUB_URL`). The forge's secrets are only read from the
/// environment, from the variables its section names.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Where to listen for HTTP.
    pub listen: SocketAddr,
    /// Where the database, the attachments and the signing key live.
    pub data_dir: PathBuf,
    /// The address people reach the hub at, such as
    /// `https://stemma.example.org`, for the links the hub prints. Cookies
    /// are marked secure when it is HTTPS. It is also the issuer of the
    /// hub's OpenID Connect tokens.
    pub public_url: Option<String>,
    /// The largest attachment accepted, in megabytes.
    pub max_upload_mb: u64,
    /// The Forgejo instance that keeps the groups' repositories. Without
    /// it, the hub creates no groups.
    pub forge: Option<ForgeConfig>,
}

/// The Forgejo instance beside the hub: each group is an organization in
/// it, and its library a repository; people sign in to it through the hub,
/// with OpenID Connect.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForgeConfig {
    /// Where the hub reaches Forgejo, for its API and git, such as
    /// `http://127.0.0.1:3000`.
    pub url: String,
    /// The address people reach Forgejo at, for links; `url` if absent.
    pub public_url: Option<String>,
    /// Where Forgejo reaches the hub, for OpenID Connect's back channel and
    /// for webhooks; the hub's public address if absent.
    pub hub_url: Option<String>,
    /// The id Forgejo gives the authentication source that signs people in
    /// through the hub (`forgejo admin auth list` shows it): 1, the first,
    /// if absent. The accounts the hub makes in Forgejo are linked to it.
    #[serde(default = "first_source")]
    pub auth_source_id: i64,
    /// The variable with the token of a Forgejo admin, with every scope.
    #[serde(default = "default_token_env")]
    pub token_env: String,
    /// The variable with the secret Forgejo signs its webhooks with.
    #[serde(default = "default_webhook_secret_env")]
    pub webhook_secret_env: String,
    /// Forgejo as a client of the hub's OpenID Connect provider.
    #[serde(default)]
    pub oidc: OidcClientConfig,
    /// The secrets, read from the variables above.
    #[serde(skip)]
    pub secrets: ForgeSecrets,
}

/// A client that signs people in through the hub.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcClientConfig {
    /// Its id: `forgejo` if absent.
    #[serde(default = "default_client_id")]
    pub client_id: String,
    /// The variable with the client's secret.
    #[serde(default = "default_client_secret_env")]
    pub client_secret_env: String,
    /// Where it may send people back to, each matched exactly: for Forgejo,
    /// `<its public address>/user/oauth2/<the source's name>/callback`; if
    /// absent, that of a source named `stemma`.
    #[serde(default)]
    pub redirect_uris: Vec<String>,
    /// Whether it must use PKCE, as Forgejo does: yes, unless set otherwise.
    /// Only `S256` is accepted, and a challenge given is always checked.
    #[serde(default = "yes")]
    pub require_pkce: bool,
}

/// The forge's secrets.
#[derive(Clone, Default)]
pub struct ForgeSecrets {
    /// A Forgejo admin's token.
    pub token: String,
    pub webhook_secret: String,
    /// The OpenID Connect client's secret.
    pub client_secret: String,
}

impl std::fmt::Debug for ForgeSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ForgeSecrets(…)")
    }
}

impl Default for OidcClientConfig {
    fn default() -> Self {
        OidcClientConfig {
            client_id: default_client_id(),
            client_secret_env: default_client_secret_env(),
            redirect_uris: Vec::new(),
            require_pkce: true,
        }
    }
}

fn yes() -> bool {
    true
}

fn first_source() -> i64 {
    1
}

fn default_client_id() -> String {
    "forgejo".into()
}

fn default_token_env() -> String {
    "STEMMA_FORGE_TOKEN".into()
}

fn default_webhook_secret_env() -> String {
    "STEMMA_FORGE_WEBHOOK_SECRET".into()
}

fn default_client_secret_env() -> String {
    "STEMMA_FORGE_CLIENT_SECRET".into()
}

/// The shortest secret accepted for webhooks and clients.
const MIN_SECRET: usize = 16;

impl ForgeConfig {
    /// A forge at `url`, whose source of id 1 signs people in through the
    /// hub as the client `forgejo`, sent back to `redirect_uri`; with
    /// `secrets`, and the rest as a file would leave it.
    pub fn new(url: &str, redirect_uri: &str, secrets: ForgeSecrets) -> ForgeConfig {
        ForgeConfig {
            oidc: OidcClientConfig {
                redirect_uris: vec![redirect_uri.to_owned()],
                ..Default::default()
            },
            secrets,
            ..Self::at(url)
        }
    }

    /// A forge at `url`, as a file that says only that would leave it.
    fn at(url: &str) -> ForgeConfig {
        ForgeConfig {
            url: url.to_owned(),
            public_url: None,
            hub_url: None,
            auth_source_id: first_source(),
            token_env: default_token_env(),
            webhook_secret_env: default_webhook_secret_env(),
            oidc: OidcClientConfig::default(),
            secrets: ForgeSecrets::default(),
        }
    }

    /// Where the hub reaches Forgejo, without a trailing slash.
    pub fn base(&self) -> String {
        self.url.trim_end_matches('/').to_owned()
    }

    /// Where people reach Forgejo, without a trailing slash.
    pub fn public_base(&self) -> String {
        self.public_url
            .as_deref()
            .unwrap_or(&self.url)
            .trim_end_matches('/')
            .to_owned()
    }

    fn read_secrets(&mut self, var: &impl Fn(&str) -> Option<String>) -> anyhow::Result<()> {
        let read = |name: &str| {
            var(name)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
                .with_context(|| format!("[forge] needs the environment variable {name}"))
        };
        self.secrets = ForgeSecrets {
            token: read(&self.token_env)?,
            webhook_secret: read(&self.webhook_secret_env)?,
            client_secret: read(&self.oidc.client_secret_env)?,
        };
        if self.oidc.redirect_uris.is_empty() {
            let callback = format!("{}/user/oauth2/stemma/callback", self.public_base());
            self.oidc.redirect_uris.push(callback);
        }
        self.check()
    }

    /// Checks what reading the file cannot.
    pub fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.secrets.webhook_secret.len() >= MIN_SECRET
                && self.secrets.client_secret.len() >= MIN_SECRET,
            "the forge's webhook and client secrets need at least {MIN_SECRET} characters"
        );
        anyhow::ensure!(
            !self.oidc.redirect_uris.is_empty(),
            "[forge.oidc] needs at least one redirect URI"
        );
        for uri in &self.oidc.redirect_uris {
            let parsed = url::Url::parse(uri).with_context(|| format!("redirect URI {uri}"))?;
            anyhow::ensure!(
                matches!(parsed.scheme(), "http" | "https") && parsed.fragment().is_none(),
                "a redirect URI is an http(s) address without a fragment: {uri}"
            );
        }
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            listen: SocketAddr::from(([127, 0, 0, 1], 8080)),
            data_dir: PathBuf::from("data"),
            public_url: None,
            max_upload_mb: 50,
            forge: None,
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
        let nonempty = |name: &str| var(name).filter(|v| !v.trim().is_empty());
        if let Some(url) = nonempty("STEMMA_FORGE_URL") {
            match &mut self.forge {
                Some(forge) => forge.url = url,
                None => self.forge = Some(ForgeConfig::at(&url)),
            }
        }
        if let Some(forge) = &mut self.forge {
            if let Some(url) = nonempty("STEMMA_FORGE_PUBLIC_URL") {
                forge.public_url = Some(url);
            }
            if let Some(url) = nonempty("STEMMA_FORGE_HUB_URL") {
                forge.hub_url = Some(url);
            }
            forge.read_secrets(&var)?;
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

    /// Where Forgejo reaches the hub, without a trailing slash.
    pub fn hub_url_for_forge(&self) -> String {
        match self.forge.as_ref().and_then(|f| f.hub_url.as_deref()) {
            Some(url) => url.trim_end_matches('/').to_owned(),
            None => self.base_url(),
        }
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("stemma.db")
    }

    pub fn attachments_dir(&self) -> PathBuf {
        self.data_dir.join("attachments")
    }

    /// The key the hub signs its OpenID Connect tokens with.
    pub fn signing_key_path(&self) -> PathBuf {
        self.data_dir.join("oidc-key.pem")
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
        assert!(config.forge.is_none());
    }

    #[test]
    fn reads_the_forge_with_its_secrets_from_the_environment() {
        let text = r#"
            [forge]
            url = "http://127.0.0.1:3000"
            public_url = "https://forge.example.org/"
            auth_source_id = 1
            [forge.oidc]
            client_id = "forgejo"
            redirect_uris = ["https://forge.example.org/user/oauth2/stemma/callback"]
        "#;
        let mut config: Config = toml::from_str(text).unwrap();
        // Without its secrets, the forge is refused.
        assert!(config.clone().apply_env(|_| None).is_err());
        let weak = config.clone().apply_env(|name| match name {
            "STEMMA_FORGE_TOKEN" => Some("admin-token".into()),
            _ => Some("short".into()),
        });
        assert!(weak.is_err());
        config
            .apply_env(|name| match name {
                "STEMMA_FORGE_TOKEN" => Some("admin-token".into()),
                "STEMMA_FORGE_WEBHOOK_SECRET" => Some("a webhook's long secret".into()),
                "STEMMA_FORGE_CLIENT_SECRET" => Some("a client's long secret".into()),
                _ => None,
            })
            .unwrap();
        let forge = config.forge.as_ref().unwrap();
        assert_eq!(forge.secrets.token, "admin-token");
        assert_eq!(forge.public_base(), "https://forge.example.org");
        assert_eq!(config.hub_url_for_forge(), "http://127.0.0.1:8080");
        // Secrets are not printed.
        assert!(!format!("{forge:?}").contains("admin-token"));
    }

    #[test]
    fn the_environment_alone_sets_a_forge_up() {
        let mut config = Config::default();
        config
            .apply_env(|name| match name {
                "STEMMA_FORGE_URL" => Some("http://forgejo:3000".into()),
                "STEMMA_FORGE_PUBLIC_URL" => Some("https://forge.example.org".into()),
                "STEMMA_FORGE_HUB_URL" => Some("http://hub:8080/".into()),
                "STEMMA_FORGE_TOKEN" => Some("admin-token".into()),
                "STEMMA_FORGE_WEBHOOK_SECRET" => Some("a webhook's long secret".into()),
                "STEMMA_FORGE_CLIENT_SECRET" => Some("a client's long secret".into()),
                _ => None,
            })
            .unwrap();
        let forge = config.forge.as_ref().unwrap();
        assert_eq!(forge.base(), "http://forgejo:3000");
        assert_eq!(forge.auth_source_id, 1);
        assert_eq!(forge.oidc.client_id, "forgejo");
        assert!(forge.oidc.require_pkce);
        assert_eq!(
            forge.oidc.redirect_uris,
            ["https://forge.example.org/user/oauth2/stemma/callback"]
        );
        assert_eq!(config.hub_url_for_forge(), "http://hub:8080");
    }

    #[test]
    fn rejects_unknown_settings() {
        assert!(toml::from_str::<Config>("lisen = \"0.0.0.0:1\"").is_err());
    }
}
