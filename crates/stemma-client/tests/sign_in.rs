//! Sign-in failures name what happened without touching local credentials.
use axum::{Router, http::StatusCode, routing::post};
use stemma_client::Hub;

async fn address(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    url
}

#[tokio::test]
async fn sign_in_errors_are_sentences() {
    let url = address(Router::new().route("/api/v1/session", post(|| async { (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": {"code": "wrong_credentials", "message": "Wrong password"}}))) }))).await;
    let error = Hub::new(&url, None)
        .unwrap()
        .sign_in("ada", "wrong")
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "That username and password don't match.");
    for status in [
        StatusCode::OK,
        StatusCode::NOT_FOUND,
        StatusCode::UNAUTHORIZED,
    ] {
        let url = address(Router::new().route(
            "/api/v1/session",
            post(move || async move { (status, "<html>Another site</html>") }),
        ))
        .await;
        let error = Hub::new(&url, None)
            .unwrap()
            .sign_in("ada", "wrong")
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with(&format!("{url} did not answer as a Stemma hub"))
        );
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let error = Hub::new(&url, None)
        .unwrap()
        .sign_in("ada", "wrong")
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with(&format!("Could not reach {url}:"))
    );
}
