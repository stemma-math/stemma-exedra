//! Attachments reach agents only through the hub's token policy.
use base64::Engine as _;
use rmcp::model::ContentBlock;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt as _;

use super::{GROUP, ToolDef, ToolError, Toolbox, parse};

const TEXT_LIMIT: u64 = 64 * 1024;
const IMAGE_LIMIT: u64 = 1024 * 1024;

pub fn definition() -> ToolDef {
    ToolDef {
        name: "download_attachment",
        terminal: &["attachment"],
        positional: &["attachment"],
        description: "Read an attachment by its id from read_thread. Small text and images are returned inline; other or large files are saved and their path returned. The person's agents.attachments setting is enforced by the hub.",
        input_schema: json!({"type": "object", "properties": {
            "group": {"type": "string", "description": GROUP},
            "attachment": {"type": "integer", "minimum": 1, "description": "The attachment id listed by the reading tools."}
        }, "required": ["attachment"]}),
        writes: false,
    }
}
#[derive(Deserialize)]
struct Args {
    group: Option<String>,
    attachment: i64,
}
fn io(error: impl std::fmt::Display) -> ToolError {
    ToolError::invalid(format!("Could not save the attachment: {error}"))
}

pub async fn download(
    toolbox: &Toolbox,
    args: Value,
    inline: bool,
) -> Result<Vec<ContentBlock>, ToolError> {
    let args: Args = parse(args)?;
    if args.attachment <= 0 {
        return Err(ToolError::invalid(
            "Use the attachment id listed by read_thread",
        ));
    }
    let group = toolbox.group(args.group.as_deref()).await?;
    if group.is_empty()
        || !group
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(ToolError::invalid("Invalid group short name"));
    }
    let mut response = toolbox
        .hub
        .download_attachment(&group, args.attachment)
        .await?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .split(';')
        .next()
        .unwrap()
        .trim()
        .to_owned();
    let name = filename(
        response
            .headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
    );
    let directory = toolbox
        .attachment_dir
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join("stemma-attachments"));
    tokio::fs::create_dir_all(&directory).await.map_err(io)?;
    let temporary = tempfile::Builder::new()
        .prefix(&format!("{}-", args.attachment))
        .suffix(&format!("-{name}"))
        .tempfile_in(&directory)
        .map_err(io)?;
    let mut file = tokio::fs::File::from_std(temporary.reopen().map_err(io)?);
    let mut size = 0;
    while let Some(chunk) = response.chunk().await.map_err(io)? {
        size += chunk.len() as u64;
        file.write_all(&chunk).await.map_err(io)?;
    }
    file.flush().await.map_err(io)?;
    drop(file);
    let text = content_type.starts_with("text/")
        || matches!(
            content_type.as_str(),
            "application/json"
                | "application/xml"
                | "application/x-tex"
                | "application/toml"
                | "application/yaml"
        )
        || content_type.ends_with("+json")
        || content_type.ends_with("+xml");
    let image = matches!(
        content_type.as_str(),
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    );
    if inline && text && size <= TEXT_LIMIT {
        if let Ok(text) = tokio::fs::read_to_string(temporary.path()).await {
            return Ok(vec![ContentBlock::text(format!(
                "{name} ({content_type}):\n\n{text}"
            ))]);
        }
    } else if inline && image && size <= IMAGE_LIMIT {
        let bytes = tokio::fs::read(temporary.path()).await.map_err(io)?;
        return Ok(vec![ContentBlock::image(
            base64::engine::general_purpose::STANDARD.encode(bytes),
            content_type,
        )]);
    }
    let (_, path) = temporary.keep().map_err(io)?;
    Ok(vec![ContentBlock::text(format!(
        "Saved {name} ({content_type}, {size} bytes) to {}",
        path.display()
    ))])
}

/// The hub sends RFC 5987 names. Decode, then retain only a filename,
/// even if a different server sends a path or control characters.
fn filename(disposition: &str) -> String {
    let encoded = disposition
        .split("filename*=UTF-8''")
        .nth(1)
        .unwrap_or("file");
    let mut decoded = Vec::new();
    let bytes = encoded.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(value) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|s| u8::from_str_radix(s, 16).ok())
        {
            decoded.push(value);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    let text = String::from_utf8_lossy(&decoded);
    let mut name: String = text
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("file")
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect();
    while name.len() > 120 {
        name.pop();
    }
    if matches!(name.as_str(), "" | "." | "..") {
        "file".into()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_cannot_escape_the_download_directory() {
        assert_eq!(
            filename("inline; filename*=UTF-8''..%2Fnotes%20%CE%B1.tex"),
            "notes α.tex"
        );
        assert_eq!(filename("attachment; filename*=UTF-8''.."), "file");
        assert_eq!(filename("attachment; filename*=UTF-8''x%00.txt"), "x.txt");
    }
}
