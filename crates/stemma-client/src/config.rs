//! Shared settings, credentials and state for the CLI, service and app.
//! Only credentials contain secrets. Environment overrides apply to every group.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::hub::Hub;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    /// The hub's address.
    pub hub: Option<String>,
    /// A personal API token.
    pub token: Option<String>,
    /// The token's id, to revoke it on logging out.
    pub token_id: Option<i64>,
    /// Who signed in.
    pub username: Option<String>,
    /// The values of the variables agents' containers are given by name
    /// (`agents.<runner>.container.env`): their sign-ins, kept here, the
    /// only file with secrets, rather than in the service's environment.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub secrets: std::collections::BTreeMap<String, String>,
}

/// The credentials file, separate from settings and active group state.
pub fn path() -> anyhow::Result<PathBuf> {
    Ok(Paths::discover()?.config.join("credentials.toml"))
}

impl Credentials {
    /// Reads the configuration, or an empty one if there is none.
    pub fn load_from(path: &Path) -> anyhow::Result<Credentials> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("reading {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Credentials::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn load() -> anyhow::Result<Credentials> {
        if std::env::var_os("STEMMA_HUB").is_some() && std::env::var_os("STEMMA_TOKEN").is_some() {
            return Ok(Self::default().with_env());
        }
        Self::load_from(&path()?)
    }

    /// Writes the configuration, readable only by its owner.
    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        private_write(path, toml::to_string(self)?.as_bytes())
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&path()?)
    }

    /// The configuration with the environment's settings over it.
    pub fn with_env(mut self) -> Credentials {
        let var = |name| std::env::var(name).ok();
        if let Some(hub) = var("STEMMA_HUB") {
            self.hub = Some(hub);
        }
        if let Some(token) = var("STEMMA_TOKEN") {
            if self.token.as_deref() != Some(&token) {
                self.token_id = None;
            }
            self.token = Some(token);
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
        Ok(Hub::new(url, Some(token))?.local(Paths::discover()?))
    }
}

/// Whether `name` may name a variable a container is given, whose value
/// the credentials keep: Stemma's own settings never enter a container.
pub fn check_secret_name(name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !name.is_empty()
            && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            && !name.to_uppercase().starts_with("STEMMA_"),
        "{name} cannot be a container's variable"
    );
    Ok(())
}

/// Reads a token kept in a file, as a job's runner gives it.
pub fn read_token_file(path: &Path) -> anyhow::Result<String> {
    let token = std::fs::read_to_string(path)
        .with_context(|| format!("reading the token in {}", path.display()))?;
    Ok(token.trim().to_owned())
}

use std::collections::BTreeMap;
use toml::Value;
use toml_edit::DocumentMut;

/// Independent folders, including on macOS. Discovery never creates or reads them.
#[derive(Clone, Debug)]
pub struct Paths {
    pub config: PathBuf,
    pub state: PathBuf,
    pub data: PathBuf,
}

impl Paths {
    pub fn discover() -> anyhow::Result<Self> {
        Self::from_env(&std::env::vars().collect(), dirs::home_dir().as_deref())
    }

    pub fn from_env(env: &BTreeMap<String, String>, home: Option<&Path>) -> anyhow::Result<Self> {
        let folder = |kind: &str, xdg: &str, fallback: &str| -> anyhow::Result<PathBuf> {
            if let Some(value) = env.get(&format!("STEMMA_{kind}_DIR")) {
                return Ok(value.into());
            }
            #[cfg(windows)]
            let base = env.get(if kind == "CONFIG" {
                "APPDATA"
            } else {
                "LOCALAPPDATA"
            });
            #[cfg(not(windows))]
            let base = env.get(xdg);
            if let Some(value) = base {
                return Ok(PathBuf::from(value).join("stemma"));
            }
            Ok(home
                .context("this system has no home directory")?
                .join(fallback)
                .join("stemma"))
        };
        Ok(Self {
            config: folder("CONFIG", "XDG_CONFIG_HOME", ".config")?,
            state: folder("STATE", "XDG_STATE_HOME", ".local/state")?,
            data: folder("DATA", "XDG_DATA_HOME", ".local/share")?,
        })
    }

