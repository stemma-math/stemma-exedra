//! The credential boundary: only Rust knows the device token and hub address.
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, bail};
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use reqwest::{Client, Method};
use serde::Serialize;
use stemma_client::config::Credentials;
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
    let config = Credentials::load()?.with_env();
    let base = config.hub()?.base().to_owned();
    Ok((base, config.token.context("Sign in to your hub")?))
}

fn allowed(request: &Request<Vec<u8>>, local_origin: &tauri::Url) -> bool {
    let uri = request.uri();
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    let from_app = origin.is_some_and(|o| {
        o.parse()
            .is_ok_and(|url| super::page::same_origin(&url, local_origin))
    });
    let path = uri.path();
    let resource = path.contains("/attachments/") || path.ends_with("/pdf");
    matches!((uri.scheme_str(), uri.authority().map(|a| a.as_str())),
        (Some("hub"), Some("localhost")) | (Some("http"), Some("hub.localhost")))
        && path.starts_with("/api/v1/")
        && path.split('/').all(|part| {
            percent_encoding::percent_decode_str(part).decode_utf8().is_ok_and(|part| {
                part != "." && part != ".." && !part.contains(['/', '\\']) && !part.chars().any(char::is_control)
            })
        })
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

pub async fn proxy(request: Request<Vec<u8>>, local_origin: &tauri::Url) -> Response<Vec<u8>> {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .filter(|o| {
            o.parse()
                .is_ok_and(|url| super::page::same_origin(&url, local_origin))
        })
        .map(str::to_owned);
    if !allowed(&request, local_origin) {
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
            pdf_policy(&mut reply);
            reply
        }
        Err(_) => response(
            502,
            br#"{"error":{"code":"offline","message":"Could not reach the hub"}}"#.to_vec(),
            origin.as_deref(),
        ),
    }
}

/// Native PDF viewers cannot render a sandboxed response. Only the exact PDF
/// MIME type gets this exception: nosniff is always set, and this resource origin
/// has neither native capabilities nor permission to read or mutate the API.
fn pdf_policy(reply: &mut Response<Vec<u8>>) {
    if reply
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|v| v == "application/pdf")
    {
        reply.headers_mut().insert(
            header::CONTENT_SECURITY_POLICY,
            header::HeaderValue::from_static(
                "default-src 'none'; base-uri 'none'; form-action 'none'",
            ),
        );
    }
}

async fn forward(
    request: Request<Vec<u8>>,
) -> anyhow::Result<(u16, reqwest::header::HeaderMap, Vec<u8>)> {
    let (base, token) = credentials()?;
    let identity = request.uri().path() == "/api/v1/me";
    let reply = forward_to(request, &base, &token).await?;
    let paths = stemma_client::config::Paths::discover()?;
    if let Some(username) = reply.1.get("x-stemma-user").and_then(|v| v.to_str().ok()) {
        paths.update_username(&base, &token, username)?;
    }
    if identity && reply.0 == 200 {
        let me: stemma_core::api::Me = serde_json::from_slice(&reply.2)?;
        paths.reconcile_groups(&base, &me.groups)?;
    }
    Ok(reply)
}

async fn forward_to(
    request: Request<Vec<u8>>,
    base: &str,
    token: &str,
) -> anyhow::Result<(u16, reqwest::header::HeaderMap, Vec<u8>)> {
    let path = request.uri().path_and_query().context("Missing path")?;
    let client = client()?;
    let mut path = path.to_string();
    let mut redirects = 0;
    let reply = loop {
        let mut outgoing = client
            .request(request.method().clone(), format!("{base}{path}"))
            .bearer_auth(token)
            .timeout(Duration::from_secs(120));
        for key in [header::CONTENT_TYPE, header::ACCEPT, header::RANGE] {
            if let Some(value) = request.headers().get(&key) {
                outgoing = outgoing.header(key, value);
            }
        }
        let reply = outgoing.body(request.body().clone()).send().await?;
        if !reply.status().is_redirection() {
            break reply;
        }
        let location = reply
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        if reply.status() != reqwest::StatusCode::PERMANENT_REDIRECT
            || redirects >= 3
            || !canonical_name_redirect(&path, location)
        {
            bail!("The hub redirected a credentialed request outside its canonical address");
        }
        path = location.into();
        redirects += 1;
    };
    let status = reply.status().as_u16();
    let headers = reply.headers().clone();
    Ok((status, headers, reply.bytes().await?.to_vec()))
}

