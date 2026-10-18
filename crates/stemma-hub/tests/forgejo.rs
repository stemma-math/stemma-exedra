//! The hub with a real Forgejo, end to end: groups made as organizations,
//! teams kept as roles, git through the hub, builds after pushes, and
//! people signing in to Forgejo through the hub.
//!
//! Skipped unless `STEMMA_TEST_FORGEJO` is Forgejo's address, set up as
//! `docker/setup.sh` sets it up: its secrets in
//! the hub's variables (`STEMMA_FORGE_TOKEN`, `STEMMA_FORGE_WEBHOOK_SECRET`,
//! `STEMMA_FORGE_CLIENT_SECRET`), its source `stemma` (of id
//! `STEMMA_TEST_FORGEJO_SOURCE`, 1 if unset) discovering the hub at
//! `STEMMA_TEST_HUB` (`http://127.0.0.1:8080` if unset), where the test
//! runs a hub of its own: nothing else may listen there meanwhile. Each run
//! makes accounts and a group of its own, and removes the group's
//! organization at the end.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{COOKIE, LOCATION, SET_COOKIE};
use reqwest::redirect::Policy;
use serde_json::{Value, json};
use stemma_core::api::NewAccount;
use stemma_hub::config::{ForgeConfig, ForgeSecrets};

struct Setting {
    forge: String,
    hub: String,
    source_id: i64,
    secrets: ForgeSecrets,
}

fn setting() -> Option<Setting> {
    let forge = std::env::var("STEMMA_TEST_FORGEJO").ok()?;
    let var = |name: &str| {
        std::env::var(name).unwrap_or_else(|_| panic!("STEMMA_TEST_FORGEJO needs {name}"))
    };
    Some(Setting {
        forge: forge.trim_end_matches('/').to_owned(),
        hub: std::env::var("STEMMA_TEST_HUB")
            .unwrap_or_else(|_| "http://127.0.0.1:8080".into())
            .trim_end_matches('/')
            .to_owned(),
        source_id: std::env::var("STEMMA_TEST_FORGEJO_SOURCE")
            .map(|id| id.parse().expect("a source's id"))
            .unwrap_or(1),
        secrets: ForgeSecrets {
            token: var("STEMMA_FORGE_TOKEN"),
            webhook_secret: var("STEMMA_FORGE_WEBHOOK_SECRET"),
            client_secret: var("STEMMA_FORGE_CLIENT_SECRET"),
        },
    })
}

/// A browser, with its cookies, that does not follow redirects by itself.
struct Browser {
    http: reqwest::Client,
    cookies: HashMap<String, HashMap<String, String>>,
}

impl Browser {
    fn new() -> Browser {
        Browser {
            http: reqwest::Client::builder()
                .redirect(Policy::none())
                .build()
                .unwrap(),
            cookies: HashMap::new(),
        }
    }

    async fn get(&mut self, url: &str) -> reqwest::Response {
        let parsed = url::Url::parse(url).unwrap();
        let origin = parsed.origin().ascii_serialization();
        let mut request = self.http.get(url);
        if let Some(jar) = self.cookies.get(&origin) {
            let header: Vec<String> = jar.iter().map(|(k, v)| format!("{k}={v}")).collect();
            request = request.header(COOKIE, header.join("; "));
        }
        let response = request.send().await.unwrap();
        for value in response.headers().get_all(SET_COOKIE) {
            let pair = value.to_str().unwrap().split(';').next().unwrap();
            if let Some((name, value)) = pair.split_once('=') {
                self.cookies
                    .entry(origin.clone())
                    .or_default()
                    .insert(name.trim().into(), value.trim().into());
            }
        }
        response
    }

    /// Follows redirects from `url`, wherever they go; returns the last
    /// answer and its address.
    async fn follow(&mut self, url: &str) -> (String, reqwest::Response) {
        let mut url = url.to_owned();
        for _ in 0..10 {
            let response = self.get(&url).await;
            let Some(location) = response.headers().get(LOCATION) else {
                return (url, response);
            };
            let next = url::Url::parse(&url)
                .unwrap()
                .join(location.to_str().unwrap())
                .unwrap();
            url = next.to_string();
        }
        panic!("too many redirects from {url}");
    }

    /// Signs in to the hub, as the web app does.
    async fn sign_in_to_hub(&mut self, hub: &str, username: &str) {
        let response = self
            .http
            .post(format!("{hub}/api/v1/session"))
            .json(&json!({"username": username, "password": "password1"}))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        let pair = response.headers()[SET_COOKIE].to_str().unwrap();
        let (name, value) = pair.split(';').next().unwrap().split_once('=').unwrap();
        let origin = url::Url::parse(hub).unwrap().origin().ascii_serialization();
        self.cookies
            .entry(origin)
            .or_default()
            .insert(name.into(), value.into());
    }
}

