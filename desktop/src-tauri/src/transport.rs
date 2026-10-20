//! The credential boundary: only Rust knows the device token and hub address.
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, bail};
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use reqwest::{Client, Method};
use serde::Serialize;
use stemma_client::config::ClientConfig;
use tauri::http::{Request, Response, header};
use tauri::{Emitter, Manager, WebviewWindow};

pub type Streams = Mutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>;

pub fn client() -> anyhow::Result<Client> {
    stemma_client::hub::install_crypto();
    // Never forward credentials across redirects. No cookie store is enabled.
    Ok(Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .build()?)
}

fn credentials() -> anyhow::Result<(String, String)> {
    let config = ClientConfig::load()?.with_env();
    let base = config.hub()?.base().to_owned();
    Ok((base, config.token.context("Sign in to your hub")?))
}

fn allowed(request: &Request<Vec<u8>>) -> bool {
    let uri = request.uri();
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    let from_app =
        origin.is_some_and(|o| o.parse().is_ok_and(|url| super::trusted_machine_url(&url)));
    let path = uri.path();
    let resource = path.contains("/attachments/") || path.ends_with("/pdf");
    uri.scheme_str() == Some("hub") && uri.authority().is_some_and(|a| a.as_str() == "localhost")
        && path.starts_with("/api/v1/")
        && !path.contains('%') && !path.contains("..") && !path.contains('\\')
        // Untrusted documents may display resources, but cannot read the API or mutate it.
        && (from_app || (resource && matches!(*request.method(), Method::GET | Method::HEAD)))
}

fn response(status: u16, body: Vec<u8>, origin: Option<&str>) -> Response<Vec<u8>> {
    let mut builder = Response::builder()
        .status(status)
        .header(header::CACHE_CONTROL, "no-store")
        .header("X-Content-Type-Options", "nosniff")
        // Also applies to HTML/SVG opened as a document. It has no script,
        // forms, same-origin privileges, or native capabilities.
        .header(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; style-src 'unsafe-inline'; img-src data:; sandbox",
        )
        .header(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            "GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS",
        )
        .header(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            "Content-Type, Accept, Range",
        )
        .header(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            "Content-Type, Content-Length, Content-Range, Content-Disposition",
        );
    if let Some(origin) = origin {
        builder = builder.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    }
    builder.body(body).expect("valid proxy response")
}

pub async fn proxy(request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .filter(|o| o.parse().is_ok_and(|url| super::trusted_machine_url(&url)))
        .map(str::to_owned);
    if !allowed(&request) {
        return response(403, b"Hub access denied".to_vec(), None);
    }
    if request.method() == Method::OPTIONS {
        return response(204, Vec::new(), origin.as_deref());
    }
    match forward(request).await {
        Ok((status, headers, body)) => {
            let mut reply = response(status, body, origin.as_deref());
            // In particular: never Set-Cookie, Location, CSP, or hub CORS headers.
            for key in [
                header::CONTENT_TYPE,
                header::CONTENT_DISPOSITION,
                header::CONTENT_RANGE,
                header::ACCEPT_RANGES,
            ] {
                if let Some(value) = headers.get(&key) {
                    reply.headers_mut().insert(key, value.clone());
                }
            }
            reply
        }
        Err(_) => response(
            502,
            br#"{"error":{"code":"offline","message":"Could not reach the hub"}}"#.to_vec(),
            origin.as_deref(),
        ),
    }
}

async fn forward(
    request: Request<Vec<u8>>,
) -> anyhow::Result<(u16, reqwest::header::HeaderMap, Vec<u8>)> {
    let (base, token) = credentials()?;
    let path = request.uri().path_and_query().context("Missing path")?;
    let mut outgoing = client()?
        .request(request.method().clone(), format!("{base}{path}"))
        .bearer_auth(token)
        .timeout(Duration::from_secs(120));
    for key in [header::CONTENT_TYPE, header::ACCEPT, header::RANGE] {
        if let Some(value) = request.headers().get(&key) {
            outgoing = outgoing.header(key, value);
        }
    }
    let reply = outgoing.body(request.into_body()).send().await?;
    if reply.status().is_redirection() {
        bail!("The hub redirected a credentialed request");
    }
    let status = reply.status().as_u16();
    let headers = reply.headers().clone();
    Ok((status, headers, reply.bytes().await?.to_vec()))
}

#[derive(Clone, Serialize)]
struct LiveEvent {
    group: String,
    event: serde_json::Value,
}

