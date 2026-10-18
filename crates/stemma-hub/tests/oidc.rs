//! The hub as an OpenID Connect provider for the forge: each check of the
//! authorization code flow.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use stemma_core::api::NewAccount;
use stemma_fake_forge::FakeForge;
use stemma_hub::config::{ForgeConfig, ForgeSecrets};
use tower::ServiceExt;

const ISSUER: &str = "https://stemma.example.org";
const BACK_CHANNEL: &str = "http://hub:8080";
const CALLBACK: &str = "https://forge.example.org/user/oauth2/stemma/callback";
const CLIENT: &str = "forgejo";
const SECRET: &str = "the client's long secret";
const VERIFIER: &str = "a-verifier-of-pkce-that-is-long-enough-to-be-valid";

struct TestHub {
    app: Router,
    state: stemma_hub::AppState,
    _forge: FakeForge,
    _data: tempfile::TempDir,
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

    /// Where it sends the browser.
    fn location(&self) -> url::Url {
        let location = self.headers[header::LOCATION].to_str().unwrap();
        url::Url::parse("http://hub.test")
            .unwrap()
            .join(location)
            .unwrap()
    }

    /// The parameters of where it sends the browser.
    fn sent_back(&self) -> HashMap<String, String> {
        self.location().query_pairs().into_owned().collect()
    }
}

impl TestHub {
    async fn new(require_pkce: bool) -> TestHub {
        let forge = FakeForge::start().await;
        let data = tempfile::tempdir().unwrap();
        let mut forge_config = ForgeConfig::new(
            &forge.url,
            CALLBACK,
            ForgeSecrets {
                token: stemma_fake_forge::TOKEN.into(),
                webhook_secret: "the webhook's long secret".into(),
                client_secret: SECRET.into(),
            },
        );
        forge_config.hub_url = Some(BACK_CHANNEL.into());
        forge_config.oidc.require_pkce = require_pkce;
        let config = stemma_hub::Config {
            data_dir: data.path().to_owned(),
            public_url: Some(ISSUER.into()),
            forge: Some(forge_config),
            ..Default::default()
        };
        let state = Arc::new(
            stemma_hub::Hub::new(stemma_hub::db::in_memory().await.unwrap(), config).unwrap(),
        );
        TestHub {
            app: stemma_hub::app(state.clone()),
            state,
            _forge: forge,
            _data: data,
        }
    }

