//! The workshop's provider boundary. Credentials are decrypted only for HTTP calls.
use std::{path::Path, time::Duration};

use axum::extract::{Path as Route, State};
use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    api::{Json, Membership},
    auth::Viewer,
    db,
    error::{ApiError, ApiResult},
    state::AppState,
};

pub struct Key(LessSafeKey);
impl Key {
    pub fn load_or_create(path: &Path) -> anyhow::Result<Self> {
        use std::io::Write;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(path) {
            Ok(mut file) => {
                let bytes: [u8; 32] = rand::random();
                file.write_all(&bytes)?;
                file.sync_all()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let bytes = std::fs::read(path)?;
        let key = UnboundKey::new(&aead::AES_256_GCM, &bytes)
            .map_err(|_| anyhow::anyhow!("Invalid workshop encryption key"))?;
        Ok(Self(LessSafeKey::new(key)))
    }
    pub fn encrypt(&self, group: i64, text: &str) -> ApiResult<Vec<u8>> {
        let nonce: [u8; 12] = rand::random();
        let mut bytes = text.as_bytes().to_vec();
        self.0
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(group.to_be_bytes()),
                &mut bytes,
            )
            .map_err(|_| ApiError::internal("Could not encrypt the workshop key"))?;
        Ok([nonce.as_slice(), &bytes].concat())
    }
    fn decrypt(&self, group: i64, bytes: &[u8]) -> ApiResult<String> {
        let failed = || ApiError::internal("Could not decrypt the workshop key");
        if bytes.len() < 12 {
            return Err(failed());
        }
        let nonce: [u8; 12] = bytes[..12].try_into().map_err(|_| failed())?;
        let mut sealed = bytes[12..].to_vec();
        let plain = self
            .0
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(group.to_be_bytes()),
                &mut sealed,
            )
            .map_err(|_| failed())?;
        String::from_utf8(plain.to_vec()).map_err(|_| failed())
    }
}

