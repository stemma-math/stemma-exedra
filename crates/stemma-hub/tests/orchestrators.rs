include!("support/workshop.rs");

async fn setup(model: &stemma_fake_model::FakeModel) -> (Hub, String) {
    let hub = hub(Some(model)).await;
    let ada = person(&hub, "ada", true).await;
    group(&hub, &ada).await;
    (hub, ada)
}
async fn key(hub: &Hub, ada: &str, provider: &str, cap: i64) -> Value {
    call(hub,ada,"PUT","/groups/geometry/workshop/model",Some(json!({"provider":provider,"model":"scripted","api_key":"fake-dedicated-key-1234","monthly_cap":cap,"step_cap":20}))).await
}
async fn idle(hub: &Hub) {
    for _ in 0..300 {
        let thinking: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM workshop_requests
             WHERE thinking = 1",
        )
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
        if thinking == 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("The orchestrator did not become idle");
}
fn launch() -> Value {
    json!({"recipe":"prove","inputs":{"statement":"True"},"runner":"codex","assignment":{"kind":"mine"}})
}
async fn free(hub: &Hub, ada: &str) -> Value {
    let mut ask = ask_body("", json!({"kind":"mine"}));
    ask["inputs"] = json!({});
    call(
        hub,
        ada,
        "POST",
        "/groups/geometry/workshop/requests",
        Some(ask),
    )
    .await
}
#[tokio::test(flavor = "multi_thread")]
async fn encrypted_write_only_keys_provider_dialects_and_caps() {
    for provider in ["anthropic", "openai", "openrouter"] {
        let model = stemma_fake_model::FakeModel::start().await;
        let (hub, ada) = setup(&model).await;
        let masked = key(&hub, &ada, provider, 15).await;
        assert_eq!(masked["last_four"], "1234");
        assert_eq!(masked["set_by"]["username"], "ada");
        assert!(!masked.to_string().contains("fake-dedicated"));
        let encrypted: Vec<u8> = sqlx::query_scalar(
            "SELECT encrypted_key
             FROM workshop_models",
        )
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
        assert!(!String::from_utf8_lossy(&encrypted).contains("fake-dedicated"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(hub._data.path().join("workshop.key"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        // A step that reads goes on to another, which the cap stops.
        model.tool("recipes", json!({}));
        free(&hub, &ada).await;
        idle(&hub).await;
        assert_eq!(model.requests().len(), 1, "the cap must stop the next step");
        let sent = &model.requests()[0];
        if provider == "anthropic" {
            assert!(sent["system"].is_string());
            assert!(sent["tools"][0]["input_schema"].is_object());
        } else {
            assert_eq!(sent["messages"][0]["role"], "system");
            assert!(sent["tools"][0]["function"]["parameters"].is_object());
        }
        let found = call(&hub, &ada, "GET", "/groups/geometry/workshop/status", None).await;
        assert_eq!(found["model"]["input_tokens"], 10);
        assert_eq!(found["model"]["output_tokens"], 5);
        assert_eq!(found["model"]["stopped"], true);
        let thread = call(&hub, &ada, "GET", "/groups/geometry/threads/1", None).await;
        let messages = thread["messages"].as_array().unwrap();
        assert!(
            messages
                .iter()
                .any(|m| m["body"].as_str().unwrap().contains("monthly token cap"))
        );
        assert!(
            messages
                .iter()
                .all(|m| m["author"]["username"] == "workshop")
        );
        assert!(messages.iter().all(|m| m["agent"].is_null()));
        let id: i64 = sqlx::query_scalar(
            "SELECT id
             FROM groups",
        )
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
        // The encrypted value is bound to its group, and survives reopening the hub.
        let reopened =
            stemma_hub::model::Key::load_or_create(&hub._data.path().join("workshop.key")).unwrap();
        assert_ne!(
            reopened.encrypt(id, "same key").unwrap(),
            reopened.encrypt(id, "same key").unwrap()
        );
        assert_eq!(
            status(
                &hub,
                &ada,
                "PATCH",
                "/api/v1/groups/geometry/threads/1/messages/1",
                json!({"body":"Impersonated"})
            )
            .await,
            403
        );
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn free_requests_launch_and_only_requester_or_admin_messages_wake_them() {
    let model = stemma_fake_model::FakeModel::start().await;
    let (hub, ada) = setup(&model).await;
    key(&hub, &ada, "openai", 10000).await;
    let bob = person(&hub, "bob", false).await;
    sqlx::query(
        "INSERT INTO memberships(group_id,user_id,role,joined_at)
         SELECT g.id,u.id,'member',0
         FROM groups g,users u
         WHERE u.username = 'bob'",
    )
    .execute(&hub.state.db)
    .await
    .unwrap();
    let board = call(&hub, &bob, "GET", "/groups/geometry/workshop/status", None).await;
    assert_eq!(board["model"]["has_key"], true);
    assert_eq!(board["model"]["last_four"], Value::Null);
    assert_eq!(board["model"]["set_by"], Value::Null);
    let mut ask = ask_body("", json!({"kind":"mine"}));
    ask["inputs"] = json!({"statement": "True"});
    let refused = status(
        &hub,
        &bob,
        "POST",
        "/api/v1/groups/geometry/workshop/requests",
        ask,
    )
    .await;
    assert_eq!(refused, 422);
    let mut socket = machine(&hub, &ada).await;
    model.tool("launch", launch());
    free(&hub, &ada).await;
    idle(&hub).await;
    let HubMessage::ProcessOffer(offer) = next(&mut socket).await else {
        panic!("recipe offer");
    };
    assert_eq!(offer.recipe.name, "prove");
    assert_eq!(offer.inputs["statement"], "True");
    assert!(
        !serde_json::to_string(&offer)
            .unwrap()
            .contains("fake-dedicated")
    );
    call(
        &hub,
        &bob,
        "POST",
        "/groups/geometry/threads/1/messages",
        Some(json!({"body":"A member discusses."})),
    )
    .await;
    idle(&hub).await;
    assert_eq!(model.requests().len(), 1);
    model.text("Waiting for the process to finish.");
    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/threads/1/messages",
        Some(json!({"body":"How is it going?"})),
    )
    .await;
    idle(&hub).await;
    assert_eq!(model.requests().len(), 2);
    send(
        &mut socket,
        WorkerMessage::ProcessDecision {
            process: offer.process,
            accept: true,
            pending: false,
        },
    )
    .await;
    let HubMessage::Process(spec) = next(&mut socket).await else {
        panic!("process");
    };
    model.tool("close",json!({"done":false,"message":"The runner failed before making a change. Please try again."}));
    send(
        &mut socket,
        WorkerMessage::ProcessFinished {
            lease: spec.lease,
            result: Box::new(ProcessResult {
                error: Some("Runner failed".into()),
                ..Default::default()
            }),
        },
    )
    .await;
    for _ in 0..100 {
        stemma_hub::orchestrator::dispatch(&hub.state)
            .await
            .unwrap();
        idle(&hub).await;
        let req = call(
            &hub,
            &ada,
            "GET",
            "/groups/geometry/workshop/requests/1",
            None,
        )
        .await;
        if req["state"] == "failed" {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("failure should wake the orchestrator and close with a reason");
}
#[tokio::test(flavor = "multi_thread")]
async fn no_key_and_provider_errors_are_explicit_and_never_echo_keys() {
    let model = stemma_fake_model::FakeModel::start().await;
    let (hub, ada) = setup(&model).await;
    free(&hub, &ada).await;
    idle(&hub).await;
    assert!(model.requests().is_empty());
    let req = call(
        &hub,
        &ada,
        "GET",
        "/groups/geometry/workshop/requests/1",
        None,
    )
    .await;
    assert!(req["waiting"].as_str().unwrap().contains("API key"));
    model.reply(
        axum::http::StatusCode::UNAUTHORIZED,
        json!({"error":{"message":"No access with fake-dedicated-key-1234"}}),
    );
    key(&hub, &ada, "openrouter", 1000).await;
    stemma_hub::orchestrator::dispatch(&hub.state)
        .await
        .unwrap();
    idle(&hub).await;
    let req = call(
        &hub,
        &ada,
        "GET",
        "/groups/geometry/workshop/requests/1",
        None,
    )
    .await;
    assert!(
        req["waiting"]
            .as_str()
            .unwrap()
            .contains("openrouter refused")
    );
    assert!(!req.to_string().contains("fake-dedicated"));
    let retry = || {
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT retry_at
             FROM workshop_requests",
        )
        .fetch_one(&hub.state.db)
    };
    // A refused key waits for an admin; a provider that keeps failing passes.
    assert_eq!(retry().await.unwrap(), None);
    for _ in 0..4 {
        model.reply(
            axum::http::StatusCode::BAD_GATEWAY,
            json!({"error":{"message":"Upstream unavailable"}}),
        );
    }
    key(&hub, &ada, "openrouter", 1000).await;
    stemma_hub::orchestrator::dispatch(&hub.state)
        .await
        .unwrap();
    idle(&hub).await;
    assert!(retry().await.unwrap().is_some());
    let req = call(
        &hub,
        &ada,
        "GET",
        "/groups/geometry/workshop/requests/1",
        None,
    )
    .await;
    assert!(
        req["waiting"]
            .as_str()
            .unwrap()
            .ends_with("Trying again in a minute")
    );
    // OpenRouter reports a failure upstream inside a 200.
    model.reply(
        axum::http::StatusCode::OK,
        json!({"error":{"message":"Upstream overloaded"}}),
    );
    sqlx::query("UPDATE workshop_requests SET retry_at = 0")
        .execute(&hub.state.db)
        .await
        .unwrap();
    stemma_hub::orchestrator::dispatch(&hub.state)
        .await
        .unwrap();
    idle(&hub).await;
    let req = call(
        &hub,
        &ada,
        "GET",
        "/groups/geometry/workshop/requests/1",
        None,
    )
    .await;
    assert!(
        req["waiting"]
            .as_str()
            .unwrap()
            .contains("failed while answering: Upstream overloaded")
    );
}
async fn opened(
    hub: &Hub,
    ada: &str,
    model: &stemma_fake_model::FakeModel,
    socket: &mut Socket,
) -> ProcessSpec {
    model.tool("launch", launch());
    free(hub, ada).await;
    idle(hub).await;
    let offer = loop {
        if let HubMessage::ProcessOffer(offer) = next(socket).await {
            break offer;
        }
    };
    send(
        socket,
        WorkerMessage::ProcessDecision {
            process: offer.process,
            accept: true,
            pending: false,
        },
    )
    .await;
    let spec = loop {
        if let HubMessage::Process(spec) = next(socket).await {
            break *spec;
        }
    };
    let temp = tempfile::tempdir().unwrap();
    let repo = &spec.repositories[0];
    assert!(
        git(
            temp.path(),
            &spec.token,
            &["clone", "-q", &format!("{}{}", hub.url, repo.path), "work"]
        )
        .status
        .success()
    );
    let dir = temp.path().join("work");
    assert!(
        git(
            &dir,
            &spec.token,
            &["checkout", "-qb", &repo.branch, &repo.base]
        )
        .status
        .success()
    );
    let module = format!("Proof{}", spec.offer.process);
    std::fs::write(
        dir.join(format!("{module}.lean")),
        format!(
            "theorem proof_{} : True := by trivial\n",
            spec.offer.process
        ),
    )
    .unwrap();
    assert!(git(&dir, &spec.token, &["add", "."]).status.success());
    assert!(
        git(&dir, &spec.token, &["commit", "-qm", "Prove True"])
            .status
            .success()
    );
    let push = git(
        &dir,
        &spec.token,
        &["push", "origin", &format!("HEAD:{}", repo.branch)],
    );
    assert!(
        push.status.success(),
        "{}",
        String::from_utf8_lossy(&push.stderr)
    );
    finish_process(hub, socket, &spec, None).await;
    spec
}
async fn finish_process(hub: &Hub, socket: &mut Socket, spec: &ProcessSpec, error: Option<String>) {
    send(
        socket,
        WorkerMessage::ProcessFinished {
            lease: spec.lease,
            result: Box::new(ProcessResult {
                summary: "Process complete.".into(),
                error,
                ..Default::default()
            }),
        },
    )
    .await;
    for _ in 0..200 {
        let done: bool = sqlx::query_scalar(
            "SELECT finished_at IS NOT NULL
             FROM workshop_processes
             WHERE id = ?",
        )
        .bind(spec.offer.process)
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
        if done {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("process did not finish");
}
async fn passing(hub: &Hub, review: i64, sorry: bool, module: &str) {
    stemma_hub::review::tick(&hub.state).await.unwrap();
    let work: i64 = sqlx::query_scalar(
        "SELECT build_work
         FROM workshop_reviews
         WHERE id = ?",
    )
    .bind(review)
    .fetch_one(&hub.state.db)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE work_leases
         SET state = 'passed'
         WHERE id = ?",
    )
    .bind(work)
    .execute(&hub.state.db)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE library_builds
         SET succeeded = 1, finished_at = 1
         WHERE work_id = ?",
    )
    .bind(work)
    .execute(&hub.state.db)
    .await
    .unwrap();
    sqlx::query("INSERT INTO library_declarations (build_id,name,module,kind,type,sorry,data_sorry,axioms,uses)
                 SELECT id, ?, ?, 'theorem', 'True', ?, 0, ?, '[]'
                 FROM library_builds
                 WHERE work_id = ?")
        .bind(format!("{module}.proof")).bind(module).bind(sorry).bind(if sorry { "[\"sorryAx\"]" } else { "[]" }).bind(work).execute(&hub.state.db).await.unwrap();
}
async fn review_state(hub: &Hub, review: i64) -> String {
    sqlx::query_scalar(
        "SELECT state
         FROM workshop_reviews
         WHERE id = ?",
    )
    .bind(review)
    .fetch_one(&hub.state.db)
    .await
    .unwrap()
}
#[tokio::test(flavor = "multi_thread")]
async fn a_built_pull_merges_as_workshop_and_queues_mains_build() {
    let model = stemma_fake_model::FakeModel::start().await;
    let (hub, ada) = setup(&model).await;
    key(&hub, &ada, "openai", 10000).await;
    let mut socket = machine(&hub, &ada).await;
    let spec = opened(&hub, &ada, &model, &mut socket).await;
    passing(&hub, 1, false, &format!("Proof{}", spec.offer.process)).await;
    model.tool(
        "decide",
        json!({"decision":"merge","message":"The proof is correct and the head builds."}),
    );
    stemma_hub::review::tick(&hub.state).await.unwrap();
    assert_eq!(review_state(&hub, 1).await, "merged");
    let notices = call(&hub, &ada, "GET", "/me/notifications", None).await;
    assert!(notices.to_string().contains("review_merged"));
    let pulls = hub._forge.pulls("geometry", "library");
    assert!(pulls[0].merged.is_some());
    let commit = pulls[0].merged.as_ref().unwrap();
    let queued: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1
         FROM work_leases
         WHERE commit_sha = ? AND review_build = 0 AND state = 'queued')",
    )
    .bind(commit)
    .fetch_one(&hub.state.db)
    .await
    .unwrap();
    assert!(queued);
    // The recipe review, not process completion, finishes the request.
    model.tool(
        "close",
        json!({"done":true,"message":"The proof was reviewed and merged into main."}),
    );
    stemma_hub::orchestrator::dispatch(&hub.state)
        .await
        .unwrap();
    idle(&hub).await;
    let request = call(
        &hub,
        &ada,
        "GET",
        "/groups/geometry/workshop/requests/1",
        None,
    )
    .await;
    assert_eq!(request["state"], "done");
}
#[tokio::test(flavor = "multi_thread")]
async fn sorry_requires_the_recipes_explicit_permission() {
    for accept in [false, true] {
        let model = stemma_fake_model::FakeModel::start().await;
        let (hub, ada) = setup(&model).await;
        key(&hub, &ada, "anthropic", 10000).await;
        let mut socket = machine(&hub, &ada).await;
        let spec = opened(&hub, &ada, &model, &mut socket).await;
        if accept {
            let raw: String = sqlx::query_scalar(
                "SELECT recipe
                 FROM workshop_processes
                 WHERE id = ?",
            )
            .bind(spec.offer.process)
            .fetch_one(&hub.state.db)
            .await
            .unwrap();
            let mut recipe: Value = serde_json::from_str(&raw).unwrap();
            recipe["recipe"]["front"]["accept_sorry"] = json!(true);
            sqlx::query(
                "UPDATE workshop_processes
                 SET recipe = ?
                 WHERE id = ?",
            )
            .bind(recipe.to_string())
            .bind(spec.offer.process)
            .execute(&hub.state.db)
            .await
            .unwrap();
            model.tool("decide",json!({"decision":"merge","message":"The admitted result is useful for this recipe, which permits sorry."}));
        }
        passing(&hub, 1, true, &format!("Proof{}", spec.offer.process)).await;
        stemma_hub::review::tick(&hub.state).await.unwrap();
        assert_eq!(
            review_state(&hub, 1).await,
            if accept { "merged" } else { "refused" }
        );
    }
}
async fn rebase_offer(socket: &mut Socket) -> ProcessSpec {
    let offer = loop {
        if let HubMessage::ProcessOffer(offer) = next(socket).await {
            break offer;
        }
    };
    assert_eq!(offer.recipe.name, "rebase");
    send(
        socket,
        WorkerMessage::ProcessDecision {
            process: offer.process,
            accept: true,
            pending: false,
        },
    )
    .await;
    loop {
        if let HubMessage::Process(spec) = next(socket).await {
            return *spec;
        }
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn racing_pulls_are_serialized_and_the_second_rebases_on_its_original_machine() {
    let model = stemma_fake_model::FakeModel::start().await;
    let (hub, ada) = setup(&model).await;
    key(&hub, &ada, "openrouter", 10000).await;
    let mut socket = machine(&hub, &ada).await;
    let first = opened(&hub, &ada, &model, &mut socket).await;
    let second = opened(&hub, &ada, &model, &mut socket).await;
    passing(&hub, 1, false, &format!("Proof{}", first.offer.process)).await;
    assert_eq!(review_state(&hub, 2).await, "queued");
    model.tool(
        "decide",
        json!({"decision":"merge","message":"Merge the first proof."}),
    );
    stemma_hub::review::tick(&hub.state).await.unwrap();
    passing(&hub, 2, false, &format!("Proof{}", second.offer.process)).await;
    stemma_hub::review::tick(&hub.state).await.unwrap();
    assert_eq!(review_state(&hub, 2).await, "rebasing");
    let rebase = rebase_offer(&mut socket).await;
    let repo = &rebase.repositories[0];
    assert!(repo.force_push);
    assert_eq!(repo.branch, second.repositories[0].branch);
    let temp = tempfile::tempdir().unwrap();
    assert!(
        git(
            temp.path(),
            &rebase.token,
            &["clone", "-q", &format!("{}{}", hub.url, repo.path), "work"]
        )
        .status
        .success()
    );
    let dir = temp.path().join("work");
    assert!(
        git(
            &dir,
            &rebase.token,
            &["checkout", "-qb", &repo.branch, &repo.base]
        )
        .status
        .success()
    );
    assert!(
        git(&dir, &rebase.token, &["rebase", "origin/main"])
            .status
            .success()
    );
    let pushed = git(
        &dir,
        &rebase.token,
        &[
            "push",
            "--force-with-lease",
            "origin",
            &format!("HEAD:{}", repo.branch),
        ],
    );
    assert!(
        pushed.status.success(),
        "{}",
        String::from_utf8_lossy(&pushed.stderr)
    );
    assert!(
        !git(&dir, &rebase.token, &["push", "origin", "HEAD:main"])
            .status
            .success()
    );
    finish_process(&hub, &mut socket, &rebase, None).await;
    passing(&hub, 2, false, &format!("Proof{}", second.offer.process)).await;
    model.tool(
        "decide",
        json!({"decision":"merge","message":"The rebased proof builds on current main."}),
    );
    stemma_hub::review::tick(&hub.state).await.unwrap();
    assert_eq!(review_state(&hub, 2).await, "merged");
    assert_eq!(hub._forge.pulls("geometry", "library").len(), 2);
    assert!(
        hub._forge
            .pulls("geometry", "library")
            .iter()
            .all(|p| p.merged.is_some())
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn the_third_unsuccessful_rebase_refuses() {
    let model = stemma_fake_model::FakeModel::start().await;
    let (hub, ada) = setup(&model).await;
    key(&hub, &ada, "openai", 10000).await;
    let mut socket = machine(&hub, &ada).await;
    let original = opened(&hub, &ada, &model, &mut socket).await;
    passing(&hub, 1, false, &format!("Proof{}", original.offer.process)).await;
    model.tool(
        "decide",
        json!({"decision":"rebase","message":"Rework this branch on main."}),
    );
    stemma_hub::review::tick(&hub.state).await.unwrap();
    for _ in 0..3 {
        let spec = rebase_offer(&mut socket).await;
        finish_process(
            &hub,
            &mut socket,
            &spec,
            Some("Could not resolve the conflict".into()),
        )
        .await;
        stemma_hub::review::tick(&hub.state).await.unwrap();
    }
    assert_eq!(review_state(&hub, 1).await, "refused");
    let rebases: i64 = sqlx::query_scalar(
        "SELECT rebases
         FROM workshop_reviews
         WHERE id = 1",
    )
    .fetch_one(&hub.state.db)
    .await
    .unwrap();
    assert_eq!(rebases, 3);
    assert!(hub._forge.pulls("geometry", "library")[0].merged.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn rate_limits_and_server_errors_back_off_without_counting_usage_twice() {
    let model = stemma_fake_model::FakeModel::start().await;
    let (hub, ada) = setup(&model).await;
    key(&hub, &ada, "openai", 1000).await;
    model.reply(
        axum::http::StatusCode::TOO_MANY_REQUESTS,
        json!({"error": {"message": "Slow down"}}),
    );
    model.reply(
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        json!({"error": {"message": "Try again"}}),
    );
    model.tool(
        "close",
        json!({"done": false, "message": "This request needs a statement to prove."}),
    );
    free(&hub, &ada).await;
    idle(&hub).await;
    assert_eq!(model.requests().len(), 3);
    let board = call(&hub, &ada, "GET", "/groups/geometry/workshop/status", None).await;
    assert_eq!(board["model"]["input_tokens"], 10);
    assert_eq!(board["model"]["output_tokens"], 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_process_about_to_expire_wakes_once() {
    let model = stemma_fake_model::FakeModel::start().await;
    let (hub, ada) = setup(&model).await;
    key(&hub, &ada, "openai", 1000).await;
    model.tool("launch", launch());
    free(&hub, &ada).await;
    idle(&hub).await;
    sqlx::query(
        "UPDATE workshop_processes
         SET expires_at = ?",
    )
    .bind(stemma_hub::db::now() + 30_000)
    .execute(&hub.state.db)
    .await
    .unwrap();
    model.text("The offer is about to expire; waiting for the requester to connect a machine.");
    for _ in 0..2 {
        stemma_hub::orchestrator::dispatch(&hub.state)
            .await
            .unwrap();
        idle(&hub).await;
    }
    assert_eq!(model.requests().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn thinking_cannot_launch_a_scheduled_recipe_early() {
    let model = stemma_fake_model::FakeModel::start().await;
    let (hub, ada) = setup(&model).await;
    key(&hub, &ada, "openai", 1000).await;
    model.tool("launch", launch());
    model.text("Waiting until the requested time.");
    let mut ask = ask_body("prove", json!({"kind":"mine"}));
    ask["schedule"]["at"] = json!(stemma_hub::db::now() + 3_600_000);
    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/workshop/requests",
        Some(ask),
    )
    .await;
    idle(&hub).await;
    let req = call(
        &hub,
        &ada,
        "GET",
        "/groups/geometry/workshop/requests/1",
        None,
    )
    .await;
    assert!(req["processes"].as_array().unwrap().is_empty());
    assert_eq!(req["waiting"], "Waiting for the scheduled time");
}
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_posts_once() {
    let model = stemma_fake_model::FakeModel::start().await;
    let (hub, ada) = setup(&model).await;
    key(&hub, &ada, "openrouter", 1_000_000).await;
    model.tool("post", json!({"message":"Waiting for a machine."}));
    model.tool("post", json!({"message":"I have posted here."}));
    model.text("Still waiting for a machine.");
    free(&hub, &ada).await;
    idle(&hub).await;
    assert_eq!(model.requests().len(), 1, "posting ends the turn");
    let thread = call(&hub, &ada, "GET", "/groups/geometry/threads/1", None).await;
    let posts: Vec<&str> = thread["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["author"]["username"] == "workshop")
        .map(|m| m["body"].as_str().unwrap())
        .collect();
    assert_eq!(posts, ["Waiting for a machine."]);
}