async fn person(state: &stemma_hub::AppState, hub: &str, username: &str, admin: bool) -> String {
    let account = NewAccount {
        username: username.into(),
        display_name: username.into(),
        password: "password1".into(),
    };
    stemma_hub::api::create_user(state, account, admin)
        .await
        .unwrap();
    let mut browser = Browser::new();
    browser.sign_in_to_hub(hub, username).await;
    let origin = url::Url::parse(hub).unwrap().origin().ascii_serialization();
    let cookie: Vec<String> = browser.cookies[&origin]
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    let created: Value = reqwest::Client::new()
        .post(format!("{hub}/api/v1/me/tokens"))
        .header(COOKIE, cookie.join("; "))
        .json(&json!({"name": "tests"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    created["secret"].as_str().unwrap().to_owned()
}

async fn call(hub: &str, token: &str, method: &str, path: &str, body: Option<Value>) -> Value {
    let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = reqwest::Client::new()
        .request(method, format!("{hub}/api/v1{path}"))
        .bearer_auth(token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    let value = response.json().await.unwrap_or(Value::Null);
    assert!(status.is_success(), "{path}: {status} {value}");
    value
}

/// Forgejo's API, as its admin.
async fn forge_api(setting: &Setting, method: &str, path: &str) -> (u16, Value) {
    let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
    let response = reqwest::Client::new()
        .request(method, format!("{}/api/v1{path}", setting.forge))
        .header("authorization", format!("token {}", setting.secrets.token))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, response.json().await.unwrap_or(Value::Null))
}

async fn team_members(setting: &Setting, org: &str, team: &str) -> Vec<String> {
    let (_, teams) = forge_api(setting, "GET", &format!("/orgs/{org}/teams")).await;
    let id = teams
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == team)
        .unwrap_or_else(|| panic!("no team {team} in {teams}"))["id"]
        .clone();
    let (_, members) = forge_api(setting, "GET", &format!("/teams/{id}/members")).await;
    let mut logins: Vec<String> = members
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["login"].as_str().unwrap().to_owned())
        .collect();
    logins.sort();
    logins
}

/// Signs in to Forgejo through the hub, where the browser is signed in;
/// returns whom Forgejo then takes it for, from the account's settings.
async fn signed_in_as(browser: &mut Browser, setting: &Setting) -> String {
    let (landed, response) = browser
        .follow(&format!("{}/user/oauth2/stemma", setting.forge))
        .await;
    assert!(
        landed.starts_with(&setting.forge) && response.status() == 200,
        "{landed}: {}",
        response.status()
    );
    let settings = browser
        .get(&format!("{}/user/settings", setting.forge))
        .await;
    assert_eq!(settings.status(), 200, "not signed in to Forgejo");
    let page = settings.text().await.unwrap();
    // The username field of the settings' form.
    let field = page
        .split("name=\"name\"")
        .nth(1)
        .expect("a username field");
    let value = field.split("value=\"").nth(1).expect("a value");
    value.split('"').next().unwrap().to_owned()
}

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=Ada", "-c", "user.email=ada@example.org"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn with_forgejo() {
    let Some(setting) = setting() else {
        eprintln!("STEMMA_TEST_FORGEJO is not set: skipping");
        return;
    };
    let _ = rustls::crypto::ring::default_provider().install_default();
    let run = &stemma_hub::secret::token("")[..6]
        .to_lowercase()
        .replace(['-', '_'], "x");
    let slug = format!("e2e-{run}");
    let (ada, bob, gus, eve) = (
        format!("ada-{run}"),
        format!("bob-{run}"),
        format!("gus-{run}"),
        format!("eve-{run}"),
    );

    // The hub, where the forge's source expects it.
    let data = tempfile::tempdir().unwrap();
    let mut forge_config = ForgeConfig::new(
        &setting.forge,
        &format!("{}/user/oauth2/stemma/callback", setting.forge),
        setting.secrets.clone(),
    );
    forge_config.auth_source_id = setting.source_id;
    let config = stemma_hub::Config {
        data_dir: data.path().to_owned(),
        listen: url::Url::parse(&setting.hub)
            .unwrap()
            .socket_addrs(|| None)
            .unwrap()[0],
        public_url: Some(setting.hub.clone()),
        forge: Some(forge_config),
        ..Default::default()
    };
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .expect("nothing else listens where the hub must");
    let state =
        Arc::new(stemma_hub::Hub::new(stemma_hub::db::in_memory().await.unwrap(), config).unwrap());
    let app = stemma_hub::app(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await });
    let hub = setting.hub.as_str();

    // A group is an organization, with its teams and its library.
    let ada_token = person(&state, hub, &ada, true).await;
    call(
        hub,
        &ada_token,
        "POST",
        "/groups",
        Some(json!({"slug": slug, "name": "End to end"})),
    )
    .await;
    let (status, _) = forge_api(&setting, "GET", &format!("/orgs/{slug}")).await;
    assert_eq!(status, 200);
    let (status, branch) = forge_api(
        &setting,
        "GET",
        &format!("/repos/{slug}/library/branches/main"),
    )
    .await;
    assert_eq!(status, 200, "{branch}");
    let first = branch["commit"]["id"].as_str().unwrap().to_owned();
    let library = call(
        hub,
        &ada_token,
        "GET",
        &format!("/groups/{slug}/library"),
        None,
    )
    .await;
    assert_eq!(library["head"], first.as_str());
    let owners = team_members(&setting, &slug, "Owners").await;
    assert!(owners.contains(&ada), "{owners:?} {ada}");

    // Roles become teams.
    let bob_token = person(&state, hub, &bob, false).await;
    let gus_token = person(&state, hub, &gus, false).await;
    for (who, role) in [(&bob_token, "member"), (&gus_token, "guest")] {
        let invitation = call(
            hub,
            &ada_token,
            "POST",
            &format!("/groups/{slug}/invitations"),
            Some(json!({"role": role})),
        )
        .await;
        call(
            hub,
            who,
            "POST",
            &format!(
                "/invitations/{}/accept",
                invitation["token"].as_str().unwrap()
            ),
            Some(json!({})),
        )
        .await;
    }
    assert_eq!(
        team_members(&setting, &slug, "Members").await,
        [bob.as_str()]
    );
    assert_eq!(
        team_members(&setting, &slug, "Guests").await,
        [gus.as_str()]
    );

    // Git goes through the hub; a push to main is built.
    let work = tempfile::tempdir().unwrap();
    let host = hub.trim_start_matches("http://");
    let url = format!("http://{ada}:{ada_token}@{host}/git/{slug}/library.git");
    let cloned = git(work.path(), &["clone", "-q", &url, "library"]);
    assert!(cloned.status.success(), "{cloned:?}");
    let dir = work.path().join("library");
    assert!(dir.join("lakefile.toml").exists());
    std::fs::write(dir.join("NOTES.md"), "Notes.\n").unwrap();
    git(&dir, &["add", "NOTES.md"]);
    git(&dir, &["commit", "-qm", "Notes"]);
    let pushed = git(&dir, &["push", "-q", "origin", "main"]);
    assert!(pushed.status.success(), "{pushed:?}");
    let second = String::from_utf8(git(&dir, &["rev-parse", "HEAD"]).stdout).unwrap();
    let second = second.trim();
    let mut builds = Value::Null;
    for _ in 0..100 {
        builds = call(
            hub,
            &ada_token,
            "GET",
            &format!("/groups/{slug}/library/builds"),
            None,
        )
        .await;
        if builds[0]["commit"] == second {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        builds[0]["commit"], second,
        "the webhook did not reach the hub"
    );
    // History is kept.
    git(&dir, &["reset", "-q", "--hard", "HEAD~1"]);
    let forced = git(&dir, &["push", "-q", "--force", "origin", "main"]);
    assert!(!forced.status.success());
    // Guests do not push.
    let url = format!("http://{gus}:{gus_token}@{host}/git/{slug}/library.git");
    let cloned = git(work.path(), &["clone", "-q", &url, "guest"]);
    assert!(cloned.status.success(), "{cloned:?}");
    let guest = work.path().join("guest");
    std::fs::write(guest.join("MINE.md"), "Mine.\n").unwrap();
    git(&guest, &["add", "MINE.md"]);
    git(&guest, &["commit", "-qm", "Mine"]);
    assert!(
        !git(&guest, &["push", "-q", "origin", "main"])
            .status
            .success()
    );

    // Bob signs in to Forgejo through the hub, and lands on his account,
    // which sees the library.
    let mut browser = Browser::new();
    browser.sign_in_to_hub(hub, &bob).await;
    assert_eq!(signed_in_as(&mut browser, &setting).await, bob);
    let repo = browser
        .get(&format!("{}/{slug}/library", setting.forge))
        .await;
    assert_eq!(repo.status(), 200);

    // Eve, outside the group, signs in too, and sees nothing of it.
    person(&state, hub, &eve, false).await;
    let mut browser = Browser::new();
    browser.sign_in_to_hub(hub, &eve).await;
    assert_eq!(signed_in_as(&mut browser, &setting).await, eve);
    let repo = browser
        .get(&format!("{}/{slug}/library", setting.forge))
        .await;
    assert_eq!(repo.status(), 404);

    // Leaving the group leaves its teams.
    let bob_id = call(hub, &bob_token, "GET", "/me", None).await["id"].clone();
    call(
        hub,
        &bob_token,
        "DELETE",
        &format!("/groups/{slug}/members/{bob_id}"),
        None,
    )
    .await;
    assert!(team_members(&setting, &slug, "Members").await.is_empty());

    forge_api(&setting, "DELETE", &format!("/repos/{slug}/library")).await;
    forge_api(&setting, "DELETE", &format!("/orgs/{slug}")).await;
}