fn send(window: &WebviewWindow, group: &str, event: serde_json::Value) {
    let _ = window.emit(
        "hub-event",
        LiveEvent {
            group: group.into(),
            event,
        },
    );
}

pub fn stop(app: &tauri::AppHandle) {
    for (_, task) in app.state::<Streams>().lock().unwrap().drain() {
        task.abort();
    }
}

#[tauri::command]
pub async fn subscribe(window: WebviewWindow, group: String) -> super::Result<()> {
    super::local(&window)?;
    if group.is_empty()
        || !group
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err("Invalid group".into());
    }
    let (base, token) = credentials().map_err(super::error)?;
    let client = client().map_err(super::error)?;
    let app = window.app_handle().clone();
    let mut streams = app.state::<Streams>().inner().lock().unwrap();
    if streams.contains_key(&group) {
        return Ok(());
    }
    let key = group.clone();
    let task = tauri::async_runtime::spawn(async move {
        let mut connected = false;
        loop {
            let result = client
                .get(format!("{base}/api/v1/groups/{group}/events"))
                .bearer_auth(&token)
                .header(header::ACCEPT, "text/event-stream")
                .send()
                .await;
            if let Ok(reply) = result
                && reply.status().is_success()
            {
                let mut events = reply.bytes_stream().eventsource();
                while let Some(Ok(event)) = events.next().await {
                    match event.event.as_str() {
                        "ready" => {
                            if connected {
                                send(&window, &group, serde_json::json!({"type":"resync"}));
                            }
                            connected = true;
                            send(
                                &window,
                                &group,
                                serde_json::json!({"type":"connection", "connected":true}),
                            );
                        }
                        "resync" => send(&window, &group, serde_json::json!({"type":"resync"})),
                        "message" => {
                            if let Ok(event) = serde_json::from_str(&event.data) {
                                send(&window, &group, event);
                            }
                        }
                        _ => {}
                    }
                }
            }
            send(
                &window,
                &group,
                serde_json::json!({"type":"connection", "connected":false}),
            );
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
    streams.insert(key, task);
    Ok(())
}

#[tauri::command]
pub fn unsubscribe(window: WebviewWindow, group: String) -> super::Result<()> {
    super::local(&window)?;
    if let Some(task) = window
        .app_handle()
        .state::<Streams>()
        .lock()
        .unwrap()
        .remove(&group)
    {
        task.abort();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_local_app_can_read_and_write_api() {
        for method in ["GET", "POST", "PUT", "DELETE", "OPTIONS"] {
            for origin in ["tauri://localhost", "http://tauri.localhost"] {
                let req = Request::builder()
                    .method(method)
                    .uri("hub://localhost/api/v1/me")
                    .header(header::ORIGIN, origin)
                    .body(Vec::new())
                    .unwrap();
                assert!(allowed(&req));
            }
            for origin in [
                "hub://localhost",
                "null",
                "https://evil.test",
                "tauri://localhost.evil",
            ] {
                let req = Request::builder()
                    .method(method)
                    .uri("hub://localhost/api/v1/me")
                    .header(header::ORIGIN, origin)
                    .body(Vec::new())
                    .unwrap();
                assert!(!allowed(&req));
            }
        }
        for uri in [
            "hub://evil/api/v1/me",
            "hub://localhost/git/a",
            "hub://localhost/api/v1/../me",
            "hub://localhost/api/v1/%2e%2e/me",
        ] {
            let req = Request::builder()
                .uri(uri)
                .header(header::ORIGIN, "tauri://localhost")
                .body(Vec::new())
                .unwrap();
            assert!(!allowed(&req), "{uri}");
        }
    }
    #[test]
    fn documents_are_sandboxed_and_cannot_mutate() {
        let uri = "hub://localhost/api/v1/groups/algebra/attachments/1";
        assert!(allowed(
            &Request::builder().uri(uri).body(Vec::new()).unwrap()
        ));
        assert!(!allowed(
            &Request::builder()
                .method("POST")
                .uri(uri)
                .body(Vec::new())
                .unwrap()
        ));
        let reply = response(200, Vec::new(), None);
        assert_eq!(
            reply.headers()[header::CONTENT_SECURITY_POLICY],
            "default-src 'none'; style-src 'unsafe-inline'; img-src data:; sandbox"
        );
        assert!(
            !reply
                .headers()
                .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        );
        assert!(!reply.headers().contains_key(header::SET_COOKIE));
    }
}
