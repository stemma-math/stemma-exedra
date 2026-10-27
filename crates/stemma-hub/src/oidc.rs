//! The hub as an OpenID Connect provider, so that people sign in to the
//! forge with their Stemma account.
//!
//! Only the clients in the configuration are served (the forge's), with
//! the authorization code flow:
//!
//! 1. The client sends the person to `/oauth/authorize`, with its id, a
//!    redirect URI that must be one of its own exactly, a `state`, and
//!    optionally a `nonce` and a PKCE challenge (`S256` only). Someone not
//!    signed in signs in to the web app first, and comes back.
//! 2. The hub sends them back with a code, which lasts a minute and is
//!    used once: using it again revokes what it gave.
//! 3. The client, authenticated with its secret, exchanges the code at
//!    `/oauth/token`, with the same redirect URI and the PKCE verifier,
//!    for an ID token, signed with the hub's key (ES256), and an access
//!    token for `/oauth/userinfo`.
//!
//! The subject is the account's own random one, never anyone else's; the
//! claims give its username, name and the address its forge account has.

use std::path::Path;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey, LineEnding};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::auth::{Viewer, bearer_token};
use crate::config::OidcClientConfig;
use crate::db;
use crate::secret;
use crate::state::AppState;

pub const AUTHORIZE_PATH: &str = "/oauth/authorize";
pub const TOKEN_PATH: &str = "/oauth/token";
pub const USERINFO_PATH: &str = "/oauth/userinfo";
pub const JWKS_PATH: &str = "/oauth/jwks";
pub const DISCOVERY_PATH: &str = "/.well-known/openid-configuration";

/// How long a code lasts, in milliseconds.
const CODE_MILLIS: i64 = 60 * 1000;

/// How long an access token lasts, and an ID token, in seconds.
const TOKEN_SECONDS: i64 = 60 * 60;
const ID_TOKEN_SECONDS: i64 = 10 * 60;

/// The longest `state` and `nonce` accepted.
const MAX_PARAMETER: usize = 512;

const SCOPES: &[&str] = &["openid", "profile", "email"];

pub fn router(state: AppState) -> Router {
    Router::new()
        .route(DISCOVERY_PATH, get(discovery))
        .route(JWKS_PATH, get(jwks))
        .route(AUTHORIZE_PATH, get(authorize))
        .route(TOKEN_PATH, axum::routing::post(token))
        .route(USERINFO_PATH, get(userinfo).post(userinfo))
        .with_state(state)
}

// ── The signing key ───────────────────────────────────────────────────

/// The P-256 key the hub signs ID tokens with, kept in the data directory.
pub struct SigningKey {
    encoding: EncodingKey,
    /// The public key, as a JWK.
    jwk: Value,
    kid: String,
}

impl SigningKey {
    /// Reads the key at `path`, or makes one there, readable only by the
    /// hub's user.
    pub fn load_or_create(path: &Path) -> anyhow::Result<SigningKey> {
        use anyhow::Context;
        let pem = match std::fs::read_to_string(path) {
            Ok(pem) => pem,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let pem = new_key()?;
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                write_private(path, &pem).with_context(|| format!("writing {}", path.display()))?;
                pem
            }
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", path.display()));
            }
        };
        Self::from_pem(&pem).with_context(|| format!("reading the key in {}", path.display()))
    }

    fn from_pem(pem: &str) -> anyhow::Result<SigningKey> {
        let secret = p256::SecretKey::from_pkcs8_pem(pem)?;
        let point = secret.public_key().to_encoded_point(false);
        let x = URL_SAFE_NO_PAD.encode(point.x().expect("an uncompressed point"));
        let y = URL_SAFE_NO_PAD.encode(point.y().expect("an uncompressed point"));
        // RFC 7638's thumbprint: the required members, in order.
        let thumbprint = format!(r#"{{"crv":"P-256","kty":"EC","x":"{x}","y":"{y}"}}"#);
        let kid = URL_SAFE_NO_PAD.encode(Sha256::digest(thumbprint.as_bytes()));
        let jwk = json!({
            "kty": "EC", "crv": "P-256", "x": x, "y": y,
            "use": "sig", "alg": "ES256", "kid": kid,
        });
        Ok(SigningKey {
            encoding: EncodingKey::from_ec_pem(pem.as_bytes())?,
            jwk,
            kid,
        })
    }

    /// The public key, as a JWK.
    pub fn jwk(&self) -> &Value {
        &self.jwk
    }

    fn sign(&self, claims: &impl Serialize) -> anyhow::Result<String> {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.kid.clone());
        Ok(jsonwebtoken::encode(&header, claims, &self.encoding)?)
    }
}

