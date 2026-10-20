//! The API, end to end, against a fresh database.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use stemma_fake_forge::FakeForge;
use stemma_hub::config::{ForgeConfig, ForgeSecrets};
use stemma_hub::{Config, Hub, app, db};
use tower::ServiceExt;

struct TestHub {
    app: Router,
    state: stemma_hub::AppState,
    forge: FakeForge,
    _data: tempfile::TempDir,
}

/// Where Forgejo sends people back to after they sign in.
const CALLBACK: &str = "http://forge.test/user/oauth2/stemma/callback";
const CLIENT_SECRET: &str = "the client's secret";
const WEBHOOK_SECRET: &str = "the webhook's secret";

/// A person using the API, with their session cookie or token.
#[derive(Clone, Default)]
struct Client {
    cookie: Option<String>,
    token: Option<String>,
}

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

impl TestHub {
    async fn new() -> TestHub {
        let forge = FakeForge::start().await;
        let config = Config {
            forge: Some(ForgeConfig::new(
                &forge.url,
                CALLBACK,
                ForgeSecrets {
                    token: stemma_fake_forge::TOKEN.into(),
                    webhook_secret: WEBHOOK_SECRET.into(),
                    client_secret: CLIENT_SECRET.into(),
                },
            )),
            ..Config::default()
        };
        Self::with(config, forge).await
    }

    /// A hub with `config`, in a folder of its own.
    async fn with(config: Config, forge: FakeForge) -> TestHub {
        let data = tempfile::tempdir().unwrap();
        let config = Config {
            data_dir: data.path().to_owned(),
            max_upload_mb: 1,
            ..config
        };
        let hub = Arc::new(Hub::new(db::in_memory().await.unwrap(), config).unwrap());
        *hub.setup_token.lock().unwrap() = Some("setup-secret".into());
        TestHub {
            app: app(hub.clone()),
            state: hub,
            forge,
            _data: data,
        }
    }

    async fn send(&self, client: &Client, request: Request<Body>) -> Reply {
        let mut request = request;
        if let Some(cookie) = &client.cookie {
            request
                .headers_mut()
                .insert(header::COOKIE, cookie.parse().unwrap());
        }
        if let Some(token) = &client.token {
            request.headers_mut().insert(
                header::AUTHORIZATION,
                format!("Bearer {token}").parse().unwrap(),
            );
        }
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            body: body.to_vec(),
        }
    }

    async fn call(
        &self,
        client: &Client,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Reply {
        let builder = Request::builder()
            .method(method)
            .uri(format!("/api/v1{path}"));
        let request = match body {
            Some(body) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string())),
            None => builder.body(Body::empty()),
        }
        .unwrap();
        self.send(client, request).await
    }

    /// A GET of a whole path, outside the API's prefix too.
    async fn fetch(&self, client: &Client, path: &str) -> Reply {
        self.send(client, Request::get(path).body(Body::empty()).unwrap())
            .await
    }

    async fn get(&self, client: &Client, path: &str) -> Reply {
        self.call(client, Method::GET, path, None).await
    }

    async fn post(&self, client: &Client, path: &str, body: Value) -> Reply {
        self.call(client, Method::POST, path, Some(body)).await
    }

    /// The first account, through the setup token, and a group it admins.
    async fn admin_with_group(&self) -> Client {
        let reply = self
            .post(
                &Client::default(),
                "/setup",
                json!({"token": "setup-secret", "username": "ada", "display_name": "Ada", "password": "analytical"}),
            )
            .await;
        assert_eq!(reply.status, StatusCode::CREATED);
        let admin = signed_in(&reply);
        let reply = self
            .post(
                &admin,
                "/groups",
                json!({"slug": "geometry", "name": "Geometry"}),
            )
            .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.json());
        admin
    }

    /// A new member of `geometry`, through an invitation from `admin`.
    async fn member(&self, admin: &Client, username: &str) -> Client {
        self.join(admin, username, "member").await
    }

    /// Someone new in `geometry` with `role`, through an invitation from
    /// `admin`.
    async fn join(&self, admin: &Client, username: &str, role: &str) -> Client {
        let reply = self
            .post(
                admin,
                "/groups/geometry/invitations",
                json!({"max_uses": 1, "role": role}),
            )
            .await;
        assert_eq!(reply.status, StatusCode::CREATED);
        let token = reply.json()["token"].as_str().unwrap().to_owned();
        let account = json!({"account": {"username": username, "display_name": username, "password": "password1"}});
        let reply = self
            .post(
                &Client::default(),
                &format!("/invitations/{token}/accept"),
                account,
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());
        signed_in(&reply)
    }
}

fn signed_in(reply: &Reply) -> Client {
    let set_cookie = reply.headers[header::SET_COOKIE].to_str().unwrap();
    assert!(set_cookie.contains("HttpOnly"));
    Client {
        cookie: Some(set_cookie.split(';').next().unwrap().to_owned()),
        token: None,
    }
}