fn canonical_name_redirect(old: &str, new: &str) -> bool {
    for prefix in ["/api/v1/groups/", "/api/v1/users/"] {
        if let (Some(old), Some(new)) = (old.strip_prefix(prefix), new.strip_prefix(prefix)) {
            let split = |s: &str| {
                let end = s.find(['/', '?']).unwrap_or(s.len());
                (s[..end].to_owned(), s[end..].to_owned())
            };
            let (old_name, old_suffix) = split(old);
            let (new_name, new_suffix) = split(new);
            let safe = |s: &str| {
                !s.is_empty()
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            };
            return old_suffix == new_suffix && safe(&old_name) && safe(&new_name);
        }
    }
    false
}

#[derive(Clone, Serialize)]
struct LiveEvent {
    subscription: String,
    group: String,
    event: serde_json::Value,
}

fn send(window: &WebviewWindow, subscription: &str, group: &str, event: serde_json::Value) {
    let _ = window.emit(
        "hub-event",
        LiveEvent {
            subscription: subscription.into(),
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
pub async fn subscribe(
    window: WebviewWindow,
    group: String,
    subscription: String,
) -> super::Result<()> {
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
    if streams.contains_key(&subscription) {
        return Ok(());
    }
    let key = subscription.clone();
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
                                send(
                                    &window,
                                    &subscription,
                                    &group,
                                    serde_json::json!({"type":"resync"}),
                                );
                            }
                            connected = true;
                            send(
                                &window,
                                &subscription,
                                &group,
                                serde_json::json!({"type":"connection", "connected":true}),
                            );
                        }
                        "resync" => send(
                            &window,
                            &subscription,
                            &group,
                            serde_json::json!({"type":"resync"}),
                        ),
                        "message" => {
                            if let Ok(event) = serde_json::from_str(&event.data) {
                                send(&window, &subscription, &group, event);
                            }
                        }
                        _ => {}
                    }
                }
            }
            send(
                &window,
                &subscription,
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
pub fn unsubscribe(window: WebviewWindow, subscription: String) -> super::Result<()> {
    super::local(&window)?;
    if let Some(task) = window
        .app_handle()
        .state::<Streams>()
        .lock()
        .unwrap()
        .remove(&subscription)
    {
        task.abort();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_canonical_name_redirects_keep_credentials() {
        assert!(canonical_name_redirect(
            "/api/v1/groups/old/threads?limit=2",
            "/api/v1/groups/new/threads?limit=2"
        ));
        for target in [
            "https://evil.test/api/v1/groups/new",
            "//evil.test/",
            "/api/v1/me",
            "/api/v1/groups/new/../me",
            "/api/v1/groups/new/threads?limit=3",
        ] {
            assert!(!canonical_name_redirect(
                "/api/v1/groups/old/threads?limit=2",
                target
            ));
        }
    }
    #[test]
    fn only_local_app_can_read_and_write_api() {
        for method in ["GET", "POST", "PUT", "DELETE", "OPTIONS"] {
            {
                let origin = "http://127.0.0.1:12345";
                let req = Request::builder()
                    .method(method)
                    .uri("hub://localhost/api/v1/me")
                    .header(header::ORIGIN, origin)
                    .body(Vec::new())
                    .unwrap();
                assert!(allowed(&req, &"http://127.0.0.1:12345".parse().unwrap()));
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
                assert!(!allowed(&req, &"http://127.0.0.1:12345".parse().unwrap()));
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
                .header(header::ORIGIN, "http://127.0.0.1:12345")
                .body(Vec::new())
                .unwrap();
            assert!(
                !allowed(&req, &"http://127.0.0.1:12345".parse().unwrap()),
                "{uri}"
            );
        }
    }
    #[test]
    fn documents_are_sandboxed_and_cannot_mutate() {
        let uri = "hub://localhost/api/v1/groups/algebra/attachments/1";
        assert!(allowed(
            &Request::builder().uri(uri).body(Vec::new()).unwrap(),
            &"http://127.0.0.1:12345".parse().unwrap()
        ));
        assert!(!allowed(
            &Request::builder()
                .method("POST")
                .uri(uri)
                .body(Vec::new())
                .unwrap(),
            &"http://127.0.0.1:12345".parse().unwrap()
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
    #[test]
    fn only_typed_pdfs_allow_the_native_viewer() {
        for kind in [
            "application/pdf",
            "text/html",
            "image/svg+xml",
            "application/octet-stream",
        ] {
            let mut reply = response(200, b"%PDF-test".to_vec(), None);
            reply
                .headers_mut()
                .insert(header::CONTENT_TYPE, kind.parse().unwrap());
            pdf_policy(&mut reply);
            assert_eq!(reply.headers()["X-Content-Type-Options"], "nosniff");
            let csp = reply.headers()[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap();
            assert_eq!(csp.contains("sandbox"), kind != "application/pdf");
            assert!(csp.contains("default-src 'none'"));
            assert!(
                !reply
                    .headers()
                    .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            );
        }
    }

    #[tokio::test]
    async fn proxy_preserves_bytes_and_ranges_and_never_follows_redirects() {
        use axum::{Router, response::IntoResponse, routing::any};
        let app = Router::new().fallback(any(|request: axum::extract::Request| async move {
            assert_eq!(
                request.headers()[header::AUTHORIZATION],
                "Bearer native-secret"
            );
            assert!(!request.headers().contains_key(header::COOKIE));
            if request.uri().path().ends_with("redirect") {
                return (
                    axum::http::StatusCode::FOUND,
                    [(header::LOCATION, "http://evil.test/")],
                )
                    .into_response();
            }
            assert_eq!(
                request.headers()[header::CONTENT_TYPE],
                "multipart/form-data; boundary=upload"
            );
            assert_eq!(request.headers()[header::RANGE], "bytes=0-9");
            let bytes = axum::body::to_bytes(request.into_body(), 1024)
                .await
                .unwrap();
            assert_eq!(&bytes[..], b"\0binary\r\nupload\xff");
            (
                axum::http::StatusCode::PARTIAL_CONTENT,
                [
                    (header::CONTENT_TYPE, "application/pdf"),
                    (header::CONTENT_RANGE, "bytes 0-9/10"),
                ],
                b"%PDF-test".to_vec(),
            )
                .into_response()
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let request = Request::builder()
            .method("POST")
            .uri("hub://localhost/api/v1/upload")
            .header(header::CONTENT_TYPE, "multipart/form-data; boundary=upload")
            .header(header::RANGE, "bytes=0-9")
            .header(header::COOKIE, "must-not-forward=this")
            .body(b"\0binary\r\nupload\xff".to_vec())
            .unwrap();
        let (status, headers, body) = forward_to(request, &base, "native-secret").await.unwrap();
        assert_eq!(status, 206);
        assert_eq!(headers[header::CONTENT_RANGE], "bytes 0-9/10");
        assert_eq!(body, b"%PDF-test");
        let request = Request::builder()
            .uri("hub://localhost/api/v1/redirect")
            .body(Vec::new())
            .unwrap();
        assert!(forward_to(request, &base, "native-secret").await.is_err());
        task.abort();
    }

    #[test]
    fn unicode_identifiers_work_but_encoded_traversal_does_not() {
        for (path, expected) in [
            ("/api/v1/groups/a/library/%CE%B1.%CE%B2", true),
            ("/api/v1/%2e%2e/me", false),
            ("/api/v1/a%2fb", false),
            ("/api/v1/a%5cb", false),
        ] {
            let request = Request::builder()
                .uri(format!("hub://localhost{path}"))
                .header(header::ORIGIN, "http://127.0.0.1:12345")
                .body(Vec::new())
                .unwrap();
            assert_eq!(
                allowed(&request, &"http://127.0.0.1:12345".parse().unwrap()),
                expected,
                "{path}"
            );
        }
    }
}
