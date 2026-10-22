//! Stop the actual hub process with long-lived clients still connected.
#![cfg(unix)]

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use stemma_core::api::NewAccount;
use stemma_core::protocol::{Hello, HubMessage, PROTOCOL_VERSION, WorkerMessage};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

#[tokio::test]
async fn signals_drain_streams_and_machines_and_exit_successfully() {
    stemma_client::hub::install_crypto();
    for signal in ["-TERM", "-INT"] {
        let dir = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let url = format!("http://{address}");
        let state = stemma_hub::open(stemma_hub::Config {
            data_dir: dir.path().to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
        let user = stemma_hub::api::create_user(
            &state,
            NewAccount {
                username: "ada".into(),
                display_name: "Ada".into(),
                password: "analytical".into(),
            },
            true,
        )
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO groups (id, slug, name, created_at) VALUES (1, 'geometry', 'Geometry', 0)",
        )
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO memberships (group_id, user_id, role, joined_at) VALUES (1, ?, 'admin', 0)")
            .bind(user).execute(&state.db).await.unwrap();
        state.db.close().await;
        drop(state);
        drop(listener);
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_stemma"))
            .args(["hub"])
            .env_remove("STEMMA_CONFIG")
            .env_remove("STEMMA_FORGE_URL")
            .env("STEMMA_DATA_DIR", dir.path())
            .env("STEMMA_LISTEN", address.to_string())
            .env("STEMMA_PUBLIC_URL", &url)
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let http = reqwest::Client::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if http
                    .get(format!("{url}/api/v1/health"))
                    .send()
                    .await
                    .is_ok()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("hub starts");
        let mut hub = stemma_client::Hub::new(&url, None).unwrap();
        hub.sign_in("ada", "analytical").await.unwrap();
        let token = hub.create_token("shutdown test").await.unwrap().secret;
        let machine = hub.create_token("Test machine").await.unwrap();
        let mut events = http
            .get(format!("{url}/api/v1/groups/geometry/events"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .bytes_stream();
        assert!(
            events
                .next()
                .await
                .unwrap()
                .unwrap()
                .windows(5)
                .any(|s| s == b"ready")
        );
        let mut request = format!("ws://{address}/api/v1/worker")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {}", machine.secret).parse().unwrap(),
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        socket
            .send(Message::Text(
                serde_json::to_string(&WorkerMessage::Hello(Hello {
                    protocol: PROTOCOL_VERSION,
                    version: env!("CARGO_PKG_VERSION").into(),
                    runners: vec![],
                    agents: stemma_core::protocol::GroupAgents {
                        attachments: Default::default(),
                        enabled: true,
                        priority: 0,
                        concurrency: None,
                        runners: None,
                    },
                    groups: Default::default(),
                    concurrency: 1,
                }))
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        let Message::Text(welcome) = socket.next().await.unwrap().unwrap() else {
            panic!("welcome")
        };
        assert!(matches!(
            serde_json::from_str::<HubMessage>(&welcome).unwrap(),
            HubMessage::Welcome(_)
        ));
        let pid = child.id().unwrap().to_string();
        assert!(
            tokio::process::Command::new("kill")
                .args([signal, &pid])
                .status()
                .await
                .unwrap()
                .success()
        );
        tokio::time::timeout(Duration::from_secs(6), async {
            while let Some(chunk) = events.next().await { chunk.unwrap(); }
            loop {
                match socket.next().await {
                    Some(Ok(Message::Close(Some(frame)))) => {
                        assert_eq!(frame.code, tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal);
                        break;
                    }
                    Some(Ok(Message::Ping(_))) => { socket.flush().await.unwrap(); }
                    Some(Ok(Message::Text(text))) => {
                        assert!(matches!(serde_json::from_str::<HubMessage>(&text).unwrap(), HubMessage::Identity(_)));
                    }
                    other => panic!("expected normal close, got {other:?}"),
                }
            }
            assert!(child.wait().await.unwrap().success());
        }).await.expect("hub exits within its grace period");
        assert!(tokio::net::TcpStream::connect(address).await.is_err());
    }
}