#[derive(Serialize)]
pub struct Settings {
    provider: String,
    model: String,
    has_key: bool,
    last_four: Option<String>,
    set_by: Option<stemma_core::api::User>,
    set_at: Option<i64>,
    monthly_cap: i64,
    step_cap: i64,
    month: String,
    input_tokens: i64,
    output_tokens: i64,
    cost_usd: Option<f64>,
    stopped: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    provider: String,
    model: String,
    /// None retains the key, empty removes it. Never serialized or debugged.
    api_key: Option<String>,
    monthly_cap: i64,
    step_cap: i64,
}
#[derive(sqlx::FromRow)]
struct Stored {
    provider: String,
    model: String,
    encrypted_key: Option<Vec<u8>>,
    last_four: Option<String>,
    set_by: Option<i64>,
    set_at: Option<i64>,
    monthly_cap: i64,
    step_cap: i64,
}
async fn stored(state: &AppState, group: i64) -> ApiResult<Stored> {
    sqlx::query(
        "INSERT INTO workshop_models (group_id) VALUES (?)
         ON CONFLICT DO NOTHING",
    )
    .bind(group)
    .execute(&state.db)
    .await?;
    Ok(sqlx::query_as(
        "SELECT provider, model, encrypted_key, last_four, set_by, set_at,
         monthly_cap, step_cap
         FROM workshop_models
         WHERE group_id = ?",
    )
    .bind(group)
    .fetch_one(&state.db)
    .await?)
}
pub fn month() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!("{:04}-{:02}", now.year(), u8::from(now.month()))
}
pub async fn settings(state: &AppState, group: i64) -> ApiResult<Settings> {
    let row = stored(state, group).await?;
    let month = month();
    let (input_tokens, output_tokens, cost_usd) = sqlx::query_as::<_, (i64, i64, Option<f64>)>(
        "SELECT input_tokens, output_tokens, cost_usd
         FROM workshop_usage
         WHERE group_id = ? AND month = ?",
    )
    .bind(group)
    .bind(&month)
    .fetch_optional(&state.db)
    .await?
    .unwrap_or_default();
    let set_by = match row.set_by {
        Some(id) => Some(crate::workshop::user(state, id).await?),
        None => None,
    };
    Ok(Settings {
        provider: row.provider,
        model: row.model,
        has_key: row.encrypted_key.is_some(),
        last_four: row.last_four,
        set_by,
        set_at: row.set_at,
        monthly_cap: row.monthly_cap,
        step_cap: row.step_cap,
        month,
        input_tokens,
        output_tokens,
        cost_usd,
        stopped: input_tokens.saturating_add(output_tokens) >= row.monthly_cap,
    })
}
impl Settings {
    /// What the board shows every member: the model and the month's use.
    pub fn public(self) -> Self {
        Self {
            last_four: None,
            set_by: None,
            set_at: None,
            ..self
        }
    }
}
pub async fn get(
    State(state): State<AppState>,
    viewer: Viewer,
    Route(slug): Route<String>,
) -> ApiResult<Json<Settings>> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    group.require_admin()?;
    Ok(Json(settings(&state, group.group_id).await?))
}
pub async fn update(
    State(state): State<AppState>,
    viewer: Viewer,
    Route(slug): Route<String>,
    Json(body): Json<Update>,
) -> ApiResult<Json<Settings>> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    group.require_admin()?;
    if !["anthropic", "openai", "openrouter"].contains(&body.provider.as_str()) {
        return Err(ApiError::invalid("Choose Anthropic, OpenAI or OpenRouter"));
    }
    if !(1..=100).contains(&body.step_cap) || body.monthly_cap < 0 {
        return Err(ApiError::invalid(
            "Use 1 to 100 steps and a nonnegative token cap",
        ));
    }
    if body.model.len() > 200 || body.model.trim().is_empty() {
        return Err(ApiError::invalid("Name a model, up to 200 characters"));
    }
    let old = stored(&state, group.group_id).await?;
    if (old.provider != body.provider || old.model != body.model)
        && body.api_key.is_none()
        && old.encrypted_key.is_some()
    {
        return Err(ApiError::invalid(
            "Provide a new API key when changing provider or model",
        ));
    }
    let (encrypted, last_four, set_by, set_at) = match body.api_key {
        None => (old.encrypted_key, old.last_four, old.set_by, old.set_at),
        Some(key) if key.is_empty() => (None, None, None, None),
        Some(key) => {
            if key.len() < 8 || key.len() > 4096 || key.chars().any(char::is_whitespace) {
                return Err(ApiError::invalid("Provide an API key, without whitespace"));
            }
            if key.starts_with("sk-ant-oat") {
                return Err(ApiError::invalid(
                    "Use an API key, not a subscription sign-in",
                ));
            }
            (
                Some(state.model_key.encrypt(group.group_id, &key)?),
                Some(
                    key.chars()
                        .rev()
                        .take(4)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect(),
                ),
                Some(viewer.id),
                Some(db::now()),
            )
        }
    };
    sqlx::query(
        "UPDATE workshop_models
         SET provider = ?, model = ?, encrypted_key = ?, last_four = ?,
         set_by = ?, set_at = ?, monthly_cap = ?, step_cap = ?
         WHERE group_id = ?",
    )
    .bind(body.provider)
    .bind(body.model.trim())
    .bind(encrypted)
    .bind(last_four)
    .bind(set_by)
    .bind(set_at)
    .bind(body.monthly_cap)
    .bind(body.step_cap)
    .bind(group.group_id)
    .execute(&state.db)
    .await?;
    crate::orchestrator::wake_group(&state, group.group_id).await?;
    Ok(Json(settings(&state, group.group_id).await?))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Call {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub text: String,
    #[serde(default)]
    pub calls: Vec<Call>,
    pub call_id: Option<String>,
}
impl Message {
    pub fn text(role: &str, text: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            text: text.into(),
            calls: vec![],
            call_id: None,
        }
    }
    pub fn result(call: &Call, text: String) -> Self {
        Self {
            role: "tool".into(),
            text,
            calls: vec![],
            call_id: Some(call.id.clone()),
        }
    }
}
pub struct Reply {
    pub message: Message,
    pub model: String,
}

