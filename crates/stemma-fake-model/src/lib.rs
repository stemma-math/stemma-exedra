//! A scripted provider speaking both Messages and Chat Completions.
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Script {
    replies: VecDeque<(StatusCode, Value)>,
    requests: Vec<Value>,
}
pub struct FakeModel {
    pub url: String,
    script: Arc<Mutex<Script>>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for FakeModel {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl FakeModel {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let script = Arc::new(Mutex::new(Script::default()));
        let app = Router::new()
            .route("/", post(answer))
            .with_state(script.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url,
            script,
            server,
        }
    }
    /// Scripted responses deliberately contain both wire formats, so the same
    /// scenario can verify each provider adapter without provider credentials.
    pub fn tool(&self, name: &str, arguments: Value) {
        self.reply(StatusCode::OK,json!({
            "choices":[{"message":{"role":"assistant","content":"","tool_calls":[{"id":"call_1","type":"function","function":{"name":name,"arguments":arguments.to_string()}}]}}],
            "content":[{"type":"tool_use","id":"call_1","name":name,"input":arguments}],
            "usage":{"input_tokens":10,"output_tokens":5,"prompt_tokens":10,"completion_tokens":5,"cost":0.001}
        }));
    }
    pub fn text(&self, text: &str) {
        self.reply(StatusCode::OK,json!({
            "choices":[{"message":{"role":"assistant","content":text}}],
            "content":[{"type":"text","text":text}],
            "usage":{"input_tokens":10,"output_tokens":5,"prompt_tokens":10,"completion_tokens":5}
        }));
    }
    pub fn reply(&self, status: StatusCode, body: Value) {
        self.script
            .lock()
            .unwrap()
            .replies
            .push_back((status, body));
    }
    pub fn requests(&self) -> Vec<Value> {
        self.script.lock().unwrap().requests.clone()
    }
}
async fn answer(State(script): State<Arc<Mutex<Script>>>, Json(body): Json<Value>) -> Response {
    let mut script = script.lock().unwrap();
    script.requests.push(body);
    let (status, value) = script.replies.pop_front().unwrap_or((
        StatusCode::BAD_REQUEST,
        json!({"error":{"message":"The fake provider's script is exhausted"}}),
    ));
    (status, Json(value)).into_response()
}
