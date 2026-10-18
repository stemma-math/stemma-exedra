//! Forgejo's webhooks: a push to the `main` of a group's library tells the
//! hub to look at where `main` is, and build its new commit.
//!
//! Each delivery is signed with the shared secret (an HMAC-SHA256 of its
//! body); one that is not is refused. What it says is only taken as a
//! hint: the hub asks Forgejo where `main` is, so that a delivery played
//! again builds nothing old.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use sha2::Sha256;

use super::{LIBRARY, MAIN};
use crate::library;
use crate::state::AppState;

/// Whether `signature`, in hex, is the HMAC-SHA256 of `body` with `secret`.
/// Compared in constant time.
pub fn verify(secret: &str, body: &[u8], signature: &str) -> bool {
    let Some(signature) = decode_hex(signature.trim()) else {
        return false;
    };
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(body);
    mac.verify_slice(&signature).is_ok()
}

/// The signature of `body` with `secret`, as Forgejo sends it.
pub fn sign(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(body);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

fn header<'a>(headers: &'a HeaderMap, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| headers.get(*name).and_then(|v| v.to_str().ok()))
}

#[derive(Deserialize)]
struct Push {
    #[serde(rename = "ref")]
    reference: String,
    repository: Repository,
    pusher: Option<Account>,
}

#[derive(Deserialize)]
struct Repository {
    name: String,
    owner: Account,
}

#[derive(Deserialize)]
struct Account {
    login: String,
}

pub async fn receive(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let Some(forge) = state.config.forge.as_ref() else {
        return StatusCode::NOT_FOUND;
    };
    let signature = header(&headers, &["x-forgejo-signature", "x-gitea-signature"]);
    if !signature.is_some_and(|s| verify(&forge.secrets.webhook_secret, &body, s)) {
        tracing::warn!("refused a webhook that was not signed with the shared secret");
        return StatusCode::UNAUTHORIZED;
    }
    if header(&headers, &["x-forgejo-event", "x-gitea-event"]) != Some("push") {
        return StatusCode::NO_CONTENT;
    }
    let Ok(push) = serde_json::from_slice::<Push>(&body) else {
        return StatusCode::BAD_REQUEST;
    };
    if push.reference != format!("refs/heads/{MAIN}") || push.repository.name != LIBRARY {
        return StatusCode::NO_CONTENT;
    }
    match pushed(&state, &push).await {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(error) => {
            tracing::error!(%error, "could not look at a push");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

/// Looks at the library's `main`, for whoever pushed if they are in the
/// group, or else for its first admin.
async fn pushed(state: &AppState, push: &Push) -> anyhow::Result<()> {
    let Some(group_id): Option<i64> = sqlx::query_scalar("SELECT id FROM groups WHERE slug = ?")
        .bind(&push.repository.owner.login)
        .fetch_optional(&state.db)
        .await?
    else {
        return Ok(());
    };
    let pusher: Option<i64> = match &push.pusher {
        Some(pusher) => {
            sqlx::query_scalar(
                "SELECT u.id FROM users u JOIN memberships m ON m.user_id = u.id
                 WHERE u.forge_login = ? AND m.group_id = ?",
            )
            .bind(&pusher.login)
            .bind(group_id)
            .fetch_optional(&state.db)
            .await?
        }
        None => None,
    };
    let requester = match pusher {
        Some(id) => Some(id),
        None => {
            sqlx::query_scalar(
                "SELECT user_id FROM memberships WHERE group_id = ?
                 ORDER BY role = 'admin' DESC, joined_at LIMIT 1",
            )
            .bind(group_id)
            .fetch_optional(&state.db)
            .await?
        }
    };
    let Some(requester) = requester else {
        return Ok(());
    };
    library::refresh(state, group_id, requester)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_verify() {
        let body = br#"{"ref":"refs/heads/main"}"#;
        let signature = sign("a shared secret", body);
        assert_eq!(signature.len(), 64);
        assert!(verify("a shared secret", body, &signature));
        assert!(verify("a shared secret", body, &signature.to_uppercase()));
        assert!(!verify("another secret", body, &signature));
        assert!(!verify("a shared secret", b"{}", &signature));
        assert!(!verify("a shared secret", body, ""));
        assert!(!verify("a shared secret", body, "not hex"));
        assert!(!verify("a shared secret", body, &signature[..62]));
    }
}
