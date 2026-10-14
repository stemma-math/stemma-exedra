//! Attachments: files of any kind, uploaded first and then carried by a
//! message. They live in the data directory, under `attachments/`.

use std::collections::HashMap;

use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use stemma_core::api::Attachment;
use tokio::io::AsyncWriteExt;

use super::{Json, Membership};
use crate::auth::Viewer;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::secret;
use crate::state::AppState;

/// Uploads one file, in the multipart field `file`. It waits for a message
/// to carry it.
pub async fn upload(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    mut multipart: Multipart,
) -> ApiResult<(StatusCode, Json<Attachment>)> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let limit = state.config.max_upload_mb * 1024 * 1024;
    let bad = |e: axum::extract::multipart::MultipartError| {
        ApiError::new(e.status(), "bad_request", e.body_text())
    };

    while let Some(mut field) = multipart.next_field().await.map_err(bad)? {
        if field.name() != Some("file") {
            continue;
        }
        let name = clean_name(field.file_name().unwrap_or("file"));
        let content_type = field
            .content_type()
            .filter(|t| !t.is_empty() && *t != "application/octet-stream")
            .map(str::to_owned)
            .unwrap_or_else(|| {
                mime_guess::from_path(&name)
                    .first_or_octet_stream()
                    .to_string()
            });

        let dir = state.config.attachments_dir();
        tokio::fs::create_dir_all(&dir).await?;
        let key = secret::token("");
        let path = dir.join(&key);
        let mut file = tokio::fs::File::create(&path).await?;
        let mut size: u64 = 0;
        let written: ApiResult<()> = async {
            while let Some(chunk) = field.chunk().await.map_err(bad)? {
                size += chunk.len() as u64;
                if size > limit {
                    return Err(ApiError::new(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "too_large",
                        format!("A file is at most {} MB", state.config.max_upload_mb),
                    ));
                }
                file.write_all(&chunk).await?;
            }
            file.flush().await?;
            Ok(())
        }
        .await;
        if let Err(e) = written {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(e);
        }

        let id = sqlx::query(
            "INSERT INTO attachments (group_id, uploader_id, name, content_type, size, storage_key, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(group.group_id)
        .bind(viewer.id)
        .bind(&name)
        .bind(&content_type)
        .bind(size as i64)
        .bind(&key)
        .bind(db::now())
        .execute(&state.db)
        .await?
        .last_insert_rowid();
        let attachment = Attachment {
            id,
            url: url(&group.slug, id, &name),
            name,
            content_type,
            size,
        };
        return Ok((StatusCode::CREATED, Json(attachment)));
    }
    Err(ApiError::invalid(
        "Send the file in a multipart field named \"file\"",
    ))
}

/// Downloads a file. Images and PDFs are shown in the browser; anything
/// else is downloaded, and nothing runs scripts on the hub's origin.
pub async fn download(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, id, _name)): Path<(String, i64, String)>,
) -> ApiResult<Response> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let row: Option<(String, String, i64, String)> = sqlx::query_as(
        "SELECT name, content_type, size, storage_key FROM attachments
         WHERE id = ? AND group_id = ? AND message_id IS NOT NULL",
    )
    .bind(id)
    .bind(group.group_id)
    .fetch_optional(&state.db)
    .await?;
    let (name, content_type, size, key) = row.ok_or_else(|| ApiError::not_found("File"))?;
    let file = tokio::fs::File::open(state.config.attachments_dir().join(key))
        .await
        .map_err(|_| ApiError::not_found("File"))?;

    let inline = shown_inline(&content_type);
    let disposition = format!(
        "{}; filename*=UTF-8''{}",
        if inline { "inline" } else { "attachment" },
        percent_encode(&name)
    );
    let mut response = Body::from_stream(tokio_util::io::ReaderStream::new(file)).into_response();
    let headers = response.headers_mut();
    let value = |s: &str| HeaderValue::from_str(s).unwrap_or(HeaderValue::from_static(""));
    headers.insert(header::CONTENT_TYPE, value(&content_type));
    headers.insert(header::CONTENT_LENGTH, size.into());
    headers.insert(header::CONTENT_DISPOSITION, value(&disposition));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=86400"),
    );
    // Browsers render PDFs in a viewer of their own, which a sandbox breaks.
    if content_type != "application/pdf" {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "sandbox; default-src 'none'; img-src 'self'; style-src 'unsafe-inline'",
            ),
        );
    }
    Ok(response)
}

fn shown_inline(content_type: &str) -> bool {
    content_type.starts_with("image/") || content_type == "application/pdf"
}

/// The files of a thread's messages from `from` to `to`, by message id.
pub(super) async fn attachments_of(
    state: &AppState,
    group: &Membership,
    thread_id: i64,
    from: u32,
    to: u32,
) -> ApiResult<HashMap<i64, Vec<Attachment>>> {
    let rows: Vec<(i64, i64, String, String, i64)> = sqlx::query_as(
        "SELECT a.message_id, a.id, a.name, a.content_type, a.size
         FROM attachments a JOIN messages m ON m.id = a.message_id
         WHERE m.thread_id = ? AND m.number BETWEEN ? AND ?
         ORDER BY a.id",
    )
    .bind(thread_id)
    .bind(from)
    .bind(to)
    .fetch_all(&state.db)
    .await?;
    let mut by_message: HashMap<i64, Vec<Attachment>> = HashMap::new();
    for (message_id, id, name, content_type, size) in rows {
        by_message.entry(message_id).or_default().push(Attachment {
            id,
            url: url(&group.slug, id, &name),
            name,
            content_type,
            size: size as u64,
        });
    }
    Ok(by_message)
}

/// Removes stored files; a file already gone is fine.
pub(super) async fn remove_files(state: &AppState, keys: &[String]) {
    let dir = state.config.attachments_dir();
    for key in keys {
        if let Err(error) = tokio::fs::remove_file(dir.join(key)).await
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%error, key, "could not remove an attachment");
        }
    }
}

fn url(group: &str, id: i64, name: &str) -> String {
    format!(
        "{}/groups/{group}/attachments/{id}/{}",
        stemma_core::api::PREFIX,
        percent_encode(name)
    )
}

/// A file name as the hub keeps it: no directories, brackets or control
/// characters, and not too long.
fn clean_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| if matches!(c, '[' | ']') { '_' } else { c })
        .take(200)
        .collect();
    match cleaned.trim() {
        "" | "." | ".." => "file".to_owned(),
        trimmed => trimmed.to_owned(),
    }
}

/// Percent-encodes everything but unreserved characters (RFC 3986).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_names() {
        assert_eq!(clean_name("../../etc/passwd"), "passwd");
        assert_eq!(clean_name("C:\\photos\\board [1].jpg"), "board _1_.jpg");
        assert_eq!(clean_name(".."), "file");
        assert_eq!(clean_name(""), "file");
        assert_eq!(clean_name("a\u{0}b.tex"), "ab.tex");
    }

    #[test]
    fn encodes_names() {
        assert_eq!(percent_encode("the board.jpg"), "the%20board.jpg");
        assert_eq!(percent_encode("ñ"), "%C3%B1");
    }
}