#[tokio::test]
async fn health() {
    let hub = TestHub::new().await;
    let reply = hub.get(&Client::default(), "/health").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["status"], "ok");
    assert_eq!(
        hub.get(&Client::default(), "/nowhere").await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn setup_creates_the_first_admin_once() {
    let hub = TestHub::new().await;
    let anyone = Client::default();
    assert_eq!(hub.get(&anyone, "/setup").await.json()["needed"], true);
    assert_eq!(
        hub.get(&anyone, "/me").await.status,
        StatusCode::UNAUTHORIZED
    );

    let account = json!({"token": "wrong", "username": "ada", "display_name": "Ada", "password": "analytical"});
    assert_eq!(
        hub.post(&anyone, "/setup", account).await.status,
        StatusCode::FORBIDDEN
    );

    let admin = hub.admin_with_group().await;
    let me = hub.get(&admin, "/me").await.json();
    assert_eq!(me["username"], "ada");
    assert_eq!(me["hub_admin"], true);
    assert_eq!(me["groups"][0]["slug"], "geometry");
    assert_eq!(me["groups"][0]["role"], "admin");
    assert_eq!(hub.get(&anyone, "/setup").await.json()["needed"], false);

    let again = json!({"token": "setup-secret", "username": "eve", "display_name": "Eve", "password": "analytical"});
    assert_eq!(
        hub.post(&anyone, "/setup", again).await.status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn signing_in_and_out() {
    let hub = TestHub::new().await;
    hub.admin_with_group().await;
    let anyone = Client::default();

    let wrong = hub
        .post(
            &anyone,
            "/session",
            json!({"username": "ada", "password": "nope"}),
        )
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    let unknown = hub
        .post(
            &anyone,
            "/session",
            json!({"username": "bob", "password": "nope"}),
        )
        .await;
    assert_eq!(unknown.status, StatusCode::UNAUTHORIZED);

    let reply = hub
        .post(
            &anyone,
            "/session",
            json!({"username": "ADA", "password": "analytical"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let ada = signed_in(&reply);
    assert_eq!(hub.get(&ada, "/me").await.status, StatusCode::OK);

    let reply = hub.call(&ada, Method::DELETE, "/session", None).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert_eq!(hub.get(&ada, "/me").await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn invitations_bring_people_in() {
    let hub = TestHub::new().await;
    let admin = hub.admin_with_group().await;

    let reply = hub
        .post(
            &admin,
            "/groups/geometry/invitations",
            json!({"max_uses": 1}),
        )
        .await;
    let token = reply.json()["token"].as_str().unwrap().to_owned();
    let info = hub
        .get(&Client::default(), &format!("/invitations/{token}"))
        .await;
    assert_eq!(info.json()["group_name"], "Geometry");

    // Without an account or a session, accepting needs one.
    let reply = hub
        .post(
            &Client::default(),
            &format!("/invitations/{token}/accept"),
            json!({}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);

    let account =
        json!({"account": {"username": "bob", "display_name": "Bob", "password": "short"}});
    let reply = hub
        .post(
            &Client::default(),
            &format!("/invitations/{token}/accept"),
            account,
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);

    let account =
        json!({"account": {"username": "bob", "display_name": "Bob", "password": "long enough"}});
    let reply = hub
        .post(
            &Client::default(),
            &format!("/invitations/{token}/accept"),
            account.clone(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let bob = signed_in(&reply);
    assert_eq!(reply.json()["groups"][0]["role"], "member");

    // Used up.
    let reply = hub
        .post(
            &Client::default(),
            &format!("/invitations/{token}/accept"),
            account,
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(
        hub.get(&admin, "/groups/geometry/invitations")
            .await
            .json()
            .as_array()
            .unwrap()
            .is_empty()
    );

    // Members cannot invite, nor create groups.
    let reply = hub
        .post(&bob, "/groups/geometry/invitations", json!({}))
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let reply = hub
        .post(&bob, "/groups", json!({"slug": "mine", "name": "Mine"}))
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);

    let members = hub.get(&bob, "/groups/geometry/members").await.json();
    assert_eq!(members.as_array().unwrap().len(), 2);

    // A revoked invitation does not work.
    let reply = hub
        .post(&admin, "/groups/geometry/invitations", json!({}))
        .await;
    let (id, token) = (
        reply.json()["id"].as_i64().unwrap(),
        reply.json()["token"].as_str().unwrap().to_owned(),
    );
    let reply = hub
        .call(
            &admin,
            Method::DELETE,
            &format!("/groups/geometry/invitations/{id}"),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert_eq!(
        hub.get(&Client::default(), &format!("/invitations/{token}"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn the_last_admin_stays() {
    let hub = TestHub::new().await;
    let admin = hub.admin_with_group().await;
    let bob = hub.member(&admin, "bob").await;
    let bob_id = hub.get(&bob, "/me").await.json()["id"].as_i64().unwrap();
    let ada_id = hub.get(&admin, "/me").await.json()["id"].as_i64().unwrap();

    let demote = hub
        .call(
            &admin,
            Method::PATCH,
            &format!("/groups/geometry/members/{ada_id}"),
            Some(json!({"role": "member"})),
        )
        .await;
    assert_eq!(demote.status, StatusCode::CONFLICT);

    let promote = hub
        .call(
            &admin,
            Method::PATCH,
            &format!("/groups/geometry/members/{bob_id}"),
            Some(json!({"role": "admin"})),
        )
        .await;
    assert_eq!(promote.status, StatusCode::NO_CONTENT);
    let leave = hub
        .call(
            &admin,
            Method::DELETE,
            &format!("/groups/geometry/members/{ada_id}"),
            None,
        )
        .await;
    assert_eq!(leave.status, StatusCode::NO_CONTENT);
    assert_eq!(
        hub.get(&admin, "/groups/geometry").await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn threads_and_messages_are_numbered_for_good() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let bob = hub.member(&ada, "bob").await;

    let reply = hub
        .post(
            &ada,
            "/groups/geometry/threads",
            json!({"title": "Compactification", "body": "Where do we glue? $X \\cup Y$"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let page = reply.json();
    assert_eq!(page["thread"]["number"], 1);
    assert_eq!(page["thread"]["kind"], "discussion");
    assert_eq!(page["messages"][0]["number"], 1);

    let reply = hub
        .post(
            &bob,
            "/groups/geometry/threads",
            json!({"title": "Notation", "kind": "chat", "resident_agent": "@claude"}),
        )
        .await;
    assert_eq!(reply.json()["thread"]["number"], 2);
    assert_eq!(reply.json()["thread"]["resident_agent"], "claude");
    assert!(reply.json()["messages"].as_array().unwrap().is_empty());

    let reply = hub
        .post(
            &ada,
            "/groups/geometry/threads",
            json!({"title": "x", "resident_agent": "claude"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);

    for (who, text) in [
        (&bob, "Along the boundary."),
        (&ada, "Agreed."),
        (&bob, "See [[t1#1]]."),
    ] {
        let reply = hub
            .post(
                who,
                "/groups/geometry/threads/1/messages",
                json!({"body": text}),
            )
            .await;
        assert_eq!(reply.status, StatusCode::CREATED);
    }
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/threads/1/messages",
            json!({"body": "Yes", "reply_to": 4}),
        )
        .await;
    assert_eq!(reply.json()["number"], 5);
    assert_eq!(reply.json()["reply_to"], 4);
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/threads/1/messages",
            json!({"body": "?", "reply_to": 9}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/threads/1/messages",
            json!({"body": "  "}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);

    // Editing: only the author; the number stays.
    let reply = hub
        .call(
            &ada,
            Method::PATCH,
            "/groups/geometry/threads/1/messages/2",
            Some(json!({"body": "Hijacked"})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let reply = hub
        .call(
            &bob,
            Method::PATCH,
            "/groups/geometry/threads/1/messages/2",
            Some(json!({"body": "Along the boundary, smoothly."})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["number"], 2);
    assert!(reply.json()["edited_at"].is_string());

    // Deleting: the author or an admin; the number is never reused.
    let reply = hub
        .call(
            &bob,
            Method::DELETE,
            "/groups/geometry/threads/1/messages/3",
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let reply = hub
        .call(
            &ada,
            Method::DELETE,
            "/groups/geometry/threads/1/messages/5",
            None,
        )
        .await;
    assert_eq!(reply.json()["deleted"], true);
    assert_eq!(reply.json()["body"], "");
    let reply = hub
        .post(
            &bob,
            "/groups/geometry/threads/1/messages",
            json!({"body": "After the deletion"}),
        )
        .await;
    assert_eq!(reply.json()["number"], 6);

    let page = hub.get(&bob, "/groups/geometry/threads/1").await.json();
    let numbers: Vec<i64> = page["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["number"].as_i64().unwrap())
        .collect();
    assert_eq!(numbers, [1, 2, 3, 4, 5, 6]);
    assert_eq!(page["thread"]["last_message"], 6);

    let range = hub
        .get(&bob, "/groups/geometry/threads/1/messages?from=2&to=4")
        .await
        .json();
    assert_eq!(range.as_array().unwrap().len(), 3);
    let one = hub
        .get(&bob, "/groups/geometry/threads/1/messages/4")
        .await
        .json();
    assert_eq!(one["body"], "See [[t1#1]].");

    // Most recently active first.
    let threads = hub.get(&bob, "/groups/geometry/threads").await.json();
    assert_eq!(threads[0]["number"], 1);
    assert_eq!(threads[1]["number"], 2);
    let chats = hub
        .get(&bob, "/groups/geometry/threads?kind=chat")
        .await
        .json();
    assert_eq!(chats.as_array().unwrap().len(), 1);

    let reply = hub
        .call(
            &bob,
            Method::PATCH,
            "/groups/geometry/threads/1",
            Some(json!({"title": "Mine now"})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let reply = hub
        .call(
            &ada,
            Method::PATCH,
            "/groups/geometry/threads/1",
            Some(json!({"title": "Compactifications"})),
        )
        .await;
    assert_eq!(reply.json()["title"], "Compactifications");
}

#[tokio::test]
async fn outsiders_see_nothing() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    hub.post(
        &ada,
        "/groups/geometry/threads",
        json!({"title": "Private"}),
    )
    .await;
    hub.post(
        &ada,
        "/groups",
        json!({"slug": "algebra", "name": "Algebra"}),
    )
    .await;
    let bob = hub.member(&ada, "bob").await;

    assert_eq!(
        hub.get(&bob, "/groups/algebra").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        hub.get(&bob, "/groups/algebra/threads").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        hub.get(&Client::default(), "/groups/geometry/threads/1")
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        hub.get(&bob, "/groups/geometry/threads/9").await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn api_tokens_work_until_revoked() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;

    let reply = hub
        .post(&ada, "/me/tokens", json!({"name": "laptop"}))
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let created = reply.json();
    let secret = created["secret"].as_str().unwrap().to_owned();
    assert!(secret.starts_with("stm_"));
    assert!(secret.starts_with(created["prefix"].as_str().unwrap()));

    let agent = Client {
        cookie: None,
        token: Some(secret),
    };
    assert_eq!(hub.get(&agent, "/me").await.json()["username"], "ada");
    let reply = hub
        .post(
            &agent,
            "/groups/geometry/threads",
            json!({"title": "From the CLI"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);

    let tokens = hub.get(&ada, "/me/tokens").await.json();
    assert_eq!(tokens[0]["name"], "laptop");
    assert!(tokens[0]["last_used_at"].is_string());
    assert!(tokens[0].get("secret").is_none());

    let id = created["id"].as_i64().unwrap();
    let reply = hub
        .call(&ada, Method::DELETE, &format!("/me/tokens/{id}"), None)
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let reply = hub.get(&agent, "/me").await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.json()["error"]["code"], "invalid_token");
}

#[tokio::test]
async fn changing_the_password_ends_other_sessions() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let other = signed_in(
        &hub.post(
            &Client::default(),
            "/session",
            json!({"username": "ada", "password": "analytical"}),
        )
        .await,
    );
    let reply = hub
        .post(
            &ada,
            "/me/password",
            json!({"current": "wrong", "new": "difference"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let reply = hub
        .post(
            &ada,
            "/me/password",
            json!({"current": "analytical", "new": "difference"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert_eq!(hub.get(&ada, "/me").await.status, StatusCode::OK);
    assert_eq!(
        hub.get(&other, "/me").await.status,
        StatusCode::UNAUTHORIZED
    );
}

fn multipart(name: &str, content_type: &str, data: &[u8]) -> (String, Vec<u8>) {
    let boundary = "stemma-test-boundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: {content_type}\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

async fn upload(
    hub: &TestHub,
    client: &Client,
    name: &str,
    content_type: &str,
    data: &[u8],
) -> Reply {
    let (content_type, body) = multipart(name, content_type, data);
    let request = Request::post("/api/v1/groups/geometry/attachments")
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .unwrap();
    hub.send(client, request).await
}

#[tokio::test]
async fn attachments_ride_on_messages() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let bob = hub.member(&ada, "bob").await;

    let reply = upload(&hub, &ada, "board.png", "image/png", b"\x89PNG not really").await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.json());
    let image = reply.json();
    let reply = upload(&hub, &ada, "../notes.tex", "", b"\\begin{document}").await;
    let tex = reply.json();
    assert_eq!(tex["name"], "notes.tex");
    assert_eq!(tex["content_type"], "application/x-tex");

    // Someone else's upload cannot be claimed.
    let reply = hub
        .post(
            &bob,
            "/groups/geometry/threads",
            json!({"title": "Mine", "body": "x", "attachments": [image["id"]]}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);

    let reply = hub
        .post(
            &ada,
            "/groups/geometry/threads",
            json!({"title": "Board", "body": "", "attachments": [image["id"], tex["id"]]}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.json());
    let message = &reply.json()["messages"][0];
    assert_eq!(message["attachments"].as_array().unwrap().len(), 2);
    let url = message["attachments"][0]["url"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(url.ends_with("/board.png"));

    let reply = hub.fetch(&bob, &url).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, b"\x89PNG not really");
    assert_eq!(reply.headers[header::CONTENT_TYPE], "image/png");
    assert!(
        reply.headers[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .starts_with("inline")
    );
    let tex_url = message["attachments"][1]["url"]
        .as_str()
        .unwrap()
        .to_owned();
    let reply = hub.fetch(&bob, &tex_url).await;
    assert!(
        reply.headers[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .starts_with("attachment")
    );
    assert_eq!(
        hub.fetch(&Client::default(), &url).await.status,
        StatusCode::UNAUTHORIZED
    );

    // Too large.
    let reply = upload(
        &hub,
        &ada,
        "big.bin",
        "application/octet-stream",
        &vec![0; 1024 * 1024 + 1],
    )
    .await;
    assert_eq!(reply.status, StatusCode::PAYLOAD_TOO_LARGE);

    // Deleting the message removes its files.
    hub.call(
        &ada,
        Method::DELETE,
        "/groups/geometry/threads/1/messages/1",
        None,
    )
    .await;
    assert_eq!(hub.fetch(&bob, &url).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn references_resolve() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    hub.post(
        &ada,
        "/groups/geometry/threads",
        json!({"title": "Gluing", "body": "one"}),
    )
    .await;
    hub.post(
        &ada,
        "/groups/geometry/threads/1/messages",
        json!({"body": "two"}),
    )
    .await;

    let reply = hub
        .post(&ada, "/groups/geometry/resolve", json!({"addresses": ["t1", "t1#2", "t1#1-2", "t1#3", "t9", "p/gluing", "lean:Foo", "w1", "nonsense", "t1#2-2"]}))
        .await;
    let resolved = reply.json();
    let exists: Vec<bool> = resolved
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["exists"].as_bool().unwrap())
        .collect();
    assert_eq!(
        exists,
        [
            true, true, true, false, false, false, false, false, false, true
        ]
    );
    assert_eq!(resolved[0]["title"], "Gluing");
    assert_eq!(resolved[1]["path"], "/g/geometry/t/1#2");
    // A paper the group does not have.
    assert_eq!(resolved[5]["kind"], "paper");
    assert_eq!(resolved[5]["note"], "There is no such paper");
    assert!(resolved[8]["kind"].is_null());
    assert_eq!(resolved[9]["canonical"], "t1#2");
}

#[tokio::test]
async fn events_are_live() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let bob = hub.member(&ada, "bob").await;

    let mut request = Request::get("/api/v1/groups/geometry/events")
        .body(Body::empty())
        .unwrap();
    request
        .headers_mut()
        .insert(header::COOKIE, bob.cookie.clone().unwrap().parse().unwrap());
    let response = hub.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
    let mut body = response.into_body();

    hub.post(
        &ada,
        "/groups/geometry/threads",
        json!({"title": "Live", "body": "Hello, $x$"}),
    )
    .await;

    let mut seen = String::new();
    while !seen.contains("message_created") {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), body.frame())
            .await
            .expect("an event in time")
            .unwrap()
            .unwrap();
        if let Ok(data) = frame.into_data() {
            seen.push_str(std::str::from_utf8(&data).unwrap());
        }
    }
    assert!(seen.contains("event: ready"));
    assert!(seen.contains(r#""type":"thread_created""#));
    assert!(seen.contains("Hello, $x$"));
}

#[tokio::test]
async fn the_web_app_is_served_for_any_page() {
    let hub = TestHub::new().await;
    for path in ["/", "/g/geometry/t/1", "/g/geometry/lib/Foo.bar"] {
        let request = Request::get(path).body(Body::empty()).unwrap();
        let reply = hub.send(&Client::default(), request).await;
        assert_eq!(reply.status, StatusCode::OK);
        assert!(
            reply.headers[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
    }
    let request = Request::get("/missing.js").body(Body::empty()).unwrap();
    assert_eq!(
        hub.send(&Client::default(), request).await.status,
        StatusCode::NOT_FOUND
    );
}

/// A client with a personal API token of `client`'s.
async fn with_token(hub: &TestHub, client: &Client) -> Client {
    let reply = hub
        .post(client, "/me/tokens", json!({"name": "agent"}))
        .await;
    Client {
        cookie: None,
        token: Some(reply.json()["secret"].as_str().unwrap().to_owned()),
    }
}

#[tokio::test]
async fn agents_post_for_people_with_their_tokens() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    hub.post(&ada, "/groups/geometry/threads", json!({"title": "Gluing"}))
        .await;
    let token = with_token(&hub, &ada).await;

    let reply = hub
        .post(
            &token,
            "/groups/geometry/threads/1/messages",
            json!({"body": "A summary of #1", "agent": "Claude Code"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.json());
    assert_eq!(reply.json()["agent"], "Claude Code");
    assert_eq!(reply.json()["agent_runner"], "claude-code");
    assert_eq!(reply.json()["author"]["username"], "ada");

    // A session is a person at the web app, not an agent.
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/threads/1/messages",
            json!({"body": "Me", "agent": "Claude"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn job_tokens_read_one_group_and_post_only_where_allowed() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    hub.post(
        &ada,
        "/groups",
        json!({"slug": "algebra", "name": "Algebra"}),
    )
    .await;
    hub.post(
        &ada,
        "/groups/geometry/threads",
        json!({"title": "Gluing", "body": "First"}),
    )
    .await;
    hub.post(&ada, "/groups/geometry/threads", json!({"title": "Other"}))
        .await;
    let (group_id, thread_id): (i64, i64) = sqlx::query_as(
        "SELECT g.id, t.id FROM groups g JOIN threads t ON t.group_id = g.id
         WHERE g.slug = 'geometry' AND t.number = 1",
    )
    .fetch_one(&hub.state.db)
    .await
    .unwrap();
    let job = |post_in| {
        let db = hub.state.db.clone();
        async move {
            let (_, secret) = stemma_hub::auth::create_job_token(
                &db,
                1,
                group_id,
                post_in,
                "Claude",
                stemma_hub::db::now() + 60_000,
            )
            .await
            .unwrap();
            Client {
                cookie: None,
                token: Some(secret),
            }
        }
    };

    let reader = job(None).await;
    let me = hub.get(&reader, "/me").await.json();
    assert_eq!(me["groups"].as_array().unwrap().len(), 1);
    assert_eq!(me["groups"][0]["slug"], "geometry");
    let groups = hub.get(&reader, "/groups").await.json();
    assert_eq!(groups.as_array().unwrap().len(), 1, "{groups}");
    assert_eq!(groups[0]["slug"], "geometry");
    assert_eq!(
        hub.get(&reader, "/groups/geometry/threads/1").await.status,
        StatusCode::OK
    );
    assert_eq!(
        hub.get(&reader, "/groups/algebra/threads").await.status,
        StatusCode::NOT_FOUND
    );
    for (path, body) in [
        ("/me/tokens", json!({"name": "more"})),
        (
            "/groups/geometry/threads",
            json!({"title": "New", "body": "Hi"}),
        ),
        ("/groups/geometry/invitations", json!({})),
    ] {
        let reply = hub.post(&reader, path, body).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{path}");
    }
    let reply = hub
        .post(
            &reader,
            "/groups/geometry/threads/1/messages",
            json!({"body": "Hello"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);

    let poster = job(Some(thread_id)).await;
    let reply = hub
        .post(
            &poster,
            "/groups/geometry/threads/1/messages",
            json!({"body": "Hello", "agent": "Someone else"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    assert_eq!(reply.json()["agent"], "Claude");
    let reply = hub
        .post(
            &poster,
            "/groups/geometry/threads/1/messages",
            json!({"body": "Look", "attachments": [1]}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let reply = hub
        .post(
            &poster,
            "/groups/geometry/threads/2/messages",
            json!({"body": "Hello"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn search_finds_titles_and_messages() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    hub.post(
        &ada,
        "/groups/geometry/threads",
        json!({"title": "The gluing lemma", "body": "Start"}),
    )
    .await;
    hub.post(
        &ada,
        "/groups/geometry/threads/1/messages",
        json!({"body": "Is the Gluing map proper? Compactness says so."}),
    )
    .await;
    let hits = hub
        .get(&ada, "/groups/geometry/search?q=gluing%20PROPER")
        .await
        .json();
    assert_eq!(hits.as_array().unwrap().len(), 1, "{hits}");
    assert_eq!(hits[0]["address"], "t1#2");
    assert!(hits[0]["snippet"].as_str().unwrap().contains("Gluing map"));

    let hits = hub
        .get(&ada, "/groups/geometry/search?q=gluing")
        .await
        .json();
    assert_eq!(hits[0]["address"], "t1");
    assert_eq!(hits[1]["address"], "t1#2");

    let reply = hub.get(&ada, "/groups/geometry/search?q=%20").await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
}

/// The hits of a search in `geometry`, by address.
async fn search(hub: &TestHub, client: &Client, query: &str) -> Vec<String> {
    let path = format!(
        "/groups/geometry/search?q={}",
        query
            .replace(' ', "%20")
            .replace(':', "%3A")
            .replace('"', "%22")
    );
    let reply = hub.get(client, &path).await;
    assert_eq!(reply.status, StatusCode::OK, "{query}: {:?}", reply.json());
    reply
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["address"].as_str().unwrap().to_owned())
        .collect()
}

fn sorted(hits: &[String]) -> Vec<String> {
    let mut hits = hits.to_vec();
    hits.sort();
    hits
}

#[tokio::test]
async fn search_takes_operators() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let bob = hub.member(&ada, "bob").await;
    hub.post(
        &ada,
        "/groups/geometry/threads",
        json!({"title": "Compact fibres", "body": "Are the fibres compact?"}),
    )
    .await;
    hub.post(
        &bob,
        "/groups/geometry/threads/1/messages",
        json!({"body": "The fibres are compact, by \"Tychonoff's theorem\"."}),
    )
    .await;
    hub.post(
        &bob,
        "/groups/geometry/threads",
        json!({"title": "Notation", "kind": "chat", "resident_agent": "claude",
               "body": "Which letter for the fibres?"}),
    )
    .await;

    // Words are prefixes, regardless of case and accents.
    let hits = search(&hub, &ada, "FIBR").await;
    assert_eq!(hits[0], "t1", "titles first");
    assert_eq!(sorted(&hits[1..]), ["t1#1", "t1#2", "t2#1"]);
    assert_eq!(search(&hub, &ada, "tychonoff").await, ["t1#2"]);
    assert_eq!(
        search(&hub, &ada, "\"theorem tychonoff\"").await,
        Vec::<String>::new()
    );
    assert_eq!(
        sorted(&search(&hub, &ada, "fibres author:bob").await),
        ["t1#2", "t2#1"]
    );
    assert_eq!(search(&hub, &ada, "fibres in:chats").await, ["t2#1"]);
    assert_eq!(
        sorted(&search(&hub, &ada, "fibres in:forum kind:message").await),
        ["t1#1", "t1#2"]
    );
    assert_eq!(search(&hub, &ada, "fibres kind:thread").await, ["t1"]);
    assert_eq!(search(&hub, &ada, "fibres in:t2").await, ["t2#1"]);
    assert_eq!(
        search(&hub, &ada, "fibres kind:agent").await,
        Vec::<String>::new()
    );
    assert_eq!(search(&hub, &ada, "fibres since:1d").await.len(), 4);
    assert!(
        search(&hub, &ada, "fibres until:2000-01-01")
            .await
            .is_empty()
    );

    // Without words, what the operators allow, newest first.
    assert_eq!(search(&hub, &ada, "in:chats").await, ["t2"]);
    assert_eq!(search(&hub, &ada, "author:ada").await, ["t1#1"]);

    // The index follows what changes: a new title, an edit, a deletion.
    hub.call(
        &ada,
        Method::PATCH,
        "/groups/geometry/threads/1",
        Some(json!({"title": "Proper maps"})),
    )
    .await;
    assert_eq!(search(&hub, &ada, "proper").await, ["t1"]);
    assert_eq!(
        search(&hub, &ada, "fibres kind:thread").await,
        Vec::<String>::new()
    );
    hub.call(
        &bob,
        Method::PATCH,
        "/groups/geometry/threads/1/messages/2",
        Some(json!({"body": "By Tikhonov."})),
    )
    .await;
    assert_eq!(search(&hub, &ada, "tychonoff").await, Vec::<String>::new());
    assert_eq!(search(&hub, &ada, "tikhonov").await, ["t1#2"]);
    hub.call(
        &bob,
        Method::DELETE,
        "/groups/geometry/threads/1/messages/2",
        None,
    )
    .await;
    assert_eq!(search(&hub, &ada, "tikhonov").await, Vec::<String>::new());

    // A wrong operator is said so.
    for wrong in ["in:nowhere", "since:soon", "kind:poem", "%20"] {
        let reply = hub
            .get(&ada, &format!("/groups/geometry/search?q={wrong}"))
            .await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{wrong}");
    }
}

#[tokio::test]
async fn search_stays_in_the_viewers_groups() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    hub.post(
        &ada,
        "/groups/geometry/threads",
        json!({"title": "A secret", "body": "Nobody else reads this"}),
    )
    .await;
    // Someone in another group only.
    hub.post(
        &ada,
        "/groups",
        json!({"slug": "algebra", "name": "Algebra"}),
    )
    .await;
    let reply = hub
        .post(&ada, "/groups/algebra/invitations", json!({"max_uses": 1}))
        .await;
    let token = reply.json()["token"].as_str().unwrap().to_owned();
    let reply = hub
        .post(
            &Client::default(),
            &format!("/invitations/{token}/accept"),
            json!({"account": {"username": "eve", "display_name": "Eve", "password": "password1"}}),
        )
        .await;
    let eve = signed_in(&reply);

    let reply = hub.get(&eve, "/groups/geometry/search?q=secret").await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let reply = hub.get(&eve, "/groups/algebra/search?q=secret").await;
    assert_eq!(reply.json(), json!([]));
    let reply = hub
        .get(&Client::default(), "/groups/geometry/search?q=secret")
        .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mentions_and_chats_queue_jobs() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    hub.post(
        &ada,
        "/groups/geometry/threads",
        json!({"title": "Gluing", "body": "@claude @codex @nobody what do you think?"}),
    )
    .await;
    let jobs = hub
        .get(&ada, "/groups/geometry/threads/1/jobs")
        .await
        .json();
    assert_eq!(jobs.as_array().unwrap().len(), 2, "{jobs}");
    assert_eq!(jobs[0]["agent"], "claude");
    assert_eq!(jobs[0]["state"], "queued");
    assert_eq!(jobs[0]["trigger"], 1);
    assert_eq!(
        jobs[0]["note"],
        "Waiting for a machine of Ada's with Claude Code"
    );
    assert_eq!(jobs[1]["note"], "Waiting for a machine of Ada's with Codex");

    // An agent's message summons no one.
    let token = with_token(&hub, &ada).await;
    hub.post(
        &token,
        "/groups/geometry/threads/1/messages",
        json!({"body": "@codex, over to you", "agent": "Claude Code"}),
    )
    .await;
    let jobs = hub
        .get(&ada, "/groups/geometry/threads/1/jobs")
        .await
        .json();
    assert_eq!(jobs.as_array().unwrap().len(), 2);

    // Cancelling one.
    let id = jobs[1]["id"].as_i64().unwrap();
    let reply = hub
        .call(
            &ada,
            Method::DELETE,
            &format!("/groups/geometry/jobs/{id}"),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let jobs = hub
        .get(&ada, "/groups/geometry/threads/1/jobs")
        .await
        .json();
    assert_eq!(jobs.as_array().unwrap().len(), 1);

    // A chat's resident agent answers every message.
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/threads",
            json!({"title": "Chat", "kind": "chat", "resident_agent": "codex", "body": "Hi"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    hub.post(
        &ada,
        "/groups/geometry/threads/2/messages",
        json!({"body": "And @codex, once more"}),
    )
    .await;
    let jobs = hub
        .get(&ada, "/groups/geometry/threads/2/jobs")
        .await
        .json();
    assert_eq!(jobs.as_array().unwrap().len(), 2);
    assert_eq!(jobs[1]["trigger"], 2);

    let reply = hub
        .post(
            &ada,
            "/groups/geometry/threads",
            json!({"title": "Chat", "kind": "chat", "resident_agent": "nobody"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
}

/// The agent a job runs, as the hub stored it when it queued the job.
async fn job_agent(hub: &TestHub, job: &Value) -> Value {
    let agent: String = sqlx::query_scalar("SELECT agent FROM jobs WHERE id = ?")
        .bind(job["id"].as_i64().unwrap())
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    serde_json::from_str(&agent).unwrap()
}

#[tokio::test]
async fn a_chat_chooses_its_model_and_effort() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let bob = hub.member(&ada, "bob").await;
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/threads",
            json!({"title": "Is every compact group profinite?", "kind": "chat",
                   "resident_agent": "claude", "model": "opus", "effort": "high",
                   "body": "Is every compact group profinite?"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.json());
    let thread = &reply.json()["thread"];
    assert_eq!(thread["resident_model"], "opus");
    assert_eq!(thread["resident_effort"], "high");

    // Everyone who writes is answered by their own @claude, as the chat chose.
    hub.post(
        &bob,
        "/groups/geometry/threads/1/messages",
        json!({"body": "Not the circle, surely"}),
    )
    .await;
    let jobs = hub
        .get(&ada, "/groups/geometry/threads/1/jobs")
        .await
        .json();
    assert_eq!(jobs.as_array().unwrap().len(), 2);
    for (job, who) in jobs.as_array().unwrap().iter().zip(["ada", "bob"]) {
        assert_eq!(job["requester"]["username"], who);
        let agent = job_agent(&hub, job).await;
        assert_eq!(agent["model"], "opus");
        assert_eq!(agent["args"], json!(["--effort", "high"]));
    }

    // Codex takes it as its reasoning effort.
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/threads",
            json!({"title": "Quick", "kind": "chat", "resident_agent": "codex",
                   "effort": "low", "body": "Hi"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let jobs = hub
        .get(&ada, "/groups/geometry/threads/2/jobs")
        .await
        .json();
    let agent = job_agent(&hub, &jobs[0]).await;
    assert_eq!(agent["model"], Value::Null);
    assert_eq!(agent["args"], json!(["-c", "model_reasoning_effort=low"]));

    // Only chats choose, and OpenCode takes no effort from them.
    for body in [
        json!({"title": "x", "model": "opus"}),
        json!({"title": "x", "kind": "chat", "effort": "high"}),
        json!({"title": "x", "kind": "chat", "resident_agent": "opencode", "effort": "high"}),
        json!({"title": "x", "kind": "chat", "resident_agent": "claude", "effort": "extreme"}),
    ] {
        let reply = hub
            .post(&ada, "/groups/geometry/threads", body.clone())
            .await;
        assert!(reply.status.is_client_error(), "{body}: {:?}", reply.json());
    }
}

#[tokio::test]
async fn machines_are_their_owners() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let bob = hub.member(&ada, "bob").await;

    let reply = hub
        .post(&bob, "/me/machines", json!({"name": "laptop"}))
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    assert!(reply.json()["secret"].as_str().unwrap().starts_with("stw_"));
    assert_eq!(reply.json()["preferred"], false);
    let id = reply.json()["id"].as_i64().unwrap();
    // Names tell one's machines apart.
    let reply = hub
        .post(&bob, "/me/machines", json!({"name": "laptop"}))
        .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    let reply = hub
        .post(
            &bob,
            "/me/machines",
            json!({"name": "desktop", "preferred": true}),
        )
        .await;
    assert_eq!(reply.json()["preferred"], true);
    let desktop = reply.json()["id"].as_i64().unwrap();

    let mine = hub.get(&bob, "/me/machines").await.json();
    assert_eq!(mine.as_array().unwrap().len(), 2);

    // One machine at most is preferred.
    let reply = hub
        .call(
            &bob,
            Method::PATCH,
            &format!("/me/machines/{id}"),
            Some(json!({"preferred": true})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["preferred"], true);
    let mine = hub.get(&bob, "/me/machines").await.json();
    assert_eq!(mine[1]["id"], desktop);
    assert_eq!(mine[1]["preferred"], false);

    let reply = hub
        .call(&ada, Method::DELETE, &format!("/me/machines/{id}"), None)
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    for id in [id, desktop] {
        let reply = hub
            .call(&bob, Method::DELETE, &format!("/me/machines/{id}"), None)
            .await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT);
    }
    assert!(
        hub.get(&bob, "/me/machines")
            .await
            .json()
            .as_array()
            .unwrap()
            .is_empty()
    );
}

async fn job_token(hub: &TestHub, user_id: i64) -> Client {
    let group_id: i64 = sqlx::query_scalar("SELECT id FROM groups WHERE slug = 'geometry'")
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    let (_, secret) = stemma_hub::auth::create_job_token(
        &hub.state.db,
        user_id,
        group_id,
        None,
        "Claude",
        stemma_hub::db::now() + 60_000,
    )
    .await
    .unwrap();
    Client {
        cookie: None,
        token: Some(secret),
    }
}

#[tokio::test]
async fn threads_anchor_to_declarations() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/threads",
            json!({"title": "About gluing", "anchor": "lean:Foo.glue", "body": "Is it right?"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.json());
    assert_eq!(reply.json()["thread"]["anchor"], "lean:Foo.glue");
    for anchor in ["t1", "p/notes"] {
        let reply = hub
            .post(
                &ada,
                "/groups/geometry/threads",
                json!({"title": "Nowhere", "anchor": anchor}),
            )
            .await;
        assert!(reply.status.is_client_error(), "{anchor}: {}", reply.status);
    }
}

#[tokio::test]
async fn the_workshop_takes_tasks_from_conversation() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let bob = hub.member(&ada, "bob").await;

    // A message to the workshop hands it a task.
    hub.post(
        &ada,
        "/groups/geometry/threads",
        json!({"title": "Gluing", "body": "The charts glue."}),
    )
    .await;
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/threads/1/messages",
            json!({"body": "@workshop formalize the gluing of t1#1, with a docstring."}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let tasks = hub.get(&bob, "/groups/geometry/tasks").await.json();
    assert_eq!(tasks.as_array().unwrap().len(), 1, "{tasks}");
    let task = &tasks[0];
    assert_eq!(task["number"], 1);
    assert_eq!(
        task["title"],
        "Formalize the gluing of t1#1, with a docstring"
    );
    assert_eq!(task["state"], "open");
    assert_eq!(task["origin"], "t1#2");
    assert_eq!(task["requester"]["username"], "ada");
    assert_eq!(
        task["progress"],
        "Waiting for a workshop machine with Lean to come online"
    );
    // The thread shows it on its way.
    let jobs = hub
        .get(&ada, "/groups/geometry/threads/1/jobs")
        .await
        .json();
    assert_eq!(jobs[0]["kind"], "task_run");
    assert_eq!(jobs[0]["task"], 1);
    assert_eq!(jobs[0]["agent_name"], "Workshop");

    // Handed over by a person, naming the message that asks.
    let reply = hub
        .post(
            &bob,
            "/groups/geometry/tasks",
            json!({"title": "State the lemma", "instructions": "As in t1#1.", "origin": "t1#1"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.json());
    let task = reply.json();
    assert_eq!(task["number"], 2);
    let page = hub.get(&bob, "/groups/geometry/tasks/2").await.json();
    assert_eq!(page["instructions"], "As in t1#1.");
    assert_eq!(
        page["progress"],
        "Waiting for a workshop machine with Lean to come online"
    );
    assert_eq!(page["log"], "");
    let reply = hub
        .post(
            &bob,
            "/groups/geometry/tasks",
            json!({"title": "No origin", "instructions": "Something."}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let reply = hub
        .post(
            &bob,
            "/groups/geometry/tasks",
            json!({"title": "Far", "instructions": "Something.", "origin": "t1#9"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);

    // A job's token hands tasks over only for an agent answering a thread.
    let bob_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username = 'bob'")
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    let job = job_token(&hub, bob_id).await;
    let reply = hub
        .post(
            &job,
            "/groups/geometry/tasks",
            json!({"title": "Sneaky", "instructions": "Something.", "origin": "t1#1"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);

    // References to tasks resolve.
    let resolved = hub
        .post(
            &ada,
            "/groups/geometry/resolve",
            json!({"addresses": ["w2", "w9"]}),
        )
        .await
        .json();
    assert_eq!(resolved[0]["exists"], true);
    assert_eq!(resolved[0]["title"], "w2: State the lemma");
    assert_eq!(resolved[0]["path"], "/g/geometry/w/2");
    assert_eq!(resolved[1]["exists"], false);

    // Only its requester or an admin cancels a task.
    let reply = hub
        .call(&bob, Method::DELETE, "/groups/geometry/tasks/1", None)
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let reply = hub
        .call(&ada, Method::DELETE, "/groups/geometry/tasks/1", None)
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let page = hub.get(&ada, "/groups/geometry/tasks/1").await.json();
    assert_eq!(page["state"], "cancelled");
    let reply = hub
        .call(&ada, Method::DELETE, "/groups/geometry/tasks/1", None)
        .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    let open = hub
        .get(&ada, "/groups/geometry/tasks?state=open")
        .await
        .json();
    assert_eq!(open.as_array().unwrap().len(), 1);
    assert_eq!(open[0]["number"], 2);
}

#[tokio::test]
async fn invitations_grant_their_role() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;

    // An invitation says the role it grants, and gives it.
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/invitations",
            json!({"role": "guest"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    assert_eq!(reply.json()["role"], "guest");
    let token = reply.json()["token"].as_str().unwrap().to_owned();
    let info = hub
        .get(&Client::default(), &format!("/invitations/{token}"))
        .await;
    assert_eq!(info.json()["role"], "guest");
    let account =
        json!({"account": {"username": "gus", "display_name": "Gus", "password": "password1"}});
    let reply = hub
        .post(
            &Client::default(),
            &format!("/invitations/{token}/accept"),
            account,
        )
        .await;
    assert_eq!(reply.json()["groups"][0]["role"], "guest");
    let gus = signed_in(&reply);
    assert_eq!(
        hub.get(&gus, "/groups/geometry").await.json()["role"],
        "guest"
    );

    // An admin invites with any role; a member or a guest, with none.
    let carol = hub.join(&ada, "carol", "admin").await;
    assert_eq!(
        hub.get(&carol, "/groups/geometry").await.json()["role"],
        "admin"
    );
    let bob = hub.member(&ada, "bob").await;
    assert_eq!(
        hub.get(&bob, "/groups/geometry").await.json()["role"],
        "member"
    );
    for (who, role) in [(&bob, "guest"), (&gus, "guest"), (&gus, "member")] {
        let reply = hub
            .post(who, "/groups/geometry/invitations", json!({"role": role}))
            .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
    }
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/invitations",
            json!({"role": "owner"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn admins_change_roles() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let bob = hub.member(&ada, "bob").await;
    let gus = hub.join(&ada, "gus", "guest").await;
    let mut ids = Vec::new();
    for client in [&ada, &bob, &gus] {
        ids.push(hub.get(client, "/me").await.json()["id"].as_i64().unwrap());
    }
    let (ada_id, bob_id, gus_id) = (ids[0], ids[1], ids[2]);
    let set = async |client: &Client, user: i64, role: &str| {
        hub.call(
            client,
            Method::PATCH,
            &format!("/groups/geometry/members/{user}"),
            Some(json!({"role": role})),
        )
        .await
        .status
    };

    // Members and guests change nobody's role, not even their own.
    assert_eq!(set(&bob, gus_id, "member").await, StatusCode::FORBIDDEN);
    assert_eq!(set(&bob, bob_id, "admin").await, StatusCode::FORBIDDEN);
    assert_eq!(set(&gus, gus_id, "member").await, StatusCode::FORBIDDEN);

    // An admin does.
    assert_eq!(set(&ada, gus_id, "member").await, StatusCode::NO_CONTENT);
    assert_eq!(set(&ada, bob_id, "guest").await, StatusCode::NO_CONTENT);
    let members = hub.get(&ada, "/groups/geometry/members").await.json();
    let role_of = |username: &str| {
        members
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["username"] == username)
            .unwrap()["role"]
            .clone()
    };
    assert_eq!(role_of("gus"), "member");
    assert_eq!(role_of("bob"), "guest");

    // But never leaves the group without an admin.
    assert_eq!(set(&ada, ada_id, "guest").await, StatusCode::CONFLICT);
}

#[tokio::test]
async fn guests_converse_but_ask_for_no_work() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let gus = hub.join(&ada, "gus", "guest").await;

    // A guest reads, posts and summons their own agents.
    let reply = hub
        .post(
            &gus,
            "/groups/geometry/threads",
            json!({"title": "A question", "body": "@claude is it proper?"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.json());
    assert_eq!(
        hub.get(&gus, "/groups/geometry/threads").await.status,
        StatusCode::OK
    );
    let jobs = hub
        .get(&gus, "/groups/geometry/threads/1/jobs")
        .await
        .json();
    assert_eq!(jobs[0]["agent"], "claude");
    assert_eq!(jobs[0]["state"], "queued");

    // But asks nothing of the workshop: not in a message…
    hub.post(
        &gus,
        "/groups/geometry/threads/1/messages",
        json!({"body": "@workshop formalize it."}),
    )
    .await;
    let tasks = hub.get(&gus, "/groups/geometry/tasks").await.json();
    assert!(tasks.as_array().unwrap().is_empty(), "{tasks}");
    let jobs = hub
        .get(&gus, "/groups/geometry/threads/1/jobs")
        .await
        .json();
    assert_eq!(jobs[1]["state"], "failed", "{jobs}");
    assert_eq!(jobs[1]["error"], "Guests cannot ask @workshop for work");

    // …nor directly…
    let task = json!({"title": "Formalize it", "instructions": "As in t1#1.", "origin": "t1#1"});
    let reply = hub.post(&gus, "/groups/geometry/tasks", task.clone()).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);

    // …nor through an agent, with its job's token, which has the guest's
    // role.
    let gus_id = hub.get(&gus, "/me").await.json()["id"].as_i64().unwrap();
    let job = job_token(&hub, gus_id).await;
    assert_eq!(
        hub.get(&job, "/groups/geometry").await.json()["role"],
        "guest"
    );
    let reply = hub.post(&job, "/groups/geometry/tasks", task.clone()).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);

    // Nor asks for a build of the library.
    let reply = hub
        .post(&gus, "/groups/geometry/library/builds", json!({}))
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);

    // Nor changes the group's settings.
    for (method, path, body) in [
        (
            Method::POST,
            "/groups/geometry/workshop/machines",
            Some(json!({"machine": 1})),
        ),
        (
            Method::POST,
            "/groups/geometry/invitations",
            Some(json!({})),
        ),
        (Method::GET, "/groups/geometry/invitations", None),
    ] {
        let reply = hub.call(&gus, method.clone(), path, body).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{method} {path}");
    }

    // A member asks the workshop for work.
    let bob = hub.member(&ada, "bob").await;
    let reply = hub.post(&bob, "/groups/geometry/tasks", task).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.json());
}

// ── People's agents and machines ──────────────────────────────────────

fn handles(agents: &Value) -> Vec<String> {
    agents
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["handle"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn people_keep_their_own_agents() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let bob = hub.member(&ada, "bob").await;

    // Everyone has one agent per runner.
    let mine = hub.get(&bob, "/me/agents").await.json();
    assert_eq!(handles(&mine), ["claude", "codex", "opencode"]);
    assert_eq!(mine[0]["runner"], "claude-code");
    assert_eq!(mine[0]["display_name"], "Claude");
    assert_eq!(mine[0]["default"], true);

    // And adds more, with handles of their own.
    let sonnet = json!({"handle": "@Claude-Sonnet", "display_name": "Claude Sonnet",
                        "runner": "claude-code", "model": "sonnet",
                        "args": ["--effort", "high"]});
    let reply = hub.post(&bob, "/me/agents", sonnet.clone()).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.json());
    assert_eq!(reply.json()["handle"], "claude-sonnet");
    assert_eq!(reply.json()["default"], false);
    let reply = hub.post(&bob, "/me/agents", sonnet).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    let mine = hub.get(&bob, "/me/agents").await.json();
    assert_eq!(
        handles(&mine),
        ["claude", "codex", "opencode", "claude-sonnet"]
    );
    assert_eq!(mine[3]["args"], json!(["--effort", "high"]));
    // Ada has none of Bob's.
    assert_eq!(handles(&hub.get(&ada, "/me/agents").await.json()).len(), 3);

    for (agent, status) in [
        // A default is changed, not added again.
        (
            json!({"handle": "claude", "display_name": "C", "runner": "claude-code"}),
            StatusCode::CONFLICT,
        ),
        // The group's agents keep their handles.
        (
            json!({"handle": "workshop", "display_name": "W", "runner": "opencode"}),
            StatusCode::CONFLICT,
        ),
        (
            json!({"handle": "1x", "display_name": "X", "runner": "codex"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({"handle": "x", "display_name": "X", "runner": "api"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({"handle": "x", "display_name": " ", "runner": "codex"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({"handle": "x", "display_name": "X", "runner": "codex",
                   "args": vec!["-v"; 40]}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        let reply = hub.post(&bob, "/me/agents", agent.clone()).await;
        assert_eq!(reply.status, status, "{agent}");
    }

    // A default is changed, but keeps its runner.
    let reply = hub
        .call(
            &bob,
            Method::PUT,
            "/me/agents/claude",
            Some(
                json!({"handle": "x", "display_name": "Claude Opus", "runner": "claude-code",
                        "model": "opus"}),
            ),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());
    assert_eq!(reply.json()["handle"], "claude");
    assert_eq!(reply.json()["default"], true);
    let mine = hub.get(&bob, "/me/agents").await.json();
    assert_eq!(mine[0]["model"], "opus");
    let reply = hub
        .call(
            &bob,
            Method::PUT,
            "/me/agents/claude",
            Some(json!({"handle": "claude", "display_name": "Claude", "runner": "codex"})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let reply = hub
        .call(
            &bob,
            Method::PUT,
            "/me/agents/nobody",
            Some(json!({"handle": "nobody", "display_name": "N", "runner": "codex"})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);

    // Removing a default puts it back as it was.
    let reply = hub
        .call(&bob, Method::DELETE, "/me/agents/claude", None)
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let mine = hub.get(&bob, "/me/agents").await.json();
    assert_eq!(mine[0]["display_name"], "Claude");
    assert!(mine[0]["model"].is_null());
    for status in [StatusCode::NO_CONTENT, StatusCode::NOT_FOUND] {
        let reply = hub
            .call(&bob, Method::DELETE, "/me/agents/claude-sonnet", None)
            .await;
        assert_eq!(reply.status, status);
    }

    // Nobody's own agent takes the workshop's handle.
    let reply = hub
        .post(
            &bob,
            "/me/agents",
            json!({"handle": "workshop", "display_name": "Mine", "runner": "opencode"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn mentions_are_the_writers_own_agents() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let bob = hub.member(&ada, "bob").await;
    let gus = hub.join(&ada, "gus", "guest").await;
    hub.post(
        &bob,
        "/me/agents",
        json!({"handle": "claude-sonnet", "display_name": "Claude Sonnet",
               "runner": "claude-code", "model": "sonnet"}),
    )
    .await;

    // Ada has no @claude-sonnet: her job fails at once, and says so.
    hub.post(
        &ada,
        "/groups/geometry/threads",
        json!({"title": "Gluing", "body": "@claude-sonnet @claude @nobody is it proper?"}),
    )
    .await;
    let jobs = hub
        .get(&ada, "/groups/geometry/threads/1/jobs")
        .await
        .json();
    assert_eq!(jobs.as_array().unwrap().len(), 2, "{jobs}");
    assert_eq!(jobs[0]["agent"], "claude-sonnet");
    assert_eq!(jobs[0]["state"], "failed");
    assert_eq!(
        jobs[0]["error"],
        "Ada has no agent @claude-sonnet: add one in the settings"
    );
    assert_eq!(jobs[1]["agent"], "claude");
    assert_eq!(jobs[1]["requester"]["username"], "ada");
    assert_eq!(jobs[1]["state"], "queued");

    // Bob's is his, as he defined it.
    hub.post(
        &bob,
        "/groups/geometry/threads/1/messages",
        json!({"body": "@claude-sonnet, you?"}),
    )
    .await;
    let jobs = hub
        .get(&bob, "/groups/geometry/threads/1/jobs")
        .await
        .json();
    assert_eq!(jobs[2]["agent"], "claude-sonnet");
    assert_eq!(jobs[2]["agent_name"], "Claude Sonnet");
    assert_eq!(jobs[2]["requester"]["username"], "bob");
    assert_eq!(jobs[2]["state"], "queued");
    assert_eq!(
        jobs[2]["note"],
        "Waiting for a machine of bob's with Claude Code"
    );
    let agent: String = sqlx::query_scalar("SELECT agent FROM jobs WHERE id = ?")
        .bind(jobs[2]["id"].as_i64().unwrap())
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    let agent: Value = serde_json::from_str(&agent).unwrap();
    assert_eq!(agent["model"], "sonnet");

    // A guest summons their own too.
    hub.post(
        &gus,
        "/groups/geometry/threads/1/messages",
        json!({"body": "@codex and you?"}),
    )
    .await;
    let jobs = hub
        .get(&gus, "/groups/geometry/threads/1/jobs")
        .await
        .json();
    assert_eq!(jobs[3]["agent"], "codex");
    assert_eq!(jobs[3]["requester"]["username"], "gus");
    assert_eq!(jobs[3]["state"], "queued");

    // A chat's resident agent is the writer's own, for each who writes.
    let reply = hub
        .post(
            &bob,
            "/groups/geometry/threads",
            json!({"title": "Chat", "kind": "chat", "resident_agent": "claude-sonnet",
                   "body": "Hi"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.json());
    hub.post(
        &ada,
        "/groups/geometry/threads/2/messages",
        json!({"body": "Me too"}),
    )
    .await;
    let jobs = hub
        .get(&ada, "/groups/geometry/threads/2/jobs")
        .await
        .json();
    assert_eq!(jobs[0]["state"], "queued");
    assert_eq!(jobs[1]["state"], "failed");
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/threads",
            json!({"title": "Chat", "kind": "chat", "resident_agent": "claude-sonnet"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
}

/// A machine of `user_id`'s, connected as if over its WebSocket, with
/// `runners`: what the hub sends it arrives on the receiver.
async fn connect_machine(
    hub: &TestHub,
    id: i64,
    owner_id: i64,
    runners: &[&str],
    preferred: bool,
) -> tokio::sync::mpsc::UnboundedReceiver<stemma_core::protocol::HubMessage> {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    stemma_hub::jobs::connected(
        &hub.state,
        id,
        stemma_hub::jobs::Connected {
            personal: true,
            connection: id as u64,
            owner_id,
            runners: runners.iter().map(|r| (*r).to_owned()).collect(),
            lean: false,
            latex: false,
            isolation: None,
            workshop: Default::default(),
            concurrency: 4,
            preferred,
            running: Default::default(),
            sender,
        },
    )
    .await
    .unwrap();
    receiver
}

/// The handle of the agent of the job a machine was sent, if it was sent
/// one.
fn sent(
    receiver: &mut tokio::sync::mpsc::UnboundedReceiver<stemma_core::protocol::HubMessage>,
) -> Option<String> {
    match receiver.try_recv().ok()? {
        stemma_core::protocol::HubMessage::Job(spec) => {
            Some(spec.conversation()?.agent.handle.clone())
        }
        _ => None,
    }
}

#[tokio::test]
async fn jobs_go_to_the_preferred_machine_with_the_runner() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let bob = hub.member(&ada, "bob").await;
    let bob_id = hub.get(&bob, "/me").await.json()["id"].as_i64().unwrap();
    let mut ids = Vec::new();
    for (name, preferred) in [("laptop", false), ("desktop", true)] {
        let reply = hub
            .post(
                &bob,
                "/me/machines",
                json!({"name": name, "preferred": preferred}),
            )
            .await;
        ids.push(reply.json()["id"].as_i64().unwrap());
    }
    let (laptop, desktop) = (ids[0], ids[1]);
    let mut on_laptop =
        connect_machine(&hub, laptop, bob_id, &["claude-code", "codex"], false).await;
    let mut on_desktop = connect_machine(&hub, desktop, bob_id, &["claude-code"], true).await;
    let machines = hub.get(&bob, "/me/machines").await.json();
    assert_eq!(machines[0]["online"], true);

    // Both have Claude Code: the preferred one runs it.
    hub.post(
        &bob,
        "/groups/geometry/threads",
        json!({"title": "Gluing", "body": "@claude is it proper?"}),
    )
    .await;
    assert_eq!(sent(&mut on_desktop).as_deref(), Some("claude"));
    assert_eq!(sent(&mut on_laptop), None);

    // Only one has Codex.
    hub.post(
        &bob,
        "/groups/geometry/threads/1/messages",
        json!({"body": "@codex?"}),
    )
    .await;
    assert_eq!(sent(&mut on_laptop).as_deref(), Some("codex"));
    assert_eq!(sent(&mut on_desktop), None);

    // None has OpenCode: the job waits, and says for what.
    hub.post(
        &bob,
        "/groups/geometry/threads/1/messages",
        json!({"body": "@opencode?"}),
    )
    .await;
    let jobs = hub
        .get(&bob, "/groups/geometry/threads/1/jobs")
        .await
        .json();
    let waiting = jobs
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["agent"] == "opencode")
        .unwrap();
    assert_eq!(waiting["state"], "queued");
    assert_eq!(
        waiting["note"],
        "Waiting for a machine of bob's with OpenCode"
    );

    // Ada's agents never run on Bob's machines.
    hub.post(
        &ada,
        "/groups/geometry/threads/1/messages",
        json!({"body": "@claude, and for me?"}),
    )
    .await;
    assert_eq!(sent(&mut on_desktop), None);
    assert_eq!(sent(&mut on_laptop), None);
    let jobs = hub
        .get(&ada, "/groups/geometry/threads/1/jobs")
        .await
        .json();
    let adas = jobs
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["requester"]["username"] == "ada")
        .unwrap();
    assert_eq!(
        adas["note"],
        "Waiting for a machine of Ada's with Claude Code"
    );

    // Preferring the other machine sends the next job there.
    hub.call(
        &bob,
        Method::PATCH,
        &format!("/me/machines/{laptop}"),
        Some(json!({"preferred": true})),
    )
    .await;
    hub.post(
        &bob,
        "/groups/geometry/threads/1/messages",
        json!({"body": "@claude once more"}),
    )
    .await;
    assert_eq!(sent(&mut on_laptop).as_deref(), Some("claude"));
    assert_eq!(sent(&mut on_desktop), None);
}

#[tokio::test]
async fn tokens_may_expire() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let reply = hub
        .post(
            &ada,
            "/me/tokens",
            json!({"name": "a session", "expires_in_minutes": 60}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    assert!(reply.json()["expires_at"].is_string());
    let session = Client {
        cookie: None,
        token: Some(reply.json()["secret"].as_str().unwrap().to_owned()),
    };
    assert_eq!(hub.get(&session, "/me").await.status, StatusCode::OK);
    // Once it expires, it no longer works, nor is it listed.
    sqlx::query("UPDATE api_tokens SET expires_at = 1 WHERE name = 'a session'")
        .execute(&hub.state.db)
        .await
        .unwrap();
    assert_eq!(
        hub.get(&session, "/me").await.status,
        StatusCode::UNAUTHORIZED
    );
    let tokens = hub.get(&ada, "/me/tokens").await.json();
    assert!(tokens.as_array().unwrap().is_empty(), "{tokens}");
    let reply = hub
        .post(
            &ada,
            "/me/tokens",
            json!({"name": "never", "expires_in_minutes": 0}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
}

// ── The forge ─────────────────────────────────────────────────────────

#[tokio::test]
async fn without_a_forge_no_group_is_made() {
    let hub = TestHub::with(Config::default(), FakeForge::start().await).await;
    let reply = hub
        .post(
            &Client::default(),
            "/setup",
            json!({"token": "setup-secret", "username": "ada", "display_name": "Ada", "password": "analytical"}),
        )
        .await;
    let ada = signed_in(&reply);
    let reply = hub
        .post(
            &ada,
            "/groups",
            json!({"slug": "geometry", "name": "Geometry"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.json()["error"]["code"], "no_forge");
    assert!(
        hub.get(&ada, "/me").await.json()["groups"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // Nor does it speak OpenID Connect.
    let reply = hub
        .fetch(&Client::default(), "/.well-known/openid-configuration")
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_group_is_an_organization_with_a_team_per_role() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let forge = &hub.forge;
    assert!(forge.organization_exists("geometry"));
    assert!(forge.repository_exists("geometry", "library"));
    assert_eq!(
        forge.hooks("geometry", "library"),
        ["http://127.0.0.1:8080/forge/hook"]
    );
    assert_eq!(
        forge.team_permission("geometry", "Members").unwrap(),
        "write"
    );
    assert_eq!(forge.team_permission("geometry", "Guests").unwrap(), "read");
    // The hub's own account stays an owner.
    assert_eq!(
        forge.team_members("geometry", "Owners"),
        ["ada", stemma_fake_forge::ADMIN]
    );
    let subject: String = sqlx::query_scalar("SELECT subject FROM users WHERE username = 'ada'")
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    assert_eq!(forge.linked_login(1, &subject).as_deref(), Some("ada"));

    // The teams follow the roles.
    let bob = hub.member(&ada, "bob").await;
    hub.join(&ada, "gus", "guest").await;
    assert_eq!(forge.team_members("geometry", "Members"), ["bob"]);
    assert_eq!(forge.team_members("geometry", "Guests"), ["gus"]);
    let bob_id = hub.get(&bob, "/me").await.json()["id"].as_i64().unwrap();
    let reply = hub
        .call(
            &ada,
            Method::PATCH,
            &format!("/groups/geometry/members/{bob_id}"),
            Some(json!({"role": "admin"})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert!(forge.team_members("geometry", "Members").is_empty());
    assert_eq!(
        forge.team_members("geometry", "Owners"),
        ["ada", "bob", stemma_fake_forge::ADMIN]
    );
    let reply = hub
        .call(
            &bob,
            Method::DELETE,
            &format!("/groups/geometry/members/{bob_id}"),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert_eq!(
        forge.team_members("geometry", "Owners"),
        ["ada", stemma_fake_forge::ADMIN]
    );

    // The library is shown where it is in the forge.
    let library = hub.get(&ada, "/groups/geometry/library").await.json();
    assert_eq!(library["available"], true);
    assert_eq!(library["clone_path"], "/git/geometry/library.git");
    assert_eq!(
        library["forge"]["repository"],
        format!("{}/geometry/library", forge.url)
    );
    assert_eq!(
        library["forge"]["organization"],
        format!("{}/geometry", forge.url)
    );
}

#[tokio::test]
async fn a_group_the_forge_cannot_hold_is_not_made() {
    let hub = TestHub::new().await;
    let reply = hub
        .post(
            &Client::default(),
            "/setup",
            json!({"token": "setup-secret", "username": "ada", "display_name": "Ada", "password": "analytical"}),
        )
        .await;
    let ada = signed_in(&reply);

    // A name the forge has already.
    hub.forge.add_user("algebra");
    let reply = hub
        .post(
            &ada,
            "/groups",
            json!({"slug": "algebra", "name": "Algebra"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CONFLICT, "{:?}", reply.json());

    // A name the forge keeps.
    let reply = hub
        .post(
            &ada,
            "/groups",
            json!({"slug": "explore", "name": "Explore"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);

    // A library the forge does not make: nothing is left behind.
    hub.forge.refuse_repositories(true);
    let reply = hub
        .post(
            &ada,
            "/groups",
            json!({"slug": "geometry", "name": "Geometry"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY, "{:?}", reply.json());
    assert!(!hub.forge.organization_exists("geometry"));
    assert_eq!(
        hub.get(&ada, "/groups/geometry").await.status,
        StatusCode::NOT_FOUND
    );
    hub.forge.refuse_repositories(false);
    let reply = hub
        .post(
            &ada,
            "/groups",
            json!({"slug": "geometry", "name": "Geometry"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.json());
}

#[tokio::test]
async fn names_the_forge_keeps_are_not_usernames() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let reply = hub
        .post(&ada, "/groups/geometry/invitations", json!({}))
        .await;
    let token = reply.json()["token"].as_str().unwrap().to_owned();
    for username in ["admin", "explore", "ada.", "a--b"] {
        let reply = hub
            .post(
                &Client::default(),
                &format!("/invitations/{token}/accept"),
                json!({"account": {"username": username, "display_name": "X", "password": "password1"}}),
            )
            .await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{username}");
    }
}

#[tokio::test]
async fn someone_whose_name_the_forge_has_still_joins() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    // Someone in Forgejo, not linked to the hub, has the name.
    hub.forge.add_user("bob");
    let bob = hub.member(&ada, "bob").await;
    assert_eq!(
        hub.get(&bob, "/groups/geometry").await.json()["role"],
        "member"
    );
    // That account is not given the group.
    assert!(hub.forge.team_members("geometry", "Members").is_empty());
}

#[tokio::test]
async fn webhooks_must_be_signed() {
    let hub = TestHub::new().await;
    hub.admin_with_group().await;
    let body = json!({
        "ref": "refs/heads/main",
        "repository": {"name": "library", "owner": {"login": "geometry"}},
        "pusher": {"login": "ada"},
    })
    .to_string();
    let send = async |signature: Option<String>, event: &str| {
        let mut request = Request::post("/forge/hook")
            .header(header::CONTENT_TYPE, "application/json")
            .header("x-forgejo-event", event);
        if let Some(signature) = signature {
            request = request.header("x-forgejo-signature", signature);
        }
        hub.send(
            &Client::default(),
            request.body(Body::from(body.clone())).unwrap(),
        )
        .await
        .status
    };
    assert_eq!(send(None, "push").await, StatusCode::UNAUTHORIZED);
    let forged = stemma_hub::forge::hook::sign("another secret", body.as_bytes());
    assert_eq!(send(Some(forged), "push").await, StatusCode::UNAUTHORIZED);
    let signed = stemma_hub::forge::hook::sign(WEBHOOK_SECRET, body.as_bytes());
    assert_eq!(
        send(Some(signed.clone()), "push").await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(send(Some(signed), "issues").await, StatusCode::NO_CONTENT);
}