/// Configurable only by the embedding hub, never from a group's API.
/// Tests substitute a loopback fake. Production endpoints are fixed.
pub struct Client {
    http: reqwest::Client,
    pub endpoint: Option<String>,
}
impl Client {
    pub fn new() -> anyhow::Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(90))
                .connect_timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            endpoint: None,
        })
    }
    async fn send(
        &self,
        provider: &str,
        model: &str,
        key: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> ApiResult<Value> {
        let anthropic = provider == "anthropic";
        let default = match provider {
            "anthropic" => "https://api.anthropic.com/v1/messages",
            "openai" => "https://api.openai.com/v1/chat/completions",
            _ => "https://openrouter.ai/api/v1/chat/completions",
        };
        let body = payload(anthropic, model, messages, tools);
        for attempt in 0..4 {
            let mut request = self
                .http
                .post(self.endpoint.as_deref().unwrap_or(default))
                .json(&body);
            request = if anthropic {
                request
                    .header("x-api-key", key)
                    .header("anthropic-version", "2023-06-01")
            } else {
                request.bearer_auth(key)
            };
            let response = request.send().await.map_err(|_| {
                ApiError::invalid(format!("{provider} could not be reached or timed out"))
            })?;
            let status = response.status();
            if (status.as_u16() == 429 || status.is_server_error()) && attempt < 3 {
                tokio::time::sleep(Duration::from_millis(250 * (1 << attempt))).await;
                continue;
            }
            let body = response.text().await.map_err(|_| {
                ApiError::invalid(format!(
                    "{provider} returned an unreadable response ({status})"
                ))
            })?;
            let value = serde_json::from_str::<Value>(&body);
            if !status.is_success() {
                let reason = value
                    .as_ref()
                    .ok()
                    .and_then(|v| v["error"]["message"].as_str())
                    .unwrap_or_else(|| body.trim());
                let safe = stemma_core::secrets::hide(&reason.replace(key, "[hidden]"));
                return Err(ApiError::invalid(format!(
                    "{provider} refused ({status}): {}",
                    safe.chars().take(500).collect::<String>()
                )));
            }
            let value = value.map_err(|_| {
                ApiError::invalid(format!(
                    "{provider} returned an unreadable response ({status})"
                ))
            })?;
            return Ok(value);
        }
        unreachable!()
    }
}
fn payload(anthropic: bool, model: &str, messages: &[Message], tools: &[Tool]) -> Value {
    if !anthropic {
        let messages: Vec<Value> = messages.iter().map(chat_message).collect();
        let tools: Vec<Value> = tools
            .iter()
            .map(|tool| json!({"type": "function", "function": tool}))
            .collect();
        return json!({"model": model, "messages": messages, "tools": tools});
    }
    let system = messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let messages: Vec<Value> = messages
        .iter()
        .filter(|m| m.role != "system")
        .map(anthropic_message)
        .collect();
    let tools: Vec<Value> = tools
        .iter()
        .map(|tool| {
            json!({"name": tool.name, "description": tool.description,
               "input_schema": tool.parameters})
        })
        .collect();
    json!({"model": model, "max_tokens": 4096, "system": system,
           "messages": messages, "tools": tools})
}
fn chat_message(message: &Message) -> Value {
    let mut out = json!({"role": message.role, "content": message.text});
    if !message.calls.is_empty() {
        let calls: Vec<Value> = message
            .calls
            .iter()
            .map(|call| {
                json!({"id": call.id, "type": "function", "function": {
                    "name": call.name, "arguments": call.arguments.to_string()
                }})
            })
            .collect();
        out["tool_calls"] = json!(calls);
    }
    if let Some(id) = &message.call_id {
        out["tool_call_id"] = json!(id);
    }
    out
}
fn anthropic_message(message: &Message) -> Value {
    if let Some(id) = &message.call_id {
        return json!({"role": "user", "content": [{
            "type": "tool_result", "tool_use_id": id, "content": message.text
        }]});
    }
    let mut content = vec![];
    if !message.text.is_empty() {
        content.push(json!({"type": "text", "text": message.text}));
    }
    content.extend(message.calls.iter().map(|call| {
        json!({"type": "tool_use", "id": call.id, "name": call.name,
               "input": call.arguments})
    }));
    json!({"role": message.role, "content": content})
}