    pub fn active_group(
        &self,
        option: Option<String>,
        env: &BTreeMap<String, String>,
    ) -> anyhow::Result<Option<String>> {
        if let Some(group) = option.or_else(|| env.get("STEMMA_GROUP").cloned()) {
            return Ok(Some(group));
        }
        Ok(read_optional(&self.state.join("group"))?
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()))
    }

    pub fn use_group(&self, group: &str) -> anyhow::Result<()> {
        validate_slug(group)?;
        private_write(&self.state.join("group"), format!("{group}\n").as_bytes())
    }
}

pub fn active_group(option: Option<String>) -> anyhow::Result<Option<String>> {
    Paths::discover()?.active_group(option, &std::env::vars().collect())
}

/// Atomic replacement, with private permissions even if an existing file was public.
fn private_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    let dir = path.parent().context("a file needs a parent directory")?;
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn read_optional(path: &Path) -> anyhow::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn validate_slug(slug: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !slug.is_empty()
            && slug
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
        "invalid group slug: {slug}"
    );
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum Source {
    Default,
    File(String),
    Group(String),
    Environment(String),
    CommandLine,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => write!(f, "default"),
            Self::File(path) => write!(f, "{path}"),
            Self::Group(group) => write!(f, "group {group}"),
            Self::Environment(name) => write!(f, "{name}"),
            Self::CommandLine => write!(f, "command line"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Explained {
    pub key: String,
    pub value: Value,
    pub source: Source,
}

/// Values kept flat internally so all consumers use the same known keys and origins.
#[derive(Clone, Debug)]
pub struct Settings {
    pub path: PathBuf,
    general: BTreeMap<String, Value>,
    groups: BTreeMap<String, BTreeMap<String, Value>>,
    defaults: BTreeMap<String, Value>,
    env: BTreeMap<String, String>,
}

impl Settings {
    pub fn load() -> anyhow::Result<Self> {
        Self::load_from(&Paths::discover()?, std::env::vars().collect())
    }

    pub fn load_from(paths: &Paths, env: BTreeMap<String, String>) -> anyhow::Result<Self> {
        let path = paths.config.join("config.toml");
        let text = read_optional(&path)?.unwrap_or_default();
        Self::parse(path, &text, paths, env)
    }

    pub fn parse(
        path: PathBuf,
        text: &str,
        paths: &Paths,
        env: BTreeMap<String, String>,
    ) -> anyhow::Result<Self> {
        let mut defaults = BTreeMap::new();
        for (key, value) in [
            ("enabled", Value::Boolean(true)),
            ("concurrency", Value::Integer(1)),
            ("priority", Value::String("normal".into())),
            ("isolation", Value::String("none".into())),
            ("attachments", Value::String("mine".into())),
            (
                "image",
                Value::String(format!("stemma-agents:{}", env!("CARGO_PKG_VERSION"))),
            ),
            (
                "workspace",
                Value::String(paths.data.join("groups").to_string_lossy().into()),
            ),
        ] {
            defaults.insert(format!("agents.{key}"), value);
        }
        for runner in ["claude", "codex", "opencode"] {
            for (key, value) in [
                (
                    "command",
                    Value::String(if runner == "claude" { "claude" } else { runner }.into()),
                ),
                ("args", Value::Array(vec![])),
                ("model", Value::String(String::new())),
                ("timeout_minutes", Value::Integer(15)),
                ("container.env", Value::Array(vec![])),
                ("container.files", Value::Array(vec![])),
                (
                    "container.command",
                    Value::String(if runner == "claude" { "claude" } else { runner }.into()),
                ),
            ] {
                defaults.insert(format!("agents.{runner}.{key}"), value);
            }
        }
        let document: Value =
            toml::from_str(text).with_context(|| format!("reading {}", path.display()))?;
        let mut flat = BTreeMap::new();
        flatten("", &document, &mut flat);
        let mut general = BTreeMap::new();
        let mut groups: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
        for (key, value) in flat {
            let (group, name) = if let Some(rest) = key.strip_prefix("groups.") {
                let (slug, name) = rest
                    .split_once('.')
                    .with_context(|| format!("unknown key {key} in {}", path.display()))?;
                validate_slug(slug)?;
                (Some(slug), name)
            } else {
                (None, key.as_str())
            };
            anyhow::ensure!(
                defaults.contains_key(name),
                "unknown key {key} in {}",
                path.display()
            );
            validate(name, &value)
                .with_context(|| format!("invalid key {key} in {}", path.display()))?;
            if let Some(group) = group {
                groups
                    .entry(group.into())
                    .or_default()
                    .insert(name.into(), value);
            } else {
                general.insert(name.into(), value);
            }
        }
        let settings = Self {
            path,
            general,
            groups,
            defaults,
            env,
        };
        // Validate environment even when a caller asks for only one key.
        settings.explain(None, &BTreeMap::new())?;
        Ok(settings)
    }

    pub fn groups(&self) -> impl Iterator<Item = &String> {
        self.groups.keys()
    }

    pub fn explain(
        &self,
        group: Option<&str>,
        options: &BTreeMap<String, Value>,
    ) -> anyhow::Result<Vec<Explained>> {
        for (key, value) in options {
            anyhow::ensure!(self.defaults.contains_key(key), "unknown key {key}");
            validate(key, value)?;
        }
        self.defaults
            .iter()
            .map(|(key, default)| {
                let mut value = default.clone();
                let mut source = Source::Default;
                if let Some(v) = self.general.get(key) {
                    value = v.clone();
                    source = Source::File(self.path.display().to_string());
                }
                if let Some(g) = group
                    && let Some(v) = self.groups.get(g).and_then(|values| values.get(key))
                {
                    value = v.clone();
                    source = Source::Group(g.into());
                }
                let variable = format!("STEMMA_{}", key.replace('.', "_").to_uppercase());
                if let Some(text) = self.env.get(&variable) {
                    value = parse_value(key, text)
                        .with_context(|| format!("invalid value from {variable}"))?;
                    source = Source::Environment(variable);
                }
                if let Some(v) = options.get(key) {
                    value = v.clone();
                    source = Source::CommandLine;
                }
                Ok(Explained {
                    key: key.clone(),
                    value,
                    source,
                })
            })
            .collect()
    }

    pub fn get(&self, key: &str, group: Option<&str>) -> anyhow::Result<Explained> {
        self.explain(group, &BTreeMap::new())?
            .into_iter()
            .find(|v| v.key == key)
            .with_context(|| format!("unknown key {key}"))
    }

    /// The value restored by removing this scope's override.
    pub fn inherited(&self, key: &str, group: Option<&str>) -> anyhow::Result<Explained> {
        let mut settings = self.clone();
        if let Some(group) = group {
            if let Some(values) = settings.groups.get_mut(group) {
                values.remove(key);
            }
        } else {
            settings.general.remove(key);
        }
        settings.get(key, group)
    }

    pub fn attachments(
        &self,
        group: Option<&str>,
    ) -> anyhow::Result<stemma_core::api::AttachmentRule> {
        self.get("agents.attachments", group)?
            .value
            .as_str()
            .unwrap()
            .parse()
            .map_err(anyhow::Error::msg)
    }

    pub fn workspace(&self, group: &str) -> anyhow::Result<PathBuf> {
        validate_slug(group)?;
        let value = self.get("agents.workspace", Some(group))?;
        let folder = expand_home(value.value.as_str().context("workspace must be a path")?)?;
        // Environment and command-line workspace values are general parents.
        Ok(if matches!(value.source, Source::Group(_)) {
            folder
        } else {
            folder.join(group)
        })
    }

    pub fn priority(&self, group: Option<&str>) -> anyhow::Result<i64> {
        priority(&self.get("agents.priority", group)?.value)
    }

    pub fn write(&self, key: &str, value: Option<&str>, group: Option<&str>) -> anyhow::Result<()> {
        anyhow::ensure!(self.defaults.contains_key(key), "unknown key {key}");
        if let Some(group) = group {
            validate_slug(group)?;
        }
        let parsed = value.map(|v| parse_value(key, v)).transpose()?;
        let text = read_optional(&self.path)?.unwrap_or_default();
        let mut doc: DocumentMut = text.parse()?;
        let mut segments: Vec<&str> = Vec::new();
        if let Some(group) = group {
            segments.extend(["groups", group]);
        }
        segments.extend(key.split('.'));
        let (last, parents) = segments.split_last().context("empty key")?;
        let mut table = doc.as_table_mut();
        for part in parents {
            if !table.contains_key(part) {
                if parsed.is_none() {
                    return Ok(());
                }
                let mut child = toml_edit::Table::new();
                child.set_implicit(true);
                table.insert(part, toml_edit::Item::Table(child));
            }
            table = table
                .get_mut(part)
                .and_then(toml_edit::Item::as_table_mut)
                .context("a setting's parent must be a table")?;
        }
        if let Some(value) = parsed {
            let mut replacement: toml_edit::Value = value.to_string().parse()?;
            if let Some(old) = table.get(last).and_then(toml_edit::Item::as_value) {
                *replacement.decor_mut() = old.decor().clone();
            }
            table.insert(last, toml_edit::Item::Value(replacement));
        } else {
            table.remove(last);
        }
        private_write(&self.path, doc.to_string().as_bytes())
    }
}

fn flatten(prefix: &str, value: &Value, out: &mut BTreeMap<String, Value>) {
    if let Value::Table(table) = value {
        for (key, value) in table {
            let key = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            // Empty tables still need validation.
            if value.as_table().is_some_and(|t| t.is_empty()) {
                if key == "agents"
                    || key == "groups"
                    || ["claude", "codex", "opencode"].iter().any(|r| {
                        key.ends_with(&format!("agents.{r}"))
                            || key.ends_with(&format!("agents.{r}.container"))
                    })
                    || key
                        .strip_prefix("groups.")
                        .is_some_and(|s| !s.contains('.') || s.ends_with(".agents"))
                {
                    continue;
                }
                out.insert(key, value.clone());
            } else {
                flatten(&key, value, out);
            }
        }
    } else {
        out.insert(prefix.into(), value.clone());
    }
}

pub fn priority(value: &Value) -> anyhow::Result<i64> {
    match value {
        Value::Integer(n) => Ok(*n),
        Value::String(s) => match s.as_str() {
            "high" => Ok(10),
            "normal" => Ok(0),
            "low" => Ok(-10),
            _ => anyhow::bail!("priority must be high, normal, low, or an integer"),
        },
        _ => anyhow::bail!("priority must be high, normal, low, or an integer"),
    }
}

fn parse_value(key: &str, text: &str) -> anyhow::Result<Value> {
    let value = if key.ends_with(".args") || key.ends_with(".env") {
        if text.trim_start().starts_with('[') {
            toml::from_str::<Value>(&format!("value = {text}"))?["value"].clone()
        } else {
            Value::Array(
                text.split_whitespace()
                    .map(|s| Value::String(s.into()))
                    .collect(),
            )
        }
    } else if key.ends_with(".files")
        || key.ends_with(".enabled")
        || key.ends_with(".concurrency")
        || key.ends_with(".timeout_minutes")
    {
        toml::from_str::<Value>(&format!("value = {text}"))?["value"].clone()
    } else if key == "agents.priority"
        && let Ok(n) = text.parse::<i64>()
    {
        Value::Integer(n)
    } else {
        Value::String(text.trim_matches('"').into())
    };
    validate(key, &value)?;
    Ok(value)
}

fn validate(key: &str, value: &Value) -> anyhow::Result<()> {
    if key == "agents.priority" {
        priority(value)?;
    } else if key == "agents.enabled" {
        anyhow::ensure!(value.is_bool(), "{key} must be a boolean");
    } else if key.ends_with(".concurrency") || key.ends_with(".timeout_minutes") {
        anyhow::ensure!(
            value
                .as_integer()
                .is_some_and(|n| n > 0 && n <= u32::MAX as i64),
            "{key} must be a positive integer"
        );
    } else if key.ends_with(".args") || key.ends_with(".env") {
        anyhow::ensure!(
            value
                .as_array()
                .is_some_and(|v| v.iter().all(Value::is_str)),
            "{key} must be an array of strings"
        );
        // Stemma's own sign-in never enters an agent's container.
        if key.ends_with(".container.env") {
            for name in value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                anyhow::ensure!(
                    !name.to_uppercase().starts_with("STEMMA_"),
                    "{key} cannot pass {name}: Stemma's own settings stay out of containers"
                );
            }
        }
    } else if key.ends_with(".files") {
        for file in value
            .as_array()
            .context("container files must be an array")?
        {
            let table = file
                .as_table()
                .context("a container file must have from and optionally to")?;
            anyhow::ensure!(
                table.get("from").is_some_and(Value::is_str)
                    && table.keys().all(|k| k == "from" || k == "to")
                    && table.values().all(Value::is_str),
                "invalid container file"
            );
        }
    } else {
        anyhow::ensure!(value.is_str(), "{key} must be a string");
    }
    if key == "agents.attachments" {
        value
            .as_str()
            .unwrap()
            .parse::<stemma_core::api::AttachmentRule>()
            .map_err(anyhow::Error::msg)?;
    }
    if key == "agents.isolation" {
        anyhow::ensure!(
            matches!(value.as_str(), Some("none" | "docker" | "podman")),
            "isolation must be none, docker or podman"
        );
    }
    Ok(())
}

pub fn expand_home(text: &str) -> anyhow::Result<PathBuf> {
    Ok(if let Some(rest) = text.strip_prefix("~/") {
        dirs::home_dir()
            .context("this system has no home directory")?
            .join(rest)
    } else {
        PathBuf::from(text)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Paths) {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths {
            config: temp.path().join("config"),
            state: temp.path().join("state"),
            data: temp.path().join("data"),
        };
        (temp, paths)
    }
    #[test]
    fn folders_use_xdg_on_macos_too_and_explicit_folders_win() {
        let env = BTreeMap::from([
            ("XDG_CONFIG_HOME".into(), "/xdg".into()),
            ("STEMMA_STATE_DIR".into(), "/state".into()),
        ]);
        let p = Paths::from_env(&env, Some(Path::new("/home/ada"))).unwrap();
        assert_eq!(p.config, Path::new("/xdg/stemma"));
        assert_eq!(p.state, Path::new("/state"));
        assert_eq!(p.data, Path::new("/home/ada/.local/share/stemma"));
    }
    #[test]
    fn layers_explain_priority_and_workspace() {
        let (_temp, paths) = fixture();
        let text = "[agents]\npriority = 'low'\n[agents.claude]\nmodel = 'opus'\n[groups.algebra.agents]\npriority = 'high'\nworkspace = '/research'\n[groups.algebra.agents.claude]\nmodel = 'sonnet'\n";
        let settings = Settings::parse(
            paths.config.join("config.toml"),
            text,
            &paths,
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(settings.priority(None).unwrap(), -10);
        assert_eq!(settings.priority(Some("algebra")).unwrap(), 10);
        assert_eq!(
            settings.workspace("algebra").unwrap(),
            Path::new("/research")
        );
        assert_eq!(
            settings.workspace("seminar").unwrap(),
            paths.data.join("groups/seminar")
        );
        assert_eq!(
            settings
                .get("agents.claude.model", Some("algebra"))
                .unwrap()
                .source,
            Source::Group("algebra".into())
        );
        let env = BTreeMap::from([
            ("STEMMA_AGENTS_CLAUDE_MODEL".into(), "haiku".into()),
            ("STEMMA_AGENTS_PRIORITY".into(), "37".into()),
            ("STEMMA_AGENTS_CODEX_ARGS".into(), "--search --quiet".into()),
        ]);
        let settings = Settings::parse(settings.path, text, &paths, env).unwrap();
        assert_eq!(settings.priority(Some("algebra")).unwrap(), 37);
        assert_eq!(
            settings
                .get("agents.claude.model", Some("algebra"))
                .unwrap()
                .value
                .as_str(),
            Some("haiku")
        );
        let options = BTreeMap::from([("agents.priority".into(), Value::Integer(99))]);
        assert_eq!(
            settings
                .explain(Some("algebra"), &options)
                .unwrap()
                .into_iter()
                .find(|v| v.key == "agents.priority")
                .unwrap()
                .source,
            Source::CommandLine
        );
        assert_eq!(
            settings
                .get("agents.codex.args", None)
                .unwrap()
                .value
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    #[test]
    fn rejects_unknown_keys_with_file_and_full_path() {
        let (_temp, paths) = fixture();
        for text in [
            "[agents]\nprefer = true",
            "[groups.algebra]\nmodel = 'x'",
            "[agents.unknown]",
            "[other]",
        ] {
            let error = Settings::parse(
                paths.config.join("config.toml"),
                text,
                &paths,
                BTreeMap::new(),
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains("unknown key"), "{error}");
            assert!(error.contains("config.toml"), "{error}");
        }
    }
    #[test]
    fn stemmas_sign_in_never_enters_a_container() {
        let (_temp, paths) = fixture();
        let text = "[agents.claude.container]\nenv = ['CLAUDE_CODE_OAUTH_TOKEN', 'STEMMA_TOKEN']";
        let error = Settings::parse(
            paths.config.join("config.toml"),
            text,
            &paths,
            BTreeMap::new(),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("STEMMA_TOKEN"), "{error:#}");
    }
    #[test]
    fn writes_preserve_comments_order_and_group_fallback() {
        let (_temp, paths) = fixture();
        std::fs::create_dir_all(&paths.config).unwrap();
        let path = paths.config.join("config.toml");
        std::fs::write(
            &path,
            "# My agents\n[agents]\nconcurrency = 2 # at once\npriority = 'low'\n",
        )
        .unwrap();
        let settings = Settings::load_from(&paths, BTreeMap::new()).unwrap();
        settings
            .write("agents.concurrency", Some("3"), None)
            .unwrap();
        settings
            .write("agents.claude.model", Some("sonnet"), Some("algebra"))
            .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# My agents\n[agents]\nconcurrency = 3 # at once\npriority"));
        assert_eq!(
            Settings::load_from(&paths, BTreeMap::new())
                .unwrap()
                .get("agents.claude.model", Some("algebra"))
                .unwrap()
                .value
                .as_str(),
            Some("sonnet")
        );
        settings
            .write("agents.claude.model", None, Some("algebra"))
            .unwrap();
        assert_eq!(
            Settings::load_from(&paths, BTreeMap::new())
                .unwrap()
                .get("agents.claude.model", Some("algebra"))
                .unwrap()
                .value
                .as_str(),
            Some("")
        );
    }
    #[test]
    fn credentials_are_atomic_private_and_separate_from_state() {
        let (_temp, paths) = fixture();
        let path = paths.config.join("credentials.toml");
        let credentials = Credentials {
            hub: Some("https://stemma.example.org".into()),
            token: Some("test-secret".into()),
            token_id: Some(3),
            username: Some("ada".into()),
            secrets: [(
                "CLAUDE_CODE_OAUTH_TOKEN".to_owned(),
                "sk-ant-oat01-x".to_owned(),
            )]
            .into(),
        };
        credentials.save_to(&path).unwrap();
        credentials.save_to(&path).unwrap();
        let read = Credentials::load_from(&path).unwrap();
        assert_eq!(read.token_id, Some(3));
        assert_eq!(read.secrets["CLAUDE_CODE_OAUTH_TOKEN"], "sk-ant-oat01-x");
        paths.use_group("algebra").unwrap();
        assert_eq!(
            paths
                .active_group(None, &BTreeMap::new())
                .unwrap()
                .as_deref(),
            Some("algebra")
        );
        assert!(!std::fs::read_to_string(&path).unwrap().contains("group"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(&paths.config)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }
}

#[cfg(test)]
mod attachment_tests {
    use super::*;
    #[test]
    fn attachment_rules_resolve_all_layers_and_validate_values() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths {
            config: temp.path().join("config"),
            state: temp.path().join("state"),
            data: temp.path().join("data"),
        };
        let parse =
            |text, env| Settings::parse(paths.config.join("config.toml"), text, &paths, env);
        use stemma_core::api::AttachmentRule as Rule;
        assert_eq!(
            parse("", BTreeMap::new())
                .unwrap()
                .attachments(None)
                .unwrap(),
            Rule::Mine
        );
        let text = "[agents]\nattachments = 'none'\n[groups.algebra.agents]\nattachments = 'group'";
        let settings = parse(text, BTreeMap::new()).unwrap();
        assert_eq!(settings.attachments(None).unwrap(), Rule::None);
        assert_eq!(settings.attachments(Some("algebra")).unwrap(), Rule::Group);
        assert_eq!(
            settings
                .inherited("agents.attachments", Some("algebra"))
                .unwrap()
                .value
                .as_str(),
            Some("none")
        );
        assert_eq!(
            settings
                .inherited("agents.attachments", Option::None)
                .unwrap()
                .value
                .as_str(),
            Some("mine")
        );
        let settings = parse(
            text,
            [("STEMMA_AGENTS_ATTACHMENTS".into(), "mine".into())].into(),
        )
        .unwrap();
        assert_eq!(settings.attachments(Some("algebra")).unwrap(), Rule::Mine);
        assert!(parse("[agents]\nattachments = 'all'", BTreeMap::new()).is_err());
    }
}

impl Paths {
    /// Reconcile names by stable ID. A busy workspace is retried on the next
    /// hub exchange; a pre-existing destination is never overwritten.
    pub fn reconcile_groups(
        &self,
        hub: &str,
        groups: &[stemma_core::api::Group],
    ) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.state)?;
        let guard = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.state.join("names.lock"))?;
        guard.lock()?;
        let registry = self.state.join("group-names.toml");
        let mut known: BTreeMap<String, BTreeMap<String, String>> = read_optional(&registry)?
            .map(|s| toml::from_str(&s))
            .transpose()?
            .unwrap_or_default();
        let names = known.entry(hub.into()).or_default();
        for group in groups {
            validate_slug(&group.slug)?;
            let key = group.id.to_string();
            if let Some(old) = names.get(&key).filter(|old| *old != &group.slug) {
                validate_slug(old)?;
                let settings_path = self.config.join("config.toml");
                let text = read_optional(&settings_path)?.unwrap_or_default();
                let mut doc = text.parse::<DocumentMut>()?;
                if doc.get("groups").and_then(|v| v.get(&group.slug)).is_some() {
                    anyhow::bail!(
                        "Cannot rename {old} to {}: settings already exist for that name",
                        group.slug
                    );
                }
                let settings = Settings::parse(
                    settings_path.clone(),
                    &text,
                    self,
                    std::env::vars().collect(),
                )?;
                let from = settings.workspace(old)?;
                let mut to = settings.workspace(&group.slug)?;
                if matches!(
                    settings.get("agents.workspace", Some(old))?.source,
                    Source::Group(_)
                ) {
                    // An exact custom workspace is the person's chosen path.
                    to = from.clone();
                }
                let locks = self.state.join("groups");
                std::fs::create_dir_all(&locks)?;
                let open = |slug: &str, suffix: &str| {
                    std::fs::OpenOptions::new()
                        .create(true)
                        .truncate(false)
                        .read(true)
                        .write(true)
                        .open(locks.join(format!("{slug}{suffix}")))
                };
                let update = open(&key, ".update.lock")?;
                let lease = open(&key, ".lock")?;
                if update.try_lock().is_err() || lease.try_lock().is_err() {
                    continue;
                }
                if from != to && from.exists() {
                    anyhow::ensure!(
                        !to.exists(),
                        "Cannot rename {}: {} already exists",
                        from.display(),
                        to.display()
                    );
                    if let Some(parent) = to.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::rename(&from, &to)?;
                }
                if let Some(table) = doc.get_mut("groups").and_then(|g| g.as_table_mut())
                    && let Some(value) = table.remove(old)
                {
                    table.insert(&group.slug, value);
                    private_write(&settings_path, doc.to_string().as_bytes())?;
                }
                if read_optional(&self.state.join("group"))?.is_some_and(|s| s.trim() == old) {
                    self.use_group(&group.slug)?;
                }
            }
            names.insert(key, group.slug.clone());
        }
        private_write(&registry, toml::to_string(&known)?.as_bytes())
    }
    /// A busy group's workspace keeps its last local name until it can move.
    pub fn group_slug(&self, hub: &str, id: i64, current: &str) -> anyhow::Result<String> {
        let known: BTreeMap<String, BTreeMap<String, String>> =
            read_optional(&self.state.join("group-names.toml"))?
                .map(|s| toml::from_str(&s))
                .transpose()?
                .unwrap_or_default();
        Ok(known
            .get(hub)
            .and_then(|groups| groups.get(&id.to_string()))
            .cloned()
            .unwrap_or_else(|| current.into()))
    }
    pub fn update_username(&self, hub: &str, token: &str, username: &str) -> anyhow::Result<()> {
        let path = self.config.join("credentials.toml");
        let mut credentials = Credentials::load_from(&path)?;
        if credentials
            .hub
            .as_deref()
            .is_some_and(|url| url.trim_end_matches('/') == hub.trim_end_matches('/'))
            && credentials.token.as_deref() == Some(token)
            && credentials.username.as_deref() != Some(username)
        {
            credentials.username = Some(username.into());
            credentials.save_to(&path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod name_tests {
    use super::*;
    #[test]
    fn renaming_a_group_preserves_settings_and_work_and_refuses_overwrite() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths {
            config: root.path().join("config"),
            state: root.path().join("state"),
            data: root.path().join("data"),
        };
        let mut group = stemma_core::api::Group {
            id: 3,
            slug: "algebra".into(),
            name: "Algebra".into(),
            role: stemma_core::api::Role::Admin,
            created_at: time::OffsetDateTime::UNIX_EPOCH,
        };
        paths
            .reconcile_groups("http://test", &[group.clone()])
            .unwrap();
        std::fs::create_dir_all(&paths.config).unwrap();
        std::fs::write(
            paths.config.join("config.toml"),
            "# Keep this comment\n[groups.algebra.agents]\npriority = 'high'\n",
        )
        .unwrap();
        std::fs::create_dir_all(paths.data.join("groups/algebra/library/.lake")).unwrap();
        std::fs::write(
            paths.data.join("groups/algebra/library/.lake/cache"),
            "keep",
        )
        .unwrap();
        paths.use_group("algebra").unwrap();
        group.slug = "geometry".into();
        std::fs::create_dir_all(paths.state.join("groups")).unwrap();
        let lease = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(paths.state.join("groups/3.lock"))
            .unwrap();
        lease.lock_shared().unwrap();
        paths
            .reconcile_groups("http://test", &[group.clone()])
            .unwrap();
        assert_eq!(
            paths.group_slug("http://test", 3, "geometry").unwrap(),
            "algebra"
        );
        assert!(
            paths
                .data
                .join("groups/algebra/library/.lake/cache")
                .exists()
        );
        drop(lease);
        std::fs::create_dir_all(paths.data.join("groups/geometry")).unwrap();
        assert!(
            paths
                .reconcile_groups("http://test", &[group.clone()])
                .is_err()
        );
        assert!(
            paths
                .data
                .join("groups/algebra/library/.lake/cache")
                .exists()
        );
        std::fs::remove_dir(paths.data.join("groups/geometry")).unwrap();
        paths.reconcile_groups("http://test", &[group]).unwrap();
        assert_eq!(
            std::fs::read_to_string(paths.data.join("groups/geometry/library/.lake/cache"))
                .unwrap(),
            "keep"
        );
        let text = std::fs::read_to_string(paths.config.join("config.toml")).unwrap();
        assert!(text.contains("# Keep this comment"));
        assert!(text.contains("groups.geometry.agents"), "{text}");
        assert!(!text.contains("groups.algebra"));
        assert_eq!(
            std::fs::read_to_string(paths.state.join("group")).unwrap(),
            "geometry\n"
        );
    }
    #[test]
    fn username_updates_only_the_matching_credentials() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths {
            config: root.path().join("config"),
            state: root.path().join("state"),
            data: root.path().join("data"),
        };
        let file = paths.config.join("credentials.toml");
        Credentials {
            hub: Some("http://test".into()),
            token: Some("test-token".into()),
            username: Some("ada".into()),
            ..Default::default()
        }
        .save_to(&file)
        .unwrap();
        paths
            .update_username("http://other", "test-token", "wrong")
            .unwrap();
        assert_eq!(
            Credentials::load_from(&file).unwrap().username.as_deref(),
            Some("ada")
        );
        paths
            .update_username("http://test", "test-token", "augusta")
            .unwrap();
        assert_eq!(
            Credentials::load_from(&file).unwrap().username.as_deref(),
            Some("augusta")
        );
    }
}
