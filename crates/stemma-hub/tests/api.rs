//! The API, end to end, against a fresh database.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use stemma_hub::{Config, Hub, app, db};
use tower::ServiceExt;

struct TestHub {
    app: Router,
    state: stemma_hub::AppState,
    _data: tempfile::TempDir,
}

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
        let data = tempfile::tempdir().unwrap();
        let config = Config {
            data_dir: data.path().to_owned(),
            max_upload_mb: 1,
            ..Config::default()
        };
        let hub = Arc::new(Hub::new(db::in_memory().await.unwrap(), config));
        *hub.setup_token.lock().unwrap() = Some("setup-secret".into());
        TestHub {
            app: app(hub.clone()),
            state: hub,
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
        let reply = self
            .post(
                admin,
                "/groups/geometry/invitations",
                json!({"max_uses": 1}),
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
    assert_eq!(resolved[5]["kind"], "paper");
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
        ("/groups/geometry/threads", json!({"title": "New"})),
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

#[tokio::test]
async fn admins_keep_the_agents_of_a_group() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let bob = hub.member(&ada, "bob").await;

    let agents = hub.get(&bob, "/groups/geometry/agents").await.json();
    let handles: Vec<&str> = agents
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["handle"].as_str().unwrap())
        .collect();
    assert_eq!(handles, ["claude", "codex", "editor", "workshop"]);
    assert_eq!(agents[0]["role"], "answer");
    assert_eq!(agents[2]["role"], "editor");
    assert_eq!(agents[3]["role"], "workshop");
    assert_eq!(agents[3]["runner"], "opencode");
    assert_eq!(agents[3]["scope"], "group");

    let qwen = json!({"handle": "@Qwen", "display_name": "Qwen", "runner": "api",
                      "model": "qwen/qwen3", "endpoint": "openrouter", "scope": "group"});
    let reply = hub
        .post(&bob, "/groups/geometry/agents", qwen.clone())
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let reply = hub
        .post(&ada, "/groups/geometry/agents", qwen.clone())
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.json());
    assert_eq!(reply.json()["handle"], "qwen");
    let reply = hub.post(&ada, "/groups/geometry/agents", qwen).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);

    let reply = hub
        .post(
            &ada,
            "/groups/geometry/agents",
            json!({"handle": "x", "display_name": "X", "runner": "nope", "scope": "group"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);

    let reply = hub
        .call(
            &ada,
            Method::PUT,
            "/groups/geometry/agents/qwen",
            Some(
                json!({"handle": "ignored", "display_name": "Qwen 3", "runner": "api",
                        "model": "", "scope": "group"}),
            ),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());
    assert_eq!(reply.json()["handle"], "qwen");
    assert!(reply.json()["model"].is_null());

    let reply = hub
        .call(&ada, Method::DELETE, "/groups/geometry/agents/qwen", None)
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
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
        "Waiting for Ada's worker, with Claude Code, to come online"
    );

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
async fn workers_are_registered_by_whom_they_serve() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let bob = hub.member(&ada, "bob").await;

    let reply = hub
        .post(&bob, "/groups/geometry/workers", json!({"name": "server"}))
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let reply = hub
        .post(&ada, "/groups/geometry/workers", json!({"name": "server"}))
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    assert!(reply.json()["secret"].as_str().unwrap().starts_with("stw_"));
    assert_eq!(reply.json()["scope"], "group");

    let reply = hub
        .post(&bob, "/me/workers", json!({"name": "laptop"}))
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    assert_eq!(reply.json()["owner"]["username"], "bob");
    let id = reply.json()["id"].as_i64().unwrap();

    let group = hub.get(&bob, "/groups/geometry/workers").await.json();
    assert_eq!(group[0]["name"], "server");
    assert_eq!(group[0]["online"], false);
    let mine = hub.get(&bob, "/me/workers").await.json();
    assert_eq!(mine.as_array().unwrap().len(), 1);

    let reply = hub
        .call(&ada, Method::DELETE, &format!("/me/workers/{id}"), None)
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let reply = hub
        .call(&bob, Method::DELETE, &format!("/me/workers/{id}"), None)
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert!(
        hub.get(&bob, "/me/workers")
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
async fn papers_keep_their_revisions_and_who_read_them() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let bob = hub.member(&ada, "bob").await;
    let first = "Where we are.\n\n# Setting\n\nA scheme $X$.\n\n## Charts\n\nOpen.\n";

    for (slug, status) in [("Bad Slug", StatusCode::UNPROCESSABLE_ENTITY)] {
        let reply = hub
            .post(
                &ada,
                "/groups/geometry/papers",
                json!({"slug": slug, "title": "T", "body": ""}),
            )
            .await;
        assert_eq!(reply.status, status);
    }
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/papers",
            json!({"slug": "compactification", "title": "Compactification", "body": first}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.json());
    assert_eq!(reply.json()["revision"], 1);
    assert_eq!(reply.json()["last_read"], 1);
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/papers",
            json!({"slug": "compactification", "title": "Again"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);

    // Bob has not read it: nothing to show as changed.
    let page = hub
        .get(&bob, "/groups/geometry/papers/compactification")
        .await
        .json();
    assert!(page["paper"]["last_read"].is_null());
    assert_eq!(page["since_read"], json!([]));
    assert_eq!(page["body"], first);
    let slugs: Vec<&str> = page["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs, ["setting", "charts"]);
    let reply = hub
        .post(
            &bob,
            "/groups/geometry/papers/compactification/read",
            json!({"revision": 1}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);

    // Ada writes a second revision; an edit over the first is refused.
    let second = first.replace("Open.", "Glued along $U \\cap V$.");
    let edit = |base: u32, body: &str| json!({"body": body, "reason": "The charts glue", "base_revision": base});
    let reply = hub
        .call(
            &ada,
            Method::PUT,
            "/groups/geometry/papers/compactification",
            Some(edit(1, &second)),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());
    assert_eq!(reply.json()["revision"], 2);
    let reply = hub
        .call(
            &bob,
            Method::PUT,
            "/groups/geometry/papers/compactification",
            Some(edit(1, "Mine")),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert!(
        reply.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("revision 2")
    );

    // Bob now sees what changed since he read it.
    let page = hub
        .get(&bob, "/groups/geometry/papers/compactification")
        .await
        .json();
    assert_eq!(page["paper"]["last_read"], 1);
    assert_eq!(page["since_read"].as_array().unwrap().len(), 1);
    assert_eq!(page["since_read"][0]["reason"], "The charts glue");
    assert_eq!(page["since_read"][0]["author"]["username"], "ada");
    assert_eq!(page["read_body"], first);
    let papers = hub.get(&bob, "/groups/geometry/papers").await.json();
    assert_eq!(papers[0]["revision"], 2);
    assert_eq!(papers[0]["last_read"], 1);

    // An agent writes for Ada through her token; she has not read it yet.
    let token = with_token(&hub, &ada).await;
    let reply = hub
        .call(
            &token,
            Method::PUT,
            "/groups/geometry/papers/compactification",
            Some(
                json!({"body": "Short.", "reason": "Shorter", "base_revision": 2,
                        "agent": "Claude Code", "title": "Compactifications"}),
            ),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());
    assert_eq!(reply.json()["updated_agent"], "Claude Code");
    assert_eq!(reply.json()["title"], "Compactifications");
    let page = hub
        .get(&ada, "/groups/geometry/papers/compactification")
        .await
        .json();
    assert_eq!(page["paper"]["last_read"], 2);
    let reply = hub
        .call(
            &ada,
            Method::PUT,
            "/groups/geometry/papers/compactification",
            Some(json!({"body": "x", "reason": "y", "base_revision": 3, "agent": "Me"})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);

    // A job's token reads, and does not write.
    let job = job_token(&hub, 1).await;
    assert_eq!(
        hub.get(&job, "/groups/geometry/papers/compactification")
            .await
            .status,
        StatusCode::OK
    );
    let reply = hub
        .call(
            &job,
            Method::PUT,
            "/groups/geometry/papers/compactification",
            Some(edit(3, "Sneaky")),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);

    // The history.
    let history = hub
        .get(&bob, "/groups/geometry/papers/compactification/revisions")
        .await
        .json();
    let numbers: Vec<i64> = history
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["number"].as_i64().unwrap())
        .collect();
    assert_eq!(numbers, [3, 2, 1]);
    assert_eq!(history[0]["agent"], "Claude Code");
    let old = hub
        .get(&bob, "/groups/geometry/papers/compactification/revisions/1")
        .await
        .json();
    assert_eq!(old["body"], first);
    assert_eq!(old["reason"], "Started the paper");
}

#[tokio::test]
async fn threads_anchor_to_papers_and_references_resolve() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    hub.post(
        &ada,
        "/groups/geometry/papers",
        json!({"slug": "notes", "title": "Notes", "body": "# Gluing\n\nText.\n"}),
    )
    .await;
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/threads",
            json!({"title": "About gluing", "anchor": "p/notes#gluing", "body": "Is it right?"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.json());
    assert_eq!(reply.json()["thread"]["anchor"], "p/notes#gluing");
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/threads",
            json!({"title": "Nowhere", "anchor": "p/none"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);

    let page = hub.get(&ada, "/groups/geometry/papers/notes").await.json();
    assert_eq!(page["threads"].as_array().unwrap().len(), 1);
    assert_eq!(page["threads"][0]["number"], 1);

    let resolved = hub
        .post(
            &ada,
            "/groups/geometry/resolve",
            json!({"addresses": ["p/notes", "p/notes#gluing", "p/notes#nowhere", "p/none"]}),
        )
        .await
        .json();
    let exists: Vec<bool> = resolved
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["exists"].as_bool().unwrap())
        .collect();
    assert_eq!(exists, [true, true, false, false]);
    assert_eq!(resolved[1]["title"], "Notes § Gluing");
    assert_eq!(resolved[1]["path"], "/g/geometry/p/notes#gluing");
}

#[tokio::test]
async fn editors_are_asked_to_update_papers() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    hub.post(
        &ada,
        "/groups/geometry/threads",
        json!({"title": "Decisions", "body": "@editor update p/compactification with this."}),
    )
    .await;
    let jobs = hub
        .get(&ada, "/groups/geometry/threads/1/jobs")
        .await
        .json();
    assert_eq!(jobs[0]["kind"], "paper_update", "{jobs}");
    assert_eq!(jobs[0]["paper"], "compactification");
    assert_eq!(jobs[0]["agent_name"], "Editor");
    assert_eq!(jobs[0]["state"], "queued");

    hub.post(
        &ada,
        "/groups/geometry/threads/1/messages",
        json!({"body": "@editor and now?"}),
    )
    .await;
    let jobs = hub
        .get(&ada, "/groups/geometry/threads/1/jobs")
        .await
        .json();
    assert_eq!(jobs[1]["state"], "failed");
    assert_eq!(
        jobs[1]["error"],
        "Say which paper @editor should update, as p/its-name"
    );
}

#[tokio::test]
async fn the_editor_updates_papers_at_night() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let bob = hub.member(&ada, "bob").await;
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/papers",
            json!({"slug": "notes", "title": "Notes", "body": "Nothing yet."}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let pause = std::time::Duration::from_millis(5);
    tokio::time::sleep(pause).await;
    hub.post(
        &bob,
        "/groups/geometry/threads",
        json!({"title": "About the notes", "anchor": "p/notes", "body": "The charts glue."}),
    )
    .await;
    hub.post(&ada, "/groups/geometry/threads", json!({"title": "Other"}))
        .await;

    // A night from now on, for two hours.
    let now = time::OffsetDateTime::now_utc();
    let from = i32::from(now.hour()) * 60 + i32::from(now.minute());
    let to = (from + 120) % (24 * 60);
    let clock = |m: i32| format!("{:02}:{:02}", m / 60, m % 60);
    let reply = hub
        .call(
            &ada,
            Method::PUT,
            "/groups/geometry/workshop",
            Some(json!({"night_from": clock(from), "night_to": clock(to)})),
        )
        .await;
    assert_eq!(reply.json()["night_now"], true);

    // The editor runs on each person's worker: only someone who lends it.
    let nights = stemma_hub::workshop::nights(&hub.state).await.unwrap();
    let asked = stemma_hub::workshop::update_papers(&hub.state, &nights)
        .await
        .unwrap();
    assert!(!asked);
    hub.call(
        &bob,
        Method::PUT,
        "/groups/geometry/workshop",
        Some(json!({"lend": true})),
    )
    .await;
    let nights = stemma_hub::workshop::nights(&hub.state).await.unwrap();
    let asked = stemma_hub::workshop::update_papers(&hub.state, &nights)
        .await
        .unwrap();
    assert!(asked);
    let jobs = hub
        .get(&ada, "/groups/geometry/threads/1/jobs")
        .await
        .json();
    assert_eq!(jobs.as_array().unwrap().len(), 1, "{jobs}");
    assert_eq!(jobs[0]["kind"], "paper_update");
    assert_eq!(jobs[0]["paper"], "notes");
    assert_eq!(jobs[0]["requester"]["username"], "bob");
    assert_eq!(jobs[0]["trigger"], 1);

    // Once a night.
    let asked = stemma_hub::workshop::update_papers(&hub.state, &nights)
        .await
        .unwrap();
    assert!(!asked);
}

#[tokio::test]
async fn the_workshop_takes_tasks_from_conversation() {
    let hub = TestHub::new().await;
    let ada = hub.admin_with_group().await;
    let bob = hub.member(&ada, "bob").await;

    // A message to the workshop hands it a task, for the night: a night
    // that never comes, so that the test does not depend on the time.
    let reply = hub
        .call(
            &ada,
            Method::PUT,
            "/groups/geometry/workshop",
            Some(json!({"night_from": "03:00", "night_to": "03:00"})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["night_now"], false);
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
            json!({"body": "@workshop formalize the gluing of t1#1 tonight, with a docstring."}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let tasks = hub.get(&bob, "/groups/geometry/tasks").await.json();
    assert_eq!(tasks.as_array().unwrap().len(), 1, "{tasks}");
    let task = &tasks[0];
    assert_eq!(task["number"], 1);
    assert_eq!(
        task["title"],
        "Formalize the gluing of t1#1 tonight, with a docstring"
    );
    assert_eq!(task["state"], "open");
    assert_eq!(task["schedule"], "night");
    assert_eq!(task["origin"], "t1#2");
    assert_eq!(task["requester"]["username"], "ada");
    assert_eq!(task["progress"], "Waiting for the night, 03:00–03:00 (UTC)");
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
    assert_eq!(task["schedule"], "now");
    let page = hub.get(&bob, "/groups/geometry/tasks/2").await.json();
    assert_eq!(page["instructions"], "As in t1#1.");
    assert_eq!(
        page["progress"],
        "Waiting for a group worker with Lean and OpenCode to come online"
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

    // The night window is the admins'; lending one's worker, anyone's.
    let settings = hub.get(&bob, "/groups/geometry/workshop").await.json();
    assert_eq!(settings["night_from"], "03:00");
    assert_eq!(settings["night_to"], "03:00");
    assert_eq!(settings["time_zone"], "UTC");
    assert_eq!(settings["lend"], false);
    let reply = hub
        .call(
            &bob,
            Method::PUT,
            "/groups/geometry/workshop",
            Some(json!({"night_from": "22:00"})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let reply = hub
        .call(
            &bob,
            Method::PUT,
            "/groups/geometry/workshop",
            Some(json!({"lend": true})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["lend"], true);
    assert_eq!(reply.json()["lenders"][0]["username"], "bob");
    let reply = hub
        .call(
            &ada,
            Method::PUT,
            "/groups/geometry/workshop",
            Some(json!({"night_from": "22:30", "night_to": "6:00", "time_zone": "Europe/Madrid"})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());
    assert_eq!(reply.json()["night_from"], "22:30");
    assert_eq!(reply.json()["night_to"], "06:00");
    assert_eq!(reply.json()["lend"], false);
    let reply = hub
        .call(
            &ada,
            Method::PUT,
            "/groups/geometry/workshop",
            Some(json!({"time_zone": "Mars/Olympus"})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
}
