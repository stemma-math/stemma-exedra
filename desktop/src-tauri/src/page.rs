//! A separate loopback origin for the privileged, bundled interface.
//!
//! Tauri classifies ALL registered custom protocols as Local (including hub).
//! Local capabilities therefore stay disabled. Only this exact HTTP origin
//! receives a remote capability; hub documents receive none.
use anyhow::Context;
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Request, Response, StatusCode, header},
};
use tauri::Manager;

pub struct Origin(pub tauri::Url);

pub fn same_origin(url: &tauri::Url, origin: &tauri::Url) -> bool {
    url.origin() == origin.origin()
}

pub fn capability(origin: &tauri::Url) -> serde_json::Value {
    let mut capability: serde_json::Value =
        serde_json::from_str(include_str!("../capabilities/local.json")).expect("local capability");
    capability["identifier"] = "interface".into();
    capability["remote"] =
        serde_json::json!({"urls": [format!("{}/*", origin.origin().ascii_serialization())]});
    capability
}

pub async fn serve(app: tauri::AppHandle) -> anyhow::Result<tauri::Url> {
    // Keep the origin stable across launches, so theme and navigation survive.
    // Never connect to an occupied address: this process must own its listener.
    let folder = app.path().app_local_data_dir()?;
    let path = folder.join("interface-port");
    let port: u16 = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .context("Stemma is already open, or its local address is unavailable")?;
    std::fs::create_dir_all(folder)?;
    std::fs::write(path, listener.local_addr()?.port().to_string())?;
    let address = listener.local_addr()?;
    let origin: tauri::Url = format!("http://{address}/").parse()?;
    let router = Router::new()
        .fallback(asset)
        .with_state((app, address.to_string()));
    tauri::async_runtime::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Ok(origin)
}

async fn asset(
    State((app, host)): State<(tauri::AppHandle, String)>,
    request: Request<Body>,
) -> Response<Body> {
    if request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        != Some(host.as_str())
        || request.method() != axum::http::Method::GET
    {
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Body::empty())
            .unwrap();
    }
    let path = request.uri().path().trim_start_matches('/');
    let assets = app.asset_resolver();
    let file = assets.get(path.to_owned()).or_else(|| {
        if path.starts_with("_app/") || path.starts_with("agents/") {
            None
        } else {
            assets.get("index.html".into())
        }
    });
    let Some(file) = file else {
        return Response::builder().status(404).body(Body::empty()).unwrap();
    };
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("app config");
    let csp = config["app"]["security"]["csp"]
        .as_str()
        .context("page CSP")
        .expect("configured CSP");
    Response::builder()
        .header(header::CONTENT_TYPE, file.mime_type)
        .header(
            header::CONTENT_SECURITY_POLICY,
            format!("{csp}; frame-ancestors 'none'"),
        )
        .header(header::CACHE_CONTROL, "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("X-Frame-Options", "DENY")
        .body(Body::from(file.bytes))
        .expect("page response")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_exact_interface_origin_gets_commands() {
        let origin: tauri::Url = "http://127.0.0.1:12345/".parse().unwrap();
        let cap = capability(&origin);
        // Registered schemes are Local in Tauri, so neither hub nor tauri gets permission.
        assert_eq!(cap["local"], false);
        assert_eq!(cap["windows"], serde_json::json!(["main"]));
        assert_eq!(
            cap["remote"]["urls"],
            serde_json::json!(["http://127.0.0.1:12345/*"])
        );
        assert!(same_origin(
            &"http://127.0.0.1:12345/g/algebra".parse().unwrap(),
            &origin
        ));
        for url in [
            "hub://localhost/",
            "http://hub.localhost/",
            "tauri://localhost/",
            "http://127.0.0.1:12346/",
            "http://localhost:12345/",
            "https://evil.test/",
        ] {
            assert!(!same_origin(&url.parse().unwrap(), &origin), "{url}");
        }
    }
}