fn new_key() -> anyhow::Result<String> {
    loop {
        let mut bytes = [0u8; 32];
        rand::fill(&mut bytes);
        // Nearly every 32 bytes are a key; the others are tried again.
        if let Ok(secret) = p256::SecretKey::from_slice(&bytes) {
            return Ok(secret.to_pkcs8_pem(LineEnding::LF)?.to_string());
        }
    }
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(text.as_bytes())
}

// ── Clients ───────────────────────────────────────────────────────────

/// The client with `id`, if the hub serves it.
fn client<'a>(state: &'a AppState, id: &str) -> Option<(&'a OidcClientConfig, &'a str)> {
    let forge = state.config.forge.as_ref()?;
    state.signing_key.as_ref()?;
    (forge.oidc.client_id == id).then_some((&forge.oidc, forge.secrets.client_secret.as_str()))
}

/// Compares two secrets in constant time.
fn same_secret(given: &str, expected: &str) -> bool {
    Sha256::digest(given.as_bytes())
        .ct_eq(&Sha256::digest(expected.as_bytes()))
        .into()
}

/// The parameters of a query or a form, refused if any is repeated.
fn parameters(text: &str) -> Option<std::collections::HashMap<String, String>> {
    let mut map = std::collections::HashMap::new();
    for (key, value) in url::form_urlencoded::parse(text.as_bytes()) {
        if map.insert(key.into_owned(), value.into_owned()).is_some() {
            return None;
        }
    }
    Some(map)
}

// ── Discovery ─────────────────────────────────────────────────────────

/// The issuer: the hub's public address.
fn issuer(state: &AppState) -> String {
    state.config.base_url()
}

