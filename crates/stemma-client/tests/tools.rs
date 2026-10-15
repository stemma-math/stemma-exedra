//! The tools and their MCP server, against a hub running in the test.

use std::sync::Arc;

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation};
use serde_json::{Value, json};
use stemma_client::mcp::StemmaServer;
use stemma_client::{Hub, Toolbox};
use stemma_core::api::NewAccount;

/// A hub on a free local port, with Ada, who administers it, and her token.
async fn hub_with_ada() -> (String, String, tempfile::TempDir) {
    let data = tempfile::tempdir().unwrap();
    let config = stemma_hub::Config {
        data_dir: data.path().to_owned(),
        ..Default::default()
    };
    let state = Arc::new(stemma_hub::Hub::new(
        stemma_hub::db::in_memory().await.unwrap(),
        config,
    ));
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
    (url, token, data)
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
        listed.starts_with("t1 · Compactification · 1 messages"),
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

    let read = tools
        .call("read_thread", json!({"address": "[[t1#1–2]]"}), None)
        .await
        .unwrap();
    assert!(read.contains("### t1#1 · Ada · "), "{read}");
    assert!(
        read.contains("### t1#2 · Claude Code, for Ada · "),
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
    assert!(whole.starts_with("# t1 · Compactification"), "{whole}");

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
    assert_eq!(paper.to_string(), "Paper not found");
}

#[tokio::test]
async fn the_paper_tools_read_write_and_compare() {
    let (url, token, _data) = hub_with_ada().await;
    raw_post(
        &url,
        &token,
        "/groups",
        json!({"slug": "geometry", "name": "Geometry"}),
    )
    .await;
    let tools = Toolbox::new(Hub::new(&url, Some(token)).unwrap(), None);
    assert_eq!(
        tools.call("list_papers", Value::Null, None).await.unwrap(),
        "No papers.\n"
    );

    // Starting a paper needs a title.
    let body =
        "Where we are.\n\n# Setting\n\nA scheme $X$.\n\n# Open\n\n> [!open]\n> Is it proper?\n";
    let refused = tools
        .call(
            "propose_paper_edit",
            json!({"paper": "p/notes", "body": body, "reason": "Start"}),
            Some("Claude Code"),
        )
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("give a title"), "{refused}");
    let stored = tools
        .call(
            "propose_paper_edit",
            json!({"paper": "p/notes", "body": body, "reason": "Start", "title": "Notes"}),
            Some("Claude Code"),
        )
        .await
        .unwrap();
    assert!(
        stored.starts_with("Stored p/notes at revision 1: http://"),
        "{stored}"
    );

    let listed = tools.call("list_papers", Value::Null, None).await.unwrap();
    assert!(
        listed.starts_with("p/notes · Notes · revision 1 by Claude Code, for Ada, "),
        "{listed}"
    );
    assert!(listed.contains("not read yet"));

    let read = tools
        .call("read_paper", json!({"address": "p/notes"}), None)
        .await
        .unwrap();
    assert!(read.starts_with("# p/notes · Notes\n\nRevision 1, by Claude Code, for Ada"));
    assert!(read.contains("Sections: #setting, #open\n"), "{read}");
    let section = tools
        .call("read_thread", json!({"address": "[[p/notes#open]]"}), None)
        .await
        .unwrap();
    assert!(
        section.ends_with("Section p/notes#open:\n\n# Open\n\n> [!open]\n> Is it proper?\n"),
        "{section}"
    );
    let nowhere = tools
        .call("read_paper", json!({"address": "p/notes#nowhere"}), None)
        .await
        .unwrap_err();
    assert!(nowhere.to_string().contains("#setting, #open"));

    // A second revision; and an edit over the first is refused.
    let second = body.replace("Is it proper?", "It is proper: [[t1#4]].");
    tools
        .call(
            "propose_paper_edit",
            json!({"paper": "p/notes", "body": second, "reason": "Answered", "base_revision": 1}),
            Some("Claude Code"),
        )
        .await
        .unwrap();
    let stale = tools
        .call(
            "propose_paper_edit",
            json!({"paper": "p/notes", "body": "x", "reason": "y", "base_revision": 1}),
            Some("Claude Code"),
        )
        .await
        .unwrap_err();
    assert!(stale.to_string().contains("revision 2"), "{stale}");

    let changes = tools
        .call(
            "paper_changes",
            json!({"paper": "p/notes", "since": 1}),
            None,
        )
        .await
        .unwrap();
    assert!(
        changes.contains("- Revision 2, by Claude Code, for Ada, "),
        "{changes}"
    );
    assert!(changes.contains(": Answered\n"));
    assert!(changes.contains("- > Is it proper?\n+ > It is proper: [[t1#4]].\n"));
    let never = tools
        .call("paper_changes", json!({"paper": "p/notes"}), None)
        .await
        .unwrap();
    assert!(never.contains("You have not read p/notes yet"), "{never}");
    let all = tools.call("paper_changes", json!({}), None).await.unwrap();
    assert!(all.contains("Revision 2; you have not read it."), "{all}");
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

    let old = tools
        .call(
            "read_paper",
            json!({"address": "p/notes", "revision": 1}),
            None,
        )
        .await
        .unwrap();
    assert!(old.contains("Revision 1 of 2"), "{old}");
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
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(
        names,
        [
            "list_threads",
            "read_thread",
            "post_message",
            "search",
            "list_papers",
            "read_paper",
            "paper_changes",
            "propose_paper_edit",
            "library_state",
            "library_lookup",
            "library_search"
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
            .all(|t| t.name != "post_message" && t.name != "propose_paper_edit")
    );
    client.cancel().await.unwrap();
}
