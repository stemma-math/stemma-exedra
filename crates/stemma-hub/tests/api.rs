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
    assert_eq!(jobs[0]["note"], "Waiting for one of Ada's machines");
    assert_eq!(jobs[1]["note"], "Waiting for one of Ada's machines");

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

// ── People's agents and machines ──────────────────────────────────────

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

/// Connect a real personal token to the scheduler without a network transport.
async fn computer(
    hub: &TestHub,
    person: &Client,
    name: &str,
    connection: u64,
    priority: i64,
) -> (
    i64,
    tokio::sync::mpsc::UnboundedReceiver<stemma_core::protocol::HubMessage>,
) {
    let token = hub
        .post(person, "/me/tokens", json!({"name": name}))
        .await
        .json();
    let id = token["id"].as_i64().unwrap();
    let owner = hub.get(person, "/me").await.json()["id"].as_i64().unwrap();
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    stemma_hub::jobs::connected(
        &hub.state,
        id,
        stemma_hub::jobs::Connected {
            connection,
            owner_id: owner,
            runners: vec!["claude-code".into(), "codex".into()],
            agents: stemma_core::protocol::GroupAgents {
                enabled: true,
                priority,
                concurrency: None,
            },
            groups: Default::default(),
            concurrency: 1,
            running: Default::default(),
            sender,
        },
    )
    .await
    .unwrap();
    (id, receiver)
}
fn dispatched(
    receiver: &mut tokio::sync::mpsc::UnboundedReceiver<stemma_core::protocol::HubMessage>,
) -> Option<stemma_core::protocol::JobSpec> {
    match receiver.try_recv().ok()? {
        stemma_core::protocol::HubMessage::Job(job) => Some(*job),
        _ => None,
    }
}

#[tokio::test]
async fn personal_tokens_route_by_group_priority_capacity_and_connection_time() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let bob = hub.member(&ada, "bob").await;
    let (_, mut bobs) = computer(&hub, &bob, "Bob's computer", 1, 100).await;
    hub.post(
        &ada,
        "/groups/geometry/threads",
        json!({"title":"Question", "body":"@claude help"}),
    )
    .await;
    let waiting = hub
        .get(&ada, "/groups/geometry/threads/1/jobs")
        .await
        .json();
    assert_eq!(waiting[0]["note"], "Waiting for one of Ada's machines");
    assert!(dispatched(&mut bobs).is_none());
    let (first, mut one) = computer(&hub, &ada, "Laptop", 2, 10).await;
    let initial = dispatched(&mut one).expect("queued work starts on connection");
    let (second, mut two) = computer(&hub, &ada, "Desktop", 3, 0).await;
    hub.post(
        &ada,
        "/groups/geometry/threads/1/messages",
        json!({"body":"@codex help too"}),
    )
    .await;
    let overflow = dispatched(&mut two).expect("the highest priority computer is full");
    stemma_hub::jobs::finished(
        &hub.state,
        second,
        3,
        overflow.id,
        stemma_core::protocol::Outcome::Answer {
            body: "A second answer".into(),
        },
        Default::default(),
    )
    .await
    .unwrap();
    let reply = hub
        .call(&ada, Method::DELETE, &format!("/me/tokens/{first}"), None)
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert!(one.is_closed());
    let retried = dispatched(&mut two).expect("revocation queues the running job again");
    assert_eq!(retried.id, initial.id);
    assert_ne!(retried.token, initial.token);
    let revoked: bool =
        sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM api_tokens WHERE token_hash = ?")
            .bind(stemma_hub::secret::token_hash(&initial.token))
            .fetch_one(&hub.state.db)
            .await
            .unwrap();
    assert!(revoked);
    stemma_hub::jobs::finished(
        &hub.state,
        second,
        3,
        retried.id,
        stemma_core::protocol::Outcome::Answer {
            body: "The answer".into(),
        },
        Default::default(),
    )
    .await
    .unwrap();
    let (third, mut three) = computer(&hub, &ada, "Newest", 4, 0).await;
    hub.post(
        &ada,
        "/groups/geometry/threads/1/messages",
        json!({"body":"@claude again"}),
    )
    .await;
    assert!(
        dispatched(&mut three).is_some(),
        "ties use the newest connection"
    );
    assert!(dispatched(&mut two).is_none());
    let hello = stemma_core::protocol::Hello {
        protocol: stemma_core::protocol::PROTOCOL_VERSION,
        version: "test".into(),
        runners: vec!["claude-code".into()],
        concurrency: 1,
        agents: stemma_core::protocol::GroupAgents {
            enabled: true,
            priority: 0,
            concurrency: None,
        },
        groups: std::collections::BTreeMap::from([(
            "geometry".into(),
            stemma_core::protocol::GroupAgents {
                enabled: false,
                priority: 99,
                concurrency: None,
            },
        )]),
    };
    stemma_hub::jobs::hello(&hub.state, second, 3, hello)
        .await
        .unwrap();
    hub.post(
        &ada,
        "/groups/geometry/threads/1/messages",
        json!({"body":"@claude wait"}),
    )
    .await;
    assert!(
        dispatched(&mut two).is_none(),
        "disabled group overrides priority"
    );
    let devices = hub.get(&ada, "/me/tokens").await.json();
    assert!(
        devices
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["id"] == third && d["connected"] == true)
    );
    for route in [
        "/me/machines",
        "/me/agents",
        "/groups/geometry/tasks",
        "/workshop/architectures",
    ] {
        assert_eq!(hub.get(&ada, route).await.status, StatusCode::NOT_FOUND);
    }
}