async fn discovery(State(state): State<AppState>) -> Response {
    if state.signing_key.is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let public = issuer(&state);
    // People's browsers go to the hub's public address; the client itself
    // reaches it where the configuration says.
    let back = state.config.hub_url_for_forge();
    axum::Json(json!({
        "issuer": public,
        "authorization_endpoint": format!("{public}{AUTHORIZE_PATH}"),
        "token_endpoint": format!("{back}{TOKEN_PATH}"),
        "userinfo_endpoint": format!("{back}{USERINFO_PATH}"),
        "jwks_uri": format!("{back}{JWKS_PATH}"),
        "response_types_supported": ["code"],
        "response_modes_supported": ["query"],
        "grant_types_supported": ["authorization_code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["ES256"],
        "scopes_supported": SCOPES,
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
        "code_challenge_methods_supported": ["S256"],
        "claims_supported": [
            "sub", "iss", "aud", "exp", "iat", "nonce", "azp",
            "name", "preferred_username", "nickname", "email", "email_verified",
        ],
        "authorization_response_iss_parameter_supported": true,
    }))
    .into_response()
}

async fn jwks(State(state): State<AppState>) -> Response {
    match &state.signing_key {
        Some(key) => axum::Json(json!({"keys": [key.jwk()]})).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// ── Authorization ─────────────────────────────────────────────────────

/// A request the hub cannot send back to the client, which is shown.
fn bad_request(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        format!("This sign-in cannot go on: {message}.\n"),
    )
        .into_response()
}

/// Sends the person back to the client with `params`.
fn back_to(redirect_uri: &str, params: &[(&str, &str)]) -> Response {
    let Ok(mut url) = url::Url::parse(redirect_uri) else {
        return bad_request("the redirect URI is not an address");
    };
    url.query_pairs_mut().extend_pairs(params);
    Redirect::to(url.as_str()).into_response()
}

fn is_pkce_value(value: &str, min: usize) -> bool {
    (min..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
}

async fn authorize(
    State(state): State<AppState>,
    viewer: Option<Viewer>,
    request: Request,
) -> Response {
    let query = request.uri().query().unwrap_or_default();
    let Some(params) = parameters(query) else {
        return bad_request("a parameter is repeated");
    };
    let param = |name: &str| params.get(name).map(String::as_str);

    // Until the client and where to send the person back are known to be
    // right, nothing is sent back.
    let Some((client, _)) = param("client_id").and_then(|id| client(&state, id)) else {
        return bad_request("the client is not known to this hub");
    };
    let Some(redirect_uri) = param("redirect_uri") else {
        return bad_request("the client did not say where to send you back");
    };
    if !client.redirect_uris.iter().any(|uri| uri == redirect_uri) {
        return bad_request("the client asked to send you back somewhere it may not");
    }

    let iss = issuer(&state);
    let client_state = param("state").unwrap_or_default();
    let fail = |error: &str, description: &str| {
        let mut back = vec![("error", error), ("error_description", description)];
        if !client_state.is_empty() && client_state.len() <= MAX_PARAMETER {
            back.push(("state", client_state));
        }
        back.push(("iss", &iss));
        back_to(redirect_uri, &back)
    };
    if param("response_type") != Some("code") {
        return fail("unsupported_response_type", "Only the code flow is served");
    }
    if params.contains_key("request") || params.contains_key("request_uri") {
        return fail("request_not_supported", "Request objects are not served");
    }
    let scopes: Vec<&str> = param("scope").unwrap_or_default().split(' ').collect();
    if !scopes.contains(&"openid") {
        return fail("invalid_scope", "The openid scope is needed");
    }
    let granted: Vec<&str> = SCOPES
        .iter()
        .copied()
        .filter(|s| scopes.contains(s))
        .collect();
    if client_state.is_empty() || client_state.len() > MAX_PARAMETER {
        return fail(
            "invalid_request",
            "A state, of at most 512 characters, is needed",
        );
    }
    let nonce = param("nonce").filter(|n| !n.is_empty());
    if nonce.is_some_and(|n| n.len() > MAX_PARAMETER) {
        return fail("invalid_request", "The nonce is too long");
    }
    let challenge = param("code_challenge");
    match (challenge, param("code_challenge_method")) {
        (None, None) if client.require_pkce => {
            return fail("invalid_request", "PKCE is needed");
        }
        (None, None) => {}
        (Some(challenge), Some("S256"))
            if challenge.len() == 43 && is_pkce_value(challenge, 43) => {}
        (Some(_), Some("S256")) => {
            return fail("invalid_request", "The code challenge is not an S256 one");
        }
        _ => {
            return fail(
                "invalid_request",
                "Only PKCE's S256 method is served, with its challenge",
            );
        }
    }

    // Only a session of the web app signs someone in.
    let Some(viewer) = viewer.filter(|v| !v.with_token) else {
        if param("prompt") == Some("none") {
            return fail("login_required", "Sign in to Stemma first");
        }
        let here = format!("{AUTHORIZE_PATH}?{query}");
        let next: String = url::form_urlencoded::byte_serialize(here.as_bytes()).collect();
        return Redirect::to(&format!("/login?next={next}")).into_response();
    };

    let code = secret::token("");
    let now = db::now();
    let stored = sqlx::query(
        "INSERT INTO oidc_codes (code_hash, client_id, user_id, redirect_uri, scope, nonce,
                                 code_challenge, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(secret::token_hash(&code))
    .bind(&client.client_id)
    .bind(viewer.id)
    .bind(redirect_uri)
    .bind(granted.join(" "))
    .bind(nonce)
    .bind(challenge)
    .bind(now)
    .bind(now + CODE_MILLIS)
    .execute(&state.db)
    .await;
    if let Err(error) = stored {
        tracing::error!(%error, "could not keep a sign-in code");
        return fail("server_error", "The hub could not sign you in");
    }
    back_to(
        redirect_uri,
        &[("code", &code), ("state", client_state), ("iss", &iss)],
    )
}

// ── Tokens ────────────────────────────────────────────────────────────

/// An error of the token endpoint, as JSON.
fn token_error(status: StatusCode, error: &str, description: &str) -> Response {
    let mut response = (
        status,
        axum::Json(json!({"error": error, "error_description": description})),
    )
        .into_response();
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if status == StatusCode::UNAUTHORIZED {
        headers.insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Basic realm=\"Stemma\""),
        );
    }
    response
}

fn invalid_client() -> Response {
    token_error(
        StatusCode::UNAUTHORIZED,
        "invalid_client",
        "The client is not known, or its secret is wrong",
    )
}

fn invalid_grant(description: &str) -> Response {
    token_error(StatusCode::BAD_REQUEST, "invalid_grant", description)
}

/// The client's id and secret from basic credentials, each form-encoded
/// as RFC 6749 says.
fn basic_credentials(headers: &HeaderMap) -> Option<Option<(String, String)>> {
    let Some(value) = headers.get(header::AUTHORIZATION) else {
        return Some(None);
    };
    let encoded = value.to_str().ok()?.strip_prefix("Basic ")?;
    let decoded = String::from_utf8(STANDARD.decode(encoded.trim()).ok()?).ok()?;
    let (id, secret) = decoded.split_once(':')?;
    let unescape = |text: &str| {
        url::form_urlencoded::parse(format!("v={text}").as_bytes())
            .next()
            .map(|(_, v)| v.into_owned())
    };
    Some(Some((unescape(id)?, unescape(secret)?)))
}

#[derive(sqlx::FromRow)]
struct CodeRow {
    client_id: String,
    user_id: i64,
    redirect_uri: String,
    scope: String,
    nonce: Option<String>,
    code_challenge: Option<String>,
    expires_at: i64,
}

#[derive(Serialize)]
struct IdClaims {
    iss: String,
    aud: String,
    azp: String,
    exp: i64,
    iat: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
    #[serde(flatten)]
    profile: serde_json::Map<String, Value>,
}

async fn token(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(params) = std::str::from_utf8(&body).ok().and_then(parameters) else {
        return token_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "The request is not a form, or repeats a parameter",
        );
    };
    let param = |name: &str| params.get(name).map(String::as_str);

    // The client, by one method and one only.
    let Some(basic) = basic_credentials(&headers) else {
        return invalid_client();
    };
    let (client_id, client_secret) = match (basic, param("client_secret")) {
        (Some(_), Some(_)) => {
            return token_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "The client authenticates in one way only",
            );
        }
        (Some((id, secret)), None) => {
            if param("client_id").is_some_and(|given| given != id) {
                return invalid_client();
            }
            (id, secret)
        }
        (None, Some(secret)) => match param("client_id") {
            Some(id) => (id.to_owned(), secret.to_owned()),
            None => return invalid_client(),
        },
        (None, None) => return invalid_client(),
    };
    let Some((client, expected)) = client(&state, &client_id) else {
        // A secret is compared all the same, so that the time taken does
        // not tell which clients exist.
        same_secret(&client_secret, "no client");
        return invalid_client();
    };
    if !same_secret(&client_secret, expected) {
        return invalid_client();
    }

    if param("grant_type") != Some("authorization_code") {
        return token_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "Only codes are exchanged here",
        );
    }
    let Some(code) = param("code") else {
        return token_error(StatusCode::BAD_REQUEST, "invalid_request", "No code");
    };
    match exchange(&state, client, code, &params).await {
        Ok(response) => response,
        Err(error) => {
            tracing::error!(%error, "could not exchange a code");
            token_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "The hub could not exchange the code",
            )
        }
    }
}