    async fn send(&self, request: Request<Body>) -> Reply {
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

    async fn get(&self, path: &str, cookie: Option<&str>) -> Reply {
        let mut request = Request::get(path);
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        self.send(request.body(Body::empty()).unwrap()).await
    }

    /// Ada's session cookie.
    async fn ada(&self) -> String {
        let account = NewAccount {
            username: "ada".into(),
            display_name: "Ada Lovelace".into(),
            password: "analytical".into(),
        };
        stemma_hub::api::create_user(&self.state, account, false)
            .await
            .unwrap();
        let reply = self
            .send(
                Request::post("/api/v1/session")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({"username": "ada", "password": "analytical"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await;
        let cookie = reply.headers[header::SET_COOKIE].to_str().unwrap();
        cookie.split(';').next().unwrap().to_owned()
    }

    /// Asks to sign in with `params` over the right ones, as `cookie`.
    async fn authorize(&self, cookie: Option<&str>, params: &[(&str, &str)]) -> Reply {
        let mut all: Vec<(&str, &str)> = vec![
            ("response_type", "code"),
            ("client_id", CLIENT),
            ("redirect_uri", CALLBACK),
            ("scope", "openid profile email"),
            ("state", "the-state"),
            ("nonce", "the-nonce"),
        ];
        for (key, value) in params {
            all.retain(|(k, _)| k != key);
            if !value.is_empty() {
                all.push((key, value));
            }
        }
        let query: String = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(all)
            .finish();
        self.get(&format!("/oauth/authorize?{query}"), cookie).await
    }

    /// A code for Ada, with PKCE.
    async fn code(&self, cookie: &str) -> String {
        let reply = self
            .authorize(
                Some(cookie),
                &[
                    ("code_challenge", &challenge(VERIFIER)),
                    ("code_challenge_method", "S256"),
                ],
            )
            .await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        reply.sent_back()["code"].clone()
    }

    /// Exchanges at the token endpoint, with basic credentials if given.
    async fn token(&self, basic: Option<(&str, &str)>, form: &[(&str, &str)]) -> Reply {
        let body: String = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        let mut request = Request::post("/oauth/token")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some((id, secret)) = basic {
            let credentials = STANDARD.encode(format!("{id}:{secret}"));
            request = request.header(header::AUTHORIZATION, format!("Basic {credentials}"));
        }
        self.send(request.body(Body::from(body)).unwrap()).await
    }

    async fn exchange(&self, code: &str, verifier: &str) -> Reply {
        self.token(
            Some((CLIENT, SECRET)),
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", CALLBACK),
                ("code_verifier", verifier),
            ],
        )
        .await
    }

    async fn userinfo(&self, token: &str) -> Reply {
        self.send(
            Request::get("/oauth/userinfo")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }
}

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

#[tokio::test]
async fn discovery_tells_where_everything_is() {
    let hub = TestHub::new(false).await;
    let reply = hub.get("/.well-known/openid-configuration", None).await;
    assert_eq!(reply.status, StatusCode::OK);
    let doc = reply.json();
    assert_eq!(doc["issuer"], ISSUER);
    // The browser goes to the public address; the forge, to its own.
    assert_eq!(
        doc["authorization_endpoint"],
        format!("{ISSUER}/oauth/authorize")
    );
    assert_eq!(doc["token_endpoint"], format!("{BACK_CHANNEL}/oauth/token"));
    assert_eq!(doc["jwks_uri"], format!("{BACK_CHANNEL}/oauth/jwks"));
    assert_eq!(doc["code_challenge_methods_supported"], json!(["S256"]));
    let keys = hub.get("/oauth/jwks", None).await.json();
    assert_eq!(keys["keys"][0]["alg"], "ES256");
    assert!(keys["keys"][0].get("d").is_none(), "no private part");
}

#[tokio::test]
async fn a_signed_in_person_signs_in_to_the_forge() {
    let hub = TestHub::new(false).await;
    let cookie = hub.ada().await;
    let reply = hub
        .authorize(
            Some(&cookie),
            &[
                ("code_challenge", &challenge(VERIFIER)),
                ("code_challenge_method", "S256"),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    let location = reply.location();
    assert_eq!(
        format!(
            "{}://{}{}",
            location.scheme(),
            location.host_str().unwrap(),
            location.path()
        ),
        CALLBACK
    );
    let back = reply.sent_back();
    assert_eq!(back["state"], "the-state");
    assert_eq!(back["iss"], ISSUER);

    let reply = hub.exchange(&back["code"], VERIFIER).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());
    assert_eq!(reply.headers[header::CACHE_CONTROL], "no-store");
    let tokens = reply.json();
    assert_eq!(tokens["token_type"], "Bearer");

    // The ID token is signed with the published key, for the client.
    let jwk = hub.get("/oauth/jwks", None).await.json()["keys"][0].clone();
    let key = jsonwebtoken::DecodingKey::from_ec_components(
        jwk["x"].as_str().unwrap(),
        jwk["y"].as_str().unwrap(),
    )
    .unwrap();
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
    validation.set_audience(&[CLIENT]);
    validation.set_issuer(&[ISSUER]);
    let id = jsonwebtoken::decode::<Value>(tokens["id_token"].as_str().unwrap(), &key, &validation)
        .unwrap();
    let ada_subject: String =
        sqlx::query_scalar("SELECT subject FROM users WHERE username = 'ada'")
            .fetch_one(&hub.state.db)
            .await
            .unwrap();
    assert_eq!(id.claims["sub"], ada_subject);
    assert_eq!(id.claims["nonce"], "the-nonce");
    assert_eq!(id.claims["preferred_username"], "ada");
    assert_eq!(id.claims["name"], "Ada Lovelace");
    assert_eq!(id.claims["email"], "ada@stemma.invalid");

    // The access token reads who they are.
    let info = hub.userinfo(tokens["access_token"].as_str().unwrap()).await;
    assert_eq!(info.status, StatusCode::OK);
    assert_eq!(info.json()["sub"], ada_subject);
    assert_eq!(info.json()["nickname"], "ada");
    // Nothing else does.
    let reply = hub.userinfo("sto_not-a-token").await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn someone_not_signed_in_signs_in_first() {
    let hub = TestHub::new(false).await;
    let reply = hub.authorize(None, &[]).await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    let location = reply.location();
    assert_eq!(location.path(), "/signin");
    let next = &reply.sent_back()["next"];
    assert!(next.starts_with("/oauth/authorize?"), "{next}");
    assert!(next.contains("state=the-state"));
    // Unless the client asked not to be shown anything.
    let reply = hub.authorize(None, &[("prompt", "none")]).await;
    assert_eq!(reply.sent_back()["error"], "login_required");
}

#[tokio::test]
async fn an_unknown_client_or_redirect_is_never_sent_back() {
    let hub = TestHub::new(false).await;
    let cookie = hub.ada().await;
    for params in [
        vec![("client_id", "someone-else")],
        vec![("client_id", "")],
        vec![("redirect_uri", "https://evil.example.org/callback")],
        // Exactly, not as a prefix or with more.
        vec![(
            "redirect_uri",
            "https://forge.example.org/user/oauth2/stemma/callback/x",
        )],
        vec![(
            "redirect_uri",
            "https://forge.example.org/user/oauth2/stemma/callback?x=1",
        )],
        vec![("redirect_uri", "")],
    ] {
        let reply = hub.authorize(Some(&cookie), &params).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{params:?}");
        assert!(reply.headers.get(header::LOCATION).is_none());
    }
    // A parameter given twice.
    let query = format!(
        "response_type=code&client_id={CLIENT}&redirect_uri={CALLBACK}&scope=openid&state=s&state=t"
    );
    let reply = hub
        .get(&format!("/oauth/authorize?{query}"), Some(&cookie))
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn bad_requests_go_back_with_an_error() {
    let hub = TestHub::new(false).await;
    let cookie = hub.ada().await;
    let long = "x".repeat(600);
    for (params, error) in [
        (
            vec![("response_type", "token")],
            "unsupported_response_type",
        ),
        (vec![("scope", "profile")], "invalid_scope"),
        (vec![("state", "")], "invalid_request"),
        (vec![("state", long.as_str())], "invalid_request"),
        (vec![("nonce", long.as_str())], "invalid_request"),
        (vec![("code_challenge", VERIFIER)], "invalid_request"),
        (
            vec![
                ("code_challenge", VERIFIER),
                ("code_challenge_method", "plain"),
            ],
            "invalid_request",
        ),
        (
            vec![
                ("code_challenge", "too-short"),
                ("code_challenge_method", "S256"),
            ],
            "invalid_request",
        ),
        (vec![("request", "eyJ...")], "request_not_supported"),
    ] {
        let reply = hub.authorize(Some(&cookie), &params).await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER, "{params:?}");
        let back = reply.sent_back();
        assert_eq!(back["error"], error, "{params:?}");
        assert!(!back.contains_key("code"));
    }
}

#[tokio::test]
async fn a_token_is_not_a_session() {
    let hub = TestHub::new(false).await;
    let cookie = hub.ada().await;
    let created = hub
        .send(
            Request::post("/api/v1/me/tokens")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"name": "t"}).to_string()))
                .unwrap(),
        )
        .await
        .json();
    let token = created["secret"].as_str().unwrap();
    let reply = hub
        .send(
            Request::get(format!(
                "/oauth/authorize?response_type=code&client_id={CLIENT}&redirect_uri={CALLBACK}&scope=openid&state=s"
            ))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
        )
        .await;
    assert_eq!(reply.location().path(), "/signin");
}

#[tokio::test]
async fn only_the_client_with_its_secret_exchanges() {
    let hub = TestHub::new(false).await;
    let cookie = hub.ada().await;
    let form = |code: String| {
        vec![
            ("grant_type", "authorization_code".to_owned()),
            ("code", code),
            ("redirect_uri", CALLBACK.to_owned()),
            ("code_verifier", VERIFIER.to_owned()),
        ]
    };
    let as_pairs =
        |form: &[(&'static str, String)]| -> Vec<(&'static str, String)> { form.to_vec() };

    // A wrong secret, or none, or another client.
    let code = hub.code(&cookie).await;
    for basic in [Some((CLIENT, "wrong")), Some(("someone", SECRET)), None] {
        let pairs = as_pairs(&form(code.clone()));
        let pairs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let reply = hub.token(basic, &pairs).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{basic:?}");
        assert_eq!(reply.json()["error"], "invalid_client");
        assert!(reply.headers.contains_key(header::WWW_AUTHENTICATE));
    }

    // In the form works too; both ways at once does not.
    let code = hub.code(&cookie).await;
    let mut pairs = as_pairs(&form(code.clone()));
    pairs.push(("client_id", CLIENT.into()));
    pairs.push(("client_secret", SECRET.into()));
    let pairs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let reply = hub.token(Some((CLIENT, SECRET)), &pairs).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    let reply = hub.token(None, &pairs).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());

    // Only codes are exchanged.
    let reply = hub
        .token(
            Some((CLIENT, SECRET)),
            &[("grant_type", "client_credentials")],
        )
        .await;
    assert_eq!(reply.json()["error"], "unsupported_grant_type");
}

#[tokio::test]
async fn a_code_is_used_once_and_using_it_again_revokes_its_tokens() {
    let hub = TestHub::new(false).await;
    let cookie = hub.ada().await;
    let code = hub.code(&cookie).await;
    let first = hub.exchange(&code, VERIFIER).await;
    assert_eq!(first.status, StatusCode::OK);
    let access = first.json()["access_token"].as_str().unwrap().to_owned();
    assert_eq!(hub.userinfo(&access).await.status, StatusCode::OK);

    let again = hub.exchange(&code, VERIFIER).await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST);
    assert_eq!(again.json()["error"], "invalid_grant");
    assert_eq!(hub.userinfo(&access).await.status, StatusCode::UNAUTHORIZED);

    let reply = hub.exchange("not-a-code", VERIFIER).await;
    assert_eq!(reply.json()["error"], "invalid_grant");
}

#[tokio::test]
async fn codes_expire() {
    let hub = TestHub::new(false).await;
    let cookie = hub.ada().await;
    let code = hub.code(&cookie).await;
    sqlx::query("UPDATE oidc_codes SET expires_at = 0")
        .execute(&hub.state.db)
        .await
        .unwrap();
    let reply = hub.exchange(&code, VERIFIER).await;
    assert_eq!(reply.json()["error"], "invalid_grant");
}

#[tokio::test]
async fn the_redirect_uri_must_be_the_codes() {
    let hub = TestHub::new(false).await;
    let cookie = hub.ada().await;
    let code = hub.code(&cookie).await;
    for redirect in ["https://forge.example.org/other", ""] {
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("code_verifier", VERIFIER),
        ];
        if !redirect.is_empty() {
            form.push(("redirect_uri", redirect));
        }
        let reply = hub.token(Some((CLIENT, SECRET)), &form).await;
        assert_eq!(reply.json()["error"], "invalid_grant", "{redirect}");
        // The code is spent by trying.
        let reply = hub.exchange(&code, VERIFIER).await;
        assert_eq!(reply.json()["error"], "invalid_grant");
    }
}

