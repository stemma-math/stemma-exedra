//! The web app, built from `web/` and embedded in the binary.
//!
//! A release build embeds `web/build` as it is when the hub is compiled;
//! a debug build reads it from disk at each request. Without a built web
//! app, the hub serves a page that says how to build it.

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/build"]
#[allow_missing = true]
struct Assets;

const PLACEHOLDER: &str = include_str!("placeholder.html");

/// Serves a file of the web app, or the app itself for any other path, so
/// that it can route on the client.
pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if let Some(response) = asset(path) {
        return response;
    }
    // A missing file with an extension is a missing file, not a page; but a
    // group's pages have dots in them, as declarations do: `lib/Foo.bar`.
    if !path.starts_with("g/")
        && path
            .rsplit('/')
            .next()
            .is_some_and(|last| last.contains('.'))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    // The app's entry point, which adapter-static names `200.html`.
    asset("200.html")
        .or_else(|| asset("index.html"))
        .unwrap_or_else(|| {
            (
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                PLACEHOLDER,
            )
                .into_response()
        })
}

fn asset(path: &str) -> Option<Response> {
    let file = Assets::get(path)?;
    let cache = if path.starts_with("_app/immutable/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let mime = file.metadata.mimetype();
    let mut response = Body::from(file.data.into_owned()).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    Some(response)
}