async fn exchange(
    state: &AppState,
    client: &OidcClientConfig,
    code: &str,
    params: &std::collections::HashMap<String, String>,
) -> anyhow::Result<Response> {
    let param = |name: &str| params.get(name).map(String::as_str);
    let code_hash = secret::token_hash(code);
    let now = db::now();
    // Used now, whatever follows: a code is tried once.
    let row: Option<CodeRow> = sqlx::query_as(
        "UPDATE oidc_codes SET used_at = ? WHERE code_hash = ? AND used_at IS NULL
         RETURNING client_id, user_id, redirect_uri, scope, nonce, code_challenge, expires_at",
    )
    .bind(now)
    .bind(&code_hash)
    .fetch_optional(&state.db)
    .await?;
    let Some(row) = row else {
        // A code used again: what it gave is revoked, for it may be stolen.
        sqlx::query(
            "UPDATE oidc_tokens SET revoked_at = ? WHERE code_hash = ? AND revoked_at IS NULL",
        )
        .bind(now)
        .bind(&code_hash)
        .execute(&state.db)
        .await?;
        return Ok(invalid_grant("The code is not valid, or was used"));
    };
    if row.expires_at <= now {
        return Ok(invalid_grant("The code expired"));
    }
    if row.client_id != client.client_id {
        return Ok(invalid_grant("The code is another client's"));
    }
    if param("redirect_uri") != Some(row.redirect_uri.as_str()) {
        return Ok(invalid_grant("The redirect URI is not the code's"));
    }
    match (&row.code_challenge, param("code_verifier")) {
        (Some(challenge), Some(verifier)) if is_pkce_value(verifier, 43) => {
            let computed = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
            if !bool::from(computed.as_bytes().ct_eq(challenge.as_bytes())) {
                return Ok(invalid_grant("The code verifier does not match"));
            }
        }
        (Some(_), _) => return Ok(invalid_grant("The code verifier is missing or malformed")),
        (None, Some(_)) => return Ok(invalid_grant("The code had no challenge")),
        (None, None) => {}
    }

    let access = secret::token("sto_");
    sqlx::query(
        "INSERT INTO oidc_tokens (token_hash, code_hash, client_id, user_id, scope, created_at,
                                  expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(secret::token_hash(&access))
    .bind(&code_hash)
    .bind(&row.client_id)
    .bind(row.user_id)
    .bind(&row.scope)
    .bind(now)
    .bind(now + TOKEN_SECONDS * 1000)
    .execute(&state.db)
    .await?;

    let seconds = now / 1000;
    let claims = IdClaims {
        iss: issuer(state),
        aud: row.client_id.clone(),
        azp: row.client_id.clone(),
        exp: seconds + ID_TOKEN_SECONDS,
        iat: seconds,
        nonce: row.nonce.clone(),
        profile: profile(state, row.user_id, &row.scope).await?,
    };
    let key = state.signing_key.as_ref().expect("a client needs a key");
    let id_token = key.sign(&claims)?;
    let mut response = axum::Json(json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": TOKEN_SECONDS,
        "id_token": id_token,
        "scope": row.scope,
    }))
    .into_response();
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    Ok(response)
}

