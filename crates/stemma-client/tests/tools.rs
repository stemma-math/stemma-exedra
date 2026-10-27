//! The tools and their MCP server, against a hub running in the test.

use std::sync::Arc;

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation};
use serde_json::{Value, json};
use stemma_client::mcp::StemmaServer;
use stemma_client::{Hub, Toolbox};
use stemma_core::api::NewAccount;
use stemma_fake_forge::FakeForge;
use stemma_hub::config::{ForgeConfig, ForgeSecrets};

/// A hub on a free local port, beside a stand-in for its forge, with Ada,
/// who administers it, and her token.
async fn hub_with_ada() -> (String, String, (tempfile::TempDir, FakeForge)) {
    let forge = FakeForge::start().await;
    let data = tempfile::tempdir().unwrap();
    let config = stemma_hub::Config {
        data_dir: data.path().to_owned(),
        forge: Some(ForgeConfig::new(
            &forge.url,
            "http://forge.test/user/oauth2/stemma/callback",
            ForgeSecrets {
                token: stemma_fake_forge::TOKEN.into(),
                webhook_secret: "the webhook's secret".into(),
                client_secret: "the client's secret".into(),
            },
        )),
        ..Default::default()
    };
    let state =
        Arc::new(stemma_hub::Hub::new(stemma_hub::db::in_memory().await.unwrap(), config).unwrap());
    let account = NewAccount {
        username: "ada".into(),
        display_name: "Ada".into(),
        password: "analytical".into(),
    };
    stemma_hub::api::create_user(&state, account, true)
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, stemma_hub::app(state)).await });

    let mut hub = Hub::new(&url, None).unwrap();
    hub.sign_in("ada", "analytical").await.unwrap();
    let token = hub.create_token("tests").await.unwrap().secret;
    hub.sign_out().await.unwrap();
    (url, token, (data, forge))
}