#[tokio::test]
async fn a_codes_client_must_be_the_one_exchanging_it() {
    let hub = TestHub::new(false).await;
    let cookie = hub.ada().await;
    let code = hub.code(&cookie).await;
    sqlx::query("UPDATE oidc_codes SET client_id = 'another'")
        .execute(&hub.state.db)
        .await
        .unwrap();
    let reply = hub.exchange(&code, VERIFIER).await;
    assert_eq!(reply.json()["error"], "invalid_grant");
}

#[tokio::test]
async fn pkce_verifiers_are_checked() {
    let hub = TestHub::new(false).await;
    let cookie = hub.ada().await;

    // A wrong verifier spends the code, so it cannot be guessed at.
    let code = hub.code(&cookie).await;
    let wrong = "another-verifier-of-pkce-that-is-long-enough-to-be-valid";
    assert_eq!(
        hub.exchange(&code, wrong).await.json()["error"],
        "invalid_grant"
    );
    assert_eq!(
        hub.exchange(&code, VERIFIER).await.json()["error"],
        "invalid_grant"
    );

    // A missing or malformed one.
    for verifier in ["", "short", &format!("{VERIFIER} with spaces")] {
        let code = hub.code(&cookie).await;
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", CALLBACK),
        ];
        if !verifier.is_empty() {
            form.push(("code_verifier", verifier));
        }
        let reply = hub.token(Some((CLIENT, SECRET)), &form).await;
        assert_eq!(reply.json()["error"], "invalid_grant", "{verifier}");
    }

    // A verifier for a code that had no challenge.
    let reply = hub.authorize(Some(&cookie), &[]).await;
    let code = reply.sent_back()["code"].clone();
    assert_eq!(
        hub.exchange(&code, VERIFIER).await.json()["error"],
        "invalid_grant"
    );
}

#[tokio::test]
async fn a_client_that_must_use_pkce_does() {
    let hub = TestHub::new(true).await;
    let cookie = hub.ada().await;
    let reply = hub.authorize(Some(&cookie), &[]).await;
    assert_eq!(reply.sent_back()["error"], "invalid_request");
    let code = hub.code(&cookie).await;
    assert_eq!(hub.exchange(&code, VERIFIER).await.status, StatusCode::OK);
}