/// Its subject, and what the scopes give of an account.
async fn profile(
    state: &AppState,
    user_id: i64,
    scope: &str,
) -> anyhow::Result<serde_json::Map<String, Value>> {
    let (subject, username, display_name): (String, String, String) =
        sqlx::query_as("SELECT subject, username, display_name FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_one(&state.db)
            .await?;
    let scopes: Vec<&str> = scope.split(' ').collect();
    let mut claims = serde_json::Map::new();
    claims.insert("sub".into(), json!(subject));
    if scopes.contains(&"profile") {
        claims.insert("name".into(), json!(display_name));
        claims.insert("preferred_username".into(), json!(username));
        claims.insert("nickname".into(), json!(username));
    }
    if scopes.contains(&"email") {
        // The address of their forge account, which goes nowhere.
        claims.insert("email".into(), json!(crate::forge::email_of(&username)));
        claims.insert("email_verified".into(), json!(false));
    }
    Ok(claims)
}

// ── User info ─────────────────────────────────────────────────────────

async fn userinfo(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let refuse = || {
        let mut response = StatusCode::UNAUTHORIZED.into_response();
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer error=\"invalid_token\""),
        );
        response
    };
    let Some(token) = bearer_token(&headers) else {
        return refuse();
    };
    let found: Result<Option<(i64, String)>, sqlx::Error> = sqlx::query_as(
        "SELECT user_id, scope FROM oidc_tokens
         WHERE token_hash = ? AND revoked_at IS NULL AND expires_at > ?",
    )
    .bind(secret::token_hash(token))
    .bind(db::now())
    .fetch_optional(&state.db)
    .await;
    let (user_id, scope) = match found {
        Ok(Some(found)) => found,
        Ok(None) => return refuse(),
        Err(error) => {
            tracing::error!(%error, "could not look up an access token");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    match profile(&state, user_id, &scope).await {
        Ok(claims) => axum::Json(Value::Object(claims)).into_response(),
        Err(error) => {
            tracing::error!(%error, "could not read an account");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Forgets codes and tokens that expired.
pub async fn forget_expired(db: &sqlx::SqlitePool) -> Result<(), sqlx::Error> {
    let now = db::now();
    sqlx::query("DELETE FROM oidc_codes WHERE expires_at < ?")
        .bind(now)
        .execute(db)
        .await?;
    sqlx::query("DELETE FROM oidc_tokens WHERE expires_at < ?")
        .bind(now)
        .execute(db)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_made_kept_and_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key.pem");
        let made = SigningKey::load_or_create(&path).unwrap();
        let read = SigningKey::load_or_create(&path).unwrap();
        assert_eq!(made.jwk(), read.jwk());
        assert_eq!(made.jwk()["kty"], "EC");
        assert_eq!(made.jwk()["kid"].as_str().unwrap().len(), 43);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "{mode:o}");
        }
        let other = SigningKey::load_or_create(&dir.path().join("other.pem")).unwrap();
        assert_ne!(made.jwk(), other.jwk());
    }

    #[test]
    fn tokens_verify_with_the_published_key() {
        let dir = tempfile::tempdir().unwrap();
        let key = SigningKey::load_or_create(&dir.path().join("key.pem")).unwrap();
        let token = key
            .sign(&json!({"sub": "1", "aud": "forgejo", "exp": 4_000_000_000i64}))
            .unwrap();
        let jwk = key.jwk();
        let decoding = jsonwebtoken::DecodingKey::from_ec_components(
            jwk["x"].as_str().unwrap(),
            jwk["y"].as_str().unwrap(),
        )
        .unwrap();
        let mut validation = jsonwebtoken::Validation::new(Algorithm::ES256);
        validation.set_audience(&["forgejo"]);
        let data = jsonwebtoken::decode::<Value>(&token, &decoding, &validation).unwrap();
        assert_eq!(data.claims["sub"], "1");
        assert_eq!(data.header.kid.as_deref(), jwk["kid"].as_str());
    }

    #[test]
    fn secrets_compare() {
        assert!(same_secret("a long secret", "a long secret"));
        assert!(!same_secret("a long secret", "a long secreT"));
        assert!(!same_secret("", "a long secret"));
    }

    #[test]
    fn repeated_parameters_are_refused() {
        assert!(parameters("a=1&b=2").is_some());
        assert!(parameters("a=1&a=1").is_none());
    }

    #[test]
    fn basic_credentials_are_form_encoded() {
        let mut headers = HeaderMap::new();
        assert_eq!(basic_credentials(&headers), Some(None));
        let encoded = STANDARD.encode("forge%3Ajo:s%2Bcret+x");
        headers.insert(
            header::AUTHORIZATION,
            format!("Basic {encoded}").parse().unwrap(),
        );
        assert_eq!(
            basic_credentials(&headers),
            Some(Some(("forge:jo".into(), "s+cret x".into())))
        );
        headers.insert(header::AUTHORIZATION, "Basic !!".parse().unwrap());
        assert_eq!(basic_credentials(&headers), None);
    }
}