async fn raw_post(url: &str, token: &str, path: &str, body: Value) -> Value {
    reqwest::Client::new()
        .post(format!("{url}/api/v1{path}"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn the_tools_read_post_and_search() {
    let (url, token, _data) = hub_with_ada().await;
    raw_post(
        &url,
        &token,
        "/groups",
        json!({"slug": "geometry", "name": "Geometry"}),
    )
    .await;
    raw_post(
        &url,
        &token,
        "/groups/geometry/threads",
        json!({"title": "Compactification", "body": "Is the gluing map proper?"}),
    )
    .await;

    // With one group, the tools need not be told which.
    let tools = Toolbox::new(Hub::new(&url, Some(token)).unwrap(), None);
    let listed = tools.call("list_threads", Value::Null, None).await.unwrap();
    assert!(
        listed.starts_with("t1 – Compactification – 1 messages"),
        "{listed}"
    );

    let posted = tools
        .call(
            "post_message",
            json!({"thread": "t1#1", "body": "Yes: it is closed with compact fibres."}),
            Some("Claude Code"),
        )
        .await
        .unwrap();
    assert!(posted.starts_with("Posted t1#2: http://"), "{posted}");
    assert!(posted.trim_end().ends_with("/g/geometry/t/1#2"));

    let noted = tools
        .call(
            "notify",
            json!({"to": "@ada", "body": "See [[t1#2]]."}),
            Some("Claude Code"),
        )
        .await
        .unwrap();
    assert_eq!(noted, "Left a note for @ada.\n");

    let read = tools
        .call("read_thread", json!({"address": "[[t1#1–2]]"}), None)
        .await
        .unwrap();
    assert!(read.contains("### t1#1 – Ada – "), "{read}");
    assert!(
        read.contains("### t1#2 – Claude Code, for Ada – "),
        "{read}"
    );
    assert!(read.contains("reply to #1"));

    let whole = tools
        .call(
            "read_thread",
            json!({"address": format!("{url}/g/geometry/t/1")}),
            None,
        )
        .await
        .unwrap();
    assert!(whole.starts_with("# t1 – Compactification"), "{whole}");

    let found = tools
        .call("search", json!({"query": "compact fibres"}), None)
        .await
        .unwrap();
    assert!(found.starts_with("- t1#2 (Compactification)"), "{found}");

    let missing = tools
        .call("read_thread", json!({"address": "t1#9"}), None)
        .await
        .unwrap_err();
    assert_eq!(missing.to_string(), "t1 has no message #9");
    let paper = tools
        .call("read_thread", json!({"address": "p/notes"}), None)
        .await
        .unwrap_err();
    assert!(paper.to_string().contains("not found"), "{paper}");
}

#[tokio::test]
async fn the_paper_tools_list_papers_and_read_their_source() {
    let (url, token, _data) = hub_with_ada().await;
    raw_post(
        &url,
        &token,
        "/groups",
        json!({"slug": "geometry", "name": "Geometry"}),
    )
    .await;
    let hub = Hub::new(&url, Some(token.clone())).unwrap();
    let tools = Toolbox::new(hub.clone(), Some("geometry".into())).read_only(true);
    let none = tools.call("list_papers", json!({}), None).await.unwrap();
    assert_eq!(none, "No papers.\n");
    hub.create_paper("geometry", "notes", "Notes on gluing")
        .await
        .unwrap();

    let listed = tools.call("list_papers", json!({}), None).await.unwrap();
    assert!(
        listed.starts_with("- p/notes – Notes on gluing – by Ada – no PDF yet"),
        "{listed}"
    );
    // A paper's main file, with who writes it and its files.
    let read = tools
        .call("read_paper", json!({"address": "p/notes"}), None)
        .await
        .unwrap();
    assert!(
        read.starts_with("# p/notes – Notes on gluing\n\nBy Ada."),
        "{read}"
    );
    assert!(
        read.contains("Files: .gitignore, main.tex, references.bib."),
        "{read}"
    );
    assert!(read.contains("## main.tex"), "{read}");
    assert!(read.contains("\\title{Notes on gluing}"), "{read}");
    // Read as any address is, and a file asked for.
    let same = tools
        .call("read_thread", json!({"address": "[[p/notes]]"}), None)
        .await
        .unwrap();
    assert!(same.contains("\\title{Notes on gluing}"), "{same}");
    let bib = tools
        .call(
            "read_paper",
            json!({"address": "notes", "file": "references.bib"}),
            None,
        )
        .await
        .unwrap();
    assert!(bib.contains("@book{example"), "{bib}");
    let missing = tools
        .call(
            "read_paper",
            json!({"address": "p/notes", "file": "intro.tex"}),
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(
        missing.to_string(),
        "p/notes has no file intro.tex: it has .gitignore, main.tex, references.bib"
    );
    let not_a_paper = tools
        .call("read_paper", json!({"address": "[[t1]]"}), None)
        .await
        .unwrap_err();
    assert!(not_a_paper.to_string().contains("is not a paper"));
    // Only read: the toolbox an agent's job has keeps them.
    let names: Vec<&str> = tools.definitions().iter().map(|t| t.name).collect();
    assert!(names.contains(&"list_papers") && names.contains(&"read_paper"));
}

#[tokio::test]
async fn the_library_tools_say_how_it_stands() {
    let (url, token, _data) = hub_with_ada().await;
    raw_post(
        &url,
        &token,
        "/groups",
        json!({"slug": "geometry", "name": "Geometry"}),
    )
    .await;
    let tools = Toolbox::new(Hub::new(&url, Some(token)).unwrap(), None);
    // The library has not been built in this hub.
    let state = tools.call("library_state", json!({}), None).await.unwrap();
    assert!(
        state.contains("No build has passed yet") || state.contains("keeps no library"),
        "{state}"
    );
    let missing = tools
        .call("read_thread", json!({"address": "lean:Foo.bar"}), None)
        .await
        .unwrap();
    assert!(missing.starts_with("lean:Foo.bar is missing"), "{missing}");
}

#[tokio::test]
async fn mcp_serves_the_tools_and_names_the_agent() {
    let (url, token, _data) = hub_with_ada().await;
    raw_post(
        &url,
        &token,
        "/groups",
        json!({"slug": "geometry", "name": "Geometry"}),
    )
    .await;
    raw_post(
        &url,
        &token,
        "/groups/geometry/threads",
        json!({"title": "Gluing", "body": "Explain #1"}),
    )
    .await;
    let hub = Hub::new(&url, Some(token)).unwrap();

    let (server, client) = tokio::io::duplex(64 * 1024);
    let toolbox = Toolbox::new(hub.clone(), Some("geometry".into()));
    tokio::spawn(async move {
        let running = StemmaServer::new(toolbox, None).serve(server).await?;
        running.waiting().await?;
        anyhow::Ok(())
    });
    let config = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("claude-code", "2.0.0"),
    );
    let client = config.serve(client).await.unwrap();

    let tools = client.list_tools(None).await.unwrap().tools;
    for tool in &tools {
        let annotations = tool.annotations.as_ref().expect("tool annotations");
        let writes = matches!(
            tool.name.as_ref(),
            "post_message"
                | "notify"
                | "open_thread"
                | "create_paper"
                | "ask_workshop"
                | "cancel_request"
                | "upload_attachment"
        );
        assert_eq!(annotations.read_only_hint, Some(!writes), "{}", tool.name);
        assert_eq!(annotations.destructive_hint, Some(false));
        assert_eq!(annotations.idempotent_hint, Some(!writes));
        assert_eq!(annotations.open_world_hint, Some(true));
    }
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(
        names,
        [
            "list_threads",
            "read_thread",
            "post_message",
            "notify",
            "open_thread",
            "search",
            "library_state",
            "library_lookup",
            "library_search",
            "create_paper",
            "list_papers",
            "read_paper",
            "list_recipes",
            "read_recipe",
            "list_requests",
            "read_request",
            "ask_workshop",
            "cancel_request",
            "download_attachment",
            "upload_attachment",
        ]
    );

    let mut call = CallToolRequestParams::new("post_message");
    call.arguments = json!({"thread": "t1", "body": "Done."})
        .as_object()
        .cloned();
    let result = client.call_tool(call).await.unwrap();
    assert_eq!(result.is_error, Some(false), "{result:?}");
    let messages = hub.messages("geometry", 1, 2, 2).await.unwrap();
    assert_eq!(messages[0].agent.as_deref(), Some("Claude Code"));
    assert_eq!(messages[0].agent_runner.as_deref(), Some("claude-code"));

    let mut call = CallToolRequestParams::new("read_thread");
    call.arguments = json!({"address": "t7"}).as_object().cloned();
    let result = client.call_tool(call).await.unwrap();
    assert_eq!(result.is_error, Some(true));
    client.cancel().await.unwrap();

    // A read-only server offers no tool that writes.
    let (server, client) = tokio::io::duplex(64 * 1024);
    let toolbox = Toolbox::new(hub, Some("geometry".into())).read_only(true);
    tokio::spawn(async move {
        let running = StemmaServer::new(toolbox, Some("Claude".into()))
            .serve(server)
            .await?;
        running.waiting().await?;
        anyhow::Ok(())
    });
    let client = ().serve(client).await.unwrap();
    let tools = client.list_tools(None).await.unwrap().tools;
    assert!(
        tools
            .iter()
            .all(|t| t.name != "post_message" && t.name != "create_task")
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn attachments_return_text_images_or_a_saved_file_and_explain_refusals() {
    let (url, token, _data) = hub_with_ada().await;
    raw_post(
        &url,
        &token,
        "/groups",
        json!({"slug": "geometry", "name": "Geometry"}),
    )
    .await;
    let mut ids = Vec::new();
    for (name, content_type, bytes) in [
        (
            "notes.tex",
            "application/x-tex",
            b"A proof of the lemma.".to_vec(),
        ),
        ("board.png", "image/png", b"\x89PNG\r\n\x1a\n".to_vec()),
        ("archive.bin", "application/octet-stream", vec![1, 2, 3]),
        ("large.txt", "text/plain", vec![b'x'; 65 * 1024]),
    ] {
        let mut body = format!("--test-boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: {content_type}\r\n\r\n").into_bytes();
        body.extend_from_slice(&bytes);
        body.extend_from_slice(b"\r\n--test-boundary--\r\n");
        let uploaded: Value = reqwest::Client::new()
            .post(format!("{url}/api/v1/groups/geometry/attachments"))
            .bearer_auth(&token)
            .header(
                "content-type",
                "multipart/form-data; boundary=test-boundary",
            )
            .body(body)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        ids.push(uploaded["id"].as_i64().expect("uploaded attachment"));
    }
    raw_post(
        &url,
        &token,
        "/groups/geometry/threads",
        json!({"title": "Files", "body": "Read these", "attachments": ids}),
    )
    .await;
    let files = tempfile::tempdir().unwrap();
    let hub = Hub::new(&url, Some(token)).unwrap();
    let session = hub
        .create_agent_token(
            "geometry",
            "claude-code",
            "test",
            stemma_core::api::AttachmentRule::Mine,
        )
        .await
        .unwrap();
    let tools = Toolbox::new(
        Hub::new(&url, Some(session.secret)).unwrap(),
        Some("geometry".into()),
    )
    .read_only(true)
    .attachment_dir(files.path().into());
    let read = tools
        .call("read_thread", json!({"address": "t1"}), None)
        .await
        .unwrap();
    assert!(
        read.contains(&format!("Attachment {}:", ids[0]))
            && read.contains("notes.tex")
            && read.contains("application/x-tex")
            && read.contains("21 bytes")
            && read.contains("Ada (@ada)"),
        "{read}"
    );
    let text = tools
        .call_content("download_attachment", json!({"attachment": ids[0]}), None)
        .await
        .unwrap();
    assert!(
        text[0]
            .as_text()
            .unwrap()
            .text
            .contains("A proof of the lemma.")
    );
    let image = tools
        .call_content("download_attachment", json!({"attachment": ids[1]}), None)
        .await
        .unwrap();
    let image = image[0].as_image().unwrap();
    assert_eq!(image.mime_type, "image/png");
    assert_eq!(image.data, "iVBORw0KGgo=");
    for (id, expected) in [(ids[2], vec![1, 2, 3]), (ids[3], vec![b'x'; 65 * 1024])] {
        let result = tools
            .call_content("download_attachment", json!({"attachment": id}), None)
            .await
            .unwrap();
        let text = &result[0].as_text().unwrap().text;
        let path = std::path::Path::new(text.split(" to ").nth(1).unwrap());
        assert!(path.starts_with(files.path()));
        assert_eq!(std::fs::read(path).unwrap(), expected);
    }
    // Inline content leaves no files; only the two saved attachments remain.
    assert_eq!(std::fs::read_dir(files.path()).unwrap().count(), 2);
    let session = hub
        .create_agent_token(
            "geometry",
            "claude-code",
            "test",
            stemma_core::api::AttachmentRule::None,
        )
        .await
        .unwrap();
    let tools = Toolbox::new(
        Hub::new(&url, Some(session.secret)).unwrap(),
        Some("geometry".into()),
    )
    .attachment_dir(files.path().into());
    let error = tools
        .call_content("download_attachment", json!({"attachment": ids[0]}), None)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        stemma_core::api::AttachmentRule::None.refusal()
    );
    assert_eq!(std::fs::read_dir(files.path()).unwrap().count(), 2);
}

#[tokio::test]
async fn former_group_names_render_as_current_names_in_tool_headings() {
    let (url, token, _data) = hub_with_ada().await;
    raw_post(
        &url,
        &token,
        "/groups",
        json!({"slug":"geometry", "name":"Geometry"}),
    )
    .await;
    raw_post(
        &url,
        &token,
        "/groups/geometry/threads",
        json!({"title":"A proof", "body":"See [[geometry/t1]]."}),
    )
    .await;
    let tools = Toolbox::new(
        Hub::new(&url, Some(token.clone())).unwrap(),
        Some("geometry".into()),
    );
    let renamed = reqwest::Client::new()
        .patch(format!("{url}/api/v1/groups/geometry"))
        .bearer_auth(&token)
        .json(&json!({"slug":"topology"}))
        .send()
        .await
        .unwrap();
    assert!(renamed.status().is_success());
    for args in [
        json!({"address":"t1"}),
        json!({"address":"t1", "group":"geometry"}),
    ] {
        let text = tools.call("read_thread", args, None).await.unwrap();
        assert!(text.contains("in topology"), "{text}");
        assert!(text.contains("[[topology/t1]]"), "{text}");
        assert!(!text.contains("geometry"), "{text}");
    }
}