pub async fn call(
    state: &AppState,
    group: i64,
    messages: &[Message],
    tools: &[Tool],
) -> ApiResult<Reply> {
    let config = stored(state, group).await?;
    let key = config.encrypted_key.as_ref().ok_or_else(|| {
        ApiError::new(
            axum::http::StatusCode::CONFLICT,
            "workshop_key",
            "A group model API key is needed",
        )
    })?;
    if settings(state, group).await?.stopped {
        return Err(ApiError::new(
            axum::http::StatusCode::CONFLICT,
            "workshop_cap",
            "The workshop reached its monthly token cap; waiting for next month or a higher cap",
        ));
    }
    let key = state.model_key.decrypt(group, key)?;
    let value = state
        .model_client
        .send(&config.provider, &config.model, &key, messages, tools)
        .await?;
    let usage = &value["usage"];
    let input = usage["input_tokens"]
        .as_i64()
        .or_else(|| usage["prompt_tokens"].as_i64())
        .unwrap_or(0)
        .max(0)
        + usage["cache_read_input_tokens"]
            .as_i64()
            .unwrap_or(0)
            .max(0)
        + usage["cache_creation_input_tokens"]
            .as_i64()
            .unwrap_or(0)
            .max(0);
    let output = usage["output_tokens"]
        .as_i64()
        .or_else(|| usage["completion_tokens"].as_i64())
        .unwrap_or(0)
        .max(0);
    sqlx::query(
        "INSERT INTO workshop_usage (group_id, month, input_tokens, output_tokens, cost_usd)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (group_id, month) DO UPDATE
         SET input_tokens = input_tokens + excluded.input_tokens,
         output_tokens = output_tokens + excluded.output_tokens,
         cost_usd = CASE WHEN excluded.cost_usd IS NULL THEN cost_usd
         ELSE coalesce(cost_usd, 0) + excluded.cost_usd END",
    )
    .bind(group)
    .bind(month())
    .bind(input)
    .bind(output)
    .bind(usage["cost"].as_f64())
    .execute(&state.db)
    .await?;
    let mut message = Message::text("assistant", "");
    if config.provider == "anthropic" {
        for block in value["content"].as_array().into_iter().flatten() {
            if block["type"] == "text" {
                message.text.push_str(block["text"].as_str().unwrap_or(""));
            }
            if block["type"] == "tool_use" {
                message.calls.push(Call {
                    id: block["id"].as_str().unwrap_or("").into(),
                    name: block["name"].as_str().unwrap_or("").into(),
                    arguments: block["input"].clone(),
                });
            }
        }
    } else {
        let m = &value["choices"][0]["message"];
        message.text = m["content"].as_str().unwrap_or("").into();
        for c in m["tool_calls"].as_array().into_iter().flatten() {
            message.calls.push(Call {
                id: c["id"].as_str().unwrap_or("").into(),
                name: c["function"]["name"].as_str().unwrap_or("").into(),
                arguments: serde_json::from_str(
                    c["function"]["arguments"].as_str().unwrap_or("{}"),
                )
                .unwrap_or(Value::Null),
            });
        }
    }
    // Scrub strings before serialization, including unusual keys containing JSON
    // punctuation, rather than replacing bytes in escaped JSON.
    message.text = stemma_core::secrets::hide(&message.text.replace(&key, "[hidden]"));
    for call in &mut message.calls {
        hide_value(&mut call.arguments, &key);
    }
    Ok(Reply {
        message,
        model: config.model,
    })
}

pub async fn has_key(state: &AppState, group: i64) -> ApiResult<bool> {
    Ok(stored(state, group).await?.encrypted_key.is_some())
}
pub async fn step_cap(state: &AppState, group: i64) -> ApiResult<i64> {
    Ok(stored(state, group).await?.step_cap)
}

fn hide_value(value: &mut Value, key: &str) {
    match value {
        Value::String(s) => *s = stemma_core::secrets::hide(&s.replace(key, "[hidden]")),
        Value::Array(values) => {
            for value in values {
                hide_value(value, key);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                hide_value(value, key);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::Key;
    #[test]
    fn encrypted_keys_are_authenticated_bound_to_the_group_and_persistent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workshop.key");
        let key = Key::load_or_create(&path).unwrap();
        let sealed = key.encrypt(7, "a dedicated test key").unwrap();
        let reopened = Key::load_or_create(&path).unwrap();
        assert_eq!(
            reopened.decrypt(7, &sealed).unwrap(),
            "a dedicated test key"
        );
        assert!(reopened.decrypt(8, &sealed).is_err());
        let mut tampered = sealed;
        tampered[12] ^= 1;
        assert!(reopened.decrypt(7, &tampered).is_err());
    }
}
