include!("support/workshop.rs");

#[tokio::test(flavor = "multi_thread")]
async fn processes_push_only_their_branch_post_only_in_their_request_and_open_as_worker() {
    let hub = hub(None).await;
    let ada = person(&hub, "ada", true).await;
    group(&hub, &ada).await;
    let recipes = call(&hub, &ada, "GET", "/groups/geometry/workshop/recipes", None).await;
    assert_eq!(recipes.as_array().unwrap().len(), 3);
    assert_eq!(recipes[0]["version"].as_str().unwrap().len(), 40);
    let mut socket = machine(&hub, &ada).await;
    let asked = call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/workshop/requests",
        Some(ask_body("prove", json!({"kind":"mine"}))),
    )
    .await;
    assert_eq!(asked["thread"]["kind"], "request");
    let HubMessage::ProcessOffer(offer) = next(&mut socket).await else {
        panic!("offer")
    };
    assert!(serde_json::to_value(&offer).unwrap().get("token").is_none());
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
        panic!("lease")
    };
    let token = &spec.token;
    let (_, answer_token) = stemma_hub::auth::create_job_token(
        &hub.state.db,
        spec.owner.id,
        spec.offer.group.id,
        "Codex",
        stemma_hub::db::now() + 60_000,
        Default::default(),
    )
    .await
    .unwrap();
    assert!(
        status(
            &hub,
            &answer_token,
            "GET",
            "/api/v1/groups/geometry/workshop/recipes",
            Value::Null
        )
        .await
        .is_success()
    );
    // An answer asks for work, later so that no offer comes now; the new
    // request says who asked. A process never asks.
    let mut later = ask_body("prove", json!({"kind":"mine"}));
    later["schedule"]["at"] = json!(stemma_hub::db::now() + 86_400_000);
    let asked = call(
        &hub,
        &answer_token,
        "POST",
        "/groups/geometry/workshop/requests",
        Some(later.clone()),
    )
    .await;
    let number = asked["thread"]["number"].as_u64().unwrap();
    let opened = call(
        &hub,
        &ada,
        "GET",
        &format!("/groups/geometry/threads/{number}"),
        None,
    )
    .await;
    assert_eq!(opened["messages"][0]["agent"], "Codex");
    assert_eq!(opened["messages"][0]["body"], "Asked by Codex, for ada.");
    assert_eq!(
        status(
            &hub,
            token,
            "POST",
            "/api/v1/groups/geometry/workshop/requests",
            later
        )
        .await,
        403
    );
    assert_eq!(
        status(
            &hub,
            &answer_token,
            "POST",
            "/api/v1/groups/geometry/workshop/requests/1",
            json!({"action":"cancel"})
        )
        .await,
        403
    );
    let guest = person(&hub, "guest", false).await;
    sqlx::query(
        "INSERT INTO memberships(group_id, user_id, role, joined_at)
         SELECT g.id, u.id, 'guest', 0
         FROM groups g, users u
         WHERE u.username = 'guest'",
    )
    .execute(&hub.state.db)
    .await
    .unwrap();
    assert!(
        status(
            &hub,
            &guest,
            "GET",
            "/api/v1/groups/geometry/workshop/requests/1",
            Value::Null
        )
        .await
        .is_success()
    );
    assert_eq!(
        status(
            &hub,
            &guest,
            "POST",
            "/api/v1/groups/geometry/workshop/requests",
            ask_body("prove", json!({"kind":"mine"}))
        )
        .await,
        403
    );
    assert_eq!(
        status(
            &hub,
            &guest,
            "POST",
            "/api/v1/groups/geometry/threads/1/messages",
            json!({"body":"No"})
        )
        .await,
        403
    );
    assert_eq!(
        spec.repositories[0].branch,
        format!("workshop/1-{}", offer.process)
    );
    assert!(
        status(&hub, token, "GET", "/api/v1/groups/geometry", Value::Null)
            .await
            .is_success()
    );
    for (method, path) in [
        ("GET", "/api/v1/me"),
        ("POST", "/api/v1/groups/geometry/threads"),
        ("POST", "/api/v1/groups/geometry/notes"),
        ("POST", "/api/v1/groups/geometry/workshop/requests"),
    ] {
        assert_eq!(
            status(&hub, token, method, path, json!({})).await,
            403,
            "{path}"
        );
    }
    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/threads",
        Some(json!({"title":"Another thread"})),
    )
    .await;
    assert_eq!(
        status(
            &hub,
            token,
            "POST",
            "/api/v1/groups/geometry/threads/2/messages",
            json!({"body":"No"})
        )
        .await,
        403
    );
    let session_post = call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/threads/1/messages",
        Some(json!({"body":"A session's answer","agent":"Codex"})),
    )
    .await;
    assert_eq!(session_post["workshop"], false);
    let posted = call(
        &hub,
        token,
        "POST",
        "/groups/geometry/threads/1/messages",
        Some(json!({"body":format!("Working on the proof; token {token}")})),
    )
    .await;
    assert_eq!(posted["workshop"], true);
    assert!(!posted["body"].as_str().unwrap().contains(token));
    assert!(posted["body"].as_str().unwrap().contains("[hidden]"));
    assert_eq!(posted["author"]["username"], "ada");
    let temp = tempfile::tempdir().unwrap();
    let repo = &spec.repositories[0];
    assert!(
        git(
            temp.path(),
            token,
            &["clone", "-q", &format!("{}{}", hub.url, repo.path), "work"]
        )
        .status
        .success()
    );
    let dir = temp.path().join("work");
    assert!(
        git(&dir, token, &["checkout", "-qb", &repo.branch, &repo.base])
            .status
            .success()
    );
    std::fs::write(
        dir.join("Proof.lean"),
        "theorem workshop_test : True := by trivial\n",
    )
    .unwrap();
    assert!(git(&dir, token, &["add", "Proof.lean"]).status.success());
    assert!(
        git(&dir, token, &["commit", "-qm", "Prove True"])
            .status
            .success()
    );
    for reference in ["main", "other", "refs/tags/test", "refs/notes/test"] {
        assert!(
            !git(
                &dir,
                token,
                &["push", "origin", &format!("HEAD:{reference}")]
            )
            .status
            .success(),
            "{reference}"
        );
    }
    let pushed = git(
        &dir,
        token,
        &["push", "origin", &format!("HEAD:{}", repo.branch)],
    );
    assert!(
        pushed.status.success(),
        "{}",
        String::from_utf8_lossy(&pushed.stderr)
    );
    send(
        &mut socket,
        WorkerMessage::ProcessFinished {
            lease: spec.lease,
            result: Box::new(ProcessResult {
                summary: "Proved True and checked the proof.".into(),
                usage: Usage {
                    input_tokens: Some(100),
                    output_tokens: Some(20),
                    ..Default::default()
                },
                ..Default::default()
            }),
        },
    )
    .await;
    for _ in 0..100 {
        let found = call(
            &hub,
            &ada,
            "GET",
            "/groups/geometry/workshop/requests/1",
            None,
        )
        .await;
        if found["state"] == "done" {
            assert!(found["processes"][0]["branches"][0]["pull_request"].is_string());
            assert_eq!(found["processes"][0]["usage"]["input_tokens"], 100);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(hub._forge.pulls("geometry", "library").len(), 1);
    let pull: Value = reqwest::Client::new()
        .get(format!(
            "{}/api/v1/repos/geometry/library/pulls/1",
            hub._forge.url
        ))
        .header(
            "authorization",
            format!("token {}", stemma_fake_forge::TOKEN),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(pull["user"]["login"], "worker");
    assert_eq!(
        status(&hub, token, "GET", "/api/v1/groups/geometry", Value::Null).await,
        401
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_offers_first_acceptance_recurring_slots_and_expiry_are_durable() {
    let hub = hub(None).await;
    let ada = person(&hub, "ada", true).await;
    group(&hub, &ada).await;
    let bob = person(&hub, "bob", false).await;
    sqlx::query(
        "INSERT INTO memberships(group_id, user_id, role, joined_at)
         SELECT g.id, u.id, 'member', 0
         FROM groups g, users u
         WHERE u.username = 'bob'",
    )
    .execute(&hub.state.db)
    .await
    .unwrap();
    let bob_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username='bob'")
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    let mut a = machine(&hub, &ada).await;
    let mut b = machine(&hub, &bob).await;
    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/workshop/requests",
        Some(ask_body(
            "review-paper",
            json!({"kind":"member","user":bob_id}),
        )),
    )
    .await;
    let HubMessage::ProcessOffer(offer) = next(&mut b).await else {
        panic!("Bob's offer")
    };
    send(
        &mut b,
        WorkerMessage::ProcessDecision {
            process: offer.process,
            accept: false,
            pending: true,
        },
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let waiting = call(
        &hub,
        &ada,
        "GET",
        "/groups/geometry/workshop/requests/1",
        None,
    )
    .await;
    assert_eq!(waiting["state"], "waiting");
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE kind='process_approval' AND user_id=?",
    )
    .bind(bob_id)
    .fetch_one(&hub.state.db)
    .await
    .unwrap();
    assert_eq!(count, 1);
    send(
        &mut b,
        WorkerMessage::ProcessDecision {
            process: offer.process,
            accept: true,
            pending: false,
        },
    )
    .await;
    let HubMessage::Process(spec) = next(&mut b).await else {
        panic!("Bob's lease")
    };
    assert_eq!(spec.owner.id, bob_id);
    assert!(spec.repositories.is_empty());

    send(
        &mut b,
        WorkerMessage::ProcessFinished {
            lease: spec.lease,
            result: Box::new(ProcessResult {
                summary: "Reviewed the paper.".into(),
                ..Default::default()
            }),
        },
    )
    .await;
    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/workshop/requests",
        Some(ask_body("review-paper", json!({"kind":"anyone"}))),
    )
    .await;
    let HubMessage::ProcessOffer(offer_a) = next(&mut a).await else {
        panic!("Ada's anyone offer")
    };
    let HubMessage::ProcessOffer(offer_b) = next(&mut b).await else {
        panic!("Bob's anyone offer")
    };
    assert_eq!(offer_a.process, offer_b.process);
    send(
        &mut b,
        WorkerMessage::ProcessDecision {
            process: offer_b.process,
            accept: true,
            pending: false,
        },
    )
    .await;
    let HubMessage::Process(spec) = next(&mut b).await else {
        panic!("first accepted")
    };
    assert_eq!(spec.owner.id, bob_id);
    assert!(matches!(
        next(&mut a).await,
        HubMessage::ProcessTaken { .. }
    ));
    send(
        &mut a,
        WorkerMessage::ProcessDecision {
            process: offer_a.process,
            accept: true,
            pending: false,
        },
    )
    .await;
    assert!(matches!(
        next(&mut a).await,
        HubMessage::ProcessTaken { .. }
    ));
    // A series' first slot is created once; two simultaneous hub tasks cannot replay it.
    let mut recurring = ask_body("review-paper", json!({"kind":"mine"}));
    recurring["schedule"] = json!({"timezone":"Europe/Madrid","every":{"kind":"hours","hours":1}});
    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/workshop/requests",
        Some(recurring),
    )
    .await;
    let (x, y) = tokio::join!(
        stemma_hub::workshop::tick(&hub.state),
        stemma_hub::workshop::tick(&hub.state)
    );
    x.unwrap();
    y.unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM workshop_processes
         WHERE thread_id = (SELECT id FROM threads WHERE number = 3)",
    )
    .fetch_one(&hub.state.db)
    .await
    .unwrap();
    assert_eq!(count, 1);
    sqlx::query(
        "UPDATE workshop_requests
         SET next_at = ?
         WHERE thread_id = (SELECT id FROM threads WHERE number = 3)",
    )
    .bind(stemma_hub::db::now() - 10_800_000)
    .execute(&hub.state.db)
    .await
    .unwrap();
    stemma_hub::workshop::tick(&hub.state).await.unwrap();
    stemma_hub::workshop::tick(&hub.state).await.unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM workshop_processes
         WHERE thread_id = (SELECT id FROM threads WHERE number = 3)",
    )
    .fetch_one(&hub.state.db)
    .await
    .unwrap();
    assert_eq!(count, 2);
    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/workshop/requests/3",
        Some(json!({"action":"pause"})),
    )
    .await;
    // Queue expiration posts and notifies, even when no eligible machine exists.
    let mut expires = ask_body("review-paper", json!({"kind":"mine"}));
    expires["model"] = json!("unavailable-model");
    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/workshop/requests",
        Some(expires),
    )
    .await;
    sqlx::query(
        "UPDATE workshop_processes
         SET expires_at = 0
         WHERE thread_id = (SELECT id FROM threads WHERE number = 4)",
    )
    .execute(&hub.state.db)
    .await
    .unwrap();
    stemma_hub::workshop::tick(&hub.state).await.unwrap();
    let expired = call(
        &hub,
        &ada,
        "GET",
        "/groups/geometry/workshop/requests/4",
        None,
    )
    .await;
    assert_eq!(expired["state"], "expired");
    let thread = call(&hub, &ada, "GET", "/groups/geometry/threads/4", None).await;
    assert!(
        thread["messages"][0]["body"]
            .as_str()
            .unwrap()
            .contains("expired")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn retries_use_another_machine_once_and_a_possible_push_fences_them() {
    let hub = hub(None).await;
    let ada = person(&hub, "ada", true).await;
    group(&hub, &ada).await;
    let other = call(
        &hub,
        &ada,
        "POST",
        "/me/tokens",
        Some(json!({"name":"Ada's second machine"})),
    )
    .await;
    let mut a = machine(&hub, &ada).await;
    let mut b = machine(&hub, other["secret"].as_str().unwrap()).await;
    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/workshop/requests",
        Some(ask_body("review-paper", json!({"kind":"mine"}))),
    )
    .await;
    let HubMessage::ProcessOffer(first) = next(&mut a).await else {
        panic!("first offer")
    };
    assert!(matches!(next(&mut b).await, HubMessage::ProcessOffer(_)));
    send(
        &mut a,
        WorkerMessage::ProcessDecision {
            process: first.process,
            accept: true,
            pending: false,
        },
    )
    .await;
    let HubMessage::Process(spec1) = next(&mut a).await else {
        panic!("first lease")
    };
    assert!(matches!(
        next(&mut b).await,
        HubMessage::ProcessTaken { .. }
    ));
    send(
        &mut a,
        WorkerMessage::ProcessFinished {
            lease: spec1.lease,
            result: Box::new(ProcessResult {
                error: Some("Runner stopped before pushing".into()),
                ..Default::default()
            }),
        },
    )
    .await;
    let HubMessage::ProcessOffer(retry) = next(&mut b).await else {
        panic!("offer on another machine")
    };
    assert_ne!(retry.process, first.process);
    send(
        &mut b,
        WorkerMessage::ProcessDecision {
            process: retry.process,
            accept: true,
            pending: false,
        },
    )
    .await;
    let HubMessage::Process(spec2) = next(&mut b).await else {
        panic!("second lease")
    };
    assert_eq!(spec2.lease.attempt, 1);
    assert_ne!(spec2.lease.id, spec1.lease.id);
    // The old generation cannot finish or heartbeat the new run.
    send(
        &mut a,
        WorkerMessage::ProcessFinished {
            lease: spec1.lease,
            result: Box::new(ProcessResult {
                summary: "Late success".into(),
                ..Default::default()
            }),
        },
    )
    .await;
    send(
        &mut b,
        WorkerMessage::ProcessFinished {
            lease: spec2.lease,
            result: Box::new(ProcessResult {
                error: Some("Second failure".into()),
                ..Default::default()
            }),
        },
    )
    .await;
    for _ in 0..100 {
        let request = call(
            &hub,
            &ada,
            "GET",
            "/groups/geometry/workshop/requests/1",
            None,
        )
        .await;
        if request["state"] == "failed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let attempts: i64 = sqlx::query_scalar("SELECT attempts FROM work_leases WHERE id=?")
        .bind(spec2.lease.id)
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    assert_eq!(attempts, 1);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM workshop_processes")
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    assert_eq!(count, 2);
    // Lost leases use the same limit, and a streamed push prevents even the first retry.
    sqlx::query("UPDATE work_leases SET state='running',attempts=1,heartbeat=? WHERE id=?")
        .bind(stemma_hub::db::now() - 61_000)
        .bind(spec2.lease.id)
        .execute(&hub.state.db)
        .await
        .unwrap();
    sqlx::query("UPDATE workshop_processes SET pushed=1 WHERE work_id=?")
        .bind(spec2.lease.id)
        .execute(&hub.state.db)
        .await
        .unwrap();
    stemma_hub::lease::release(&hub.state, None, stemma_hub::db::now())
        .await
        .unwrap();
    let status: String = sqlx::query_scalar("SELECT state FROM work_leases WHERE id=?")
        .bind(spec2.lease.id)
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    assert_eq!(status, "failed");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_approved_offer_waits_when_capacity_fills_before_acceptance() {
    let hub = hub(None).await;
    let ada = person(&hub, "ada", true).await;
    group(&hub, &ada).await;
    let mut socket = machine(&hub, &ada).await;
    for _ in 0..3 {
        call(
            &hub,
            &ada,
            "POST",
            "/groups/geometry/workshop/requests",
            Some(ask_body("review-paper", json!({"kind":"mine"}))),
        )
        .await;
    }
    let mut offers = Vec::new();
    for _ in 0..3 {
        let HubMessage::ProcessOffer(offer) = next(&mut socket).await else {
            panic!("offer")
        };
        offers.push(offer);
    }
    for offer in &offers[..2] {
        send(
            &mut socket,
            WorkerMessage::ProcessDecision {
                process: offer.process,
                accept: true,
                pending: false,
            },
        )
        .await;
    }
    let HubMessage::Process(first) = next(&mut socket).await else {
        panic!("first lease")
    };
    assert!(matches!(next(&mut socket).await, HubMessage::Process(_)));
    send(
        &mut socket,
        WorkerMessage::ProcessDecision {
            process: offers[2].process,
            accept: true,
            pending: false,
        },
    )
    .await;
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(80), next(&mut socket))
            .await
            .is_err(),
        "A full machine must keep its approval rather than being told the process is taken"
    );
    send(
        &mut socket,
        WorkerMessage::ProcessFinished {
            lease: first.lease,
            result: Box::new(ProcessResult {
                summary: "Finished the first review.".into(),
                ..Default::default()
            }),
        },
    )
    .await;
    for _ in 0..50 {
        let state: String = sqlx::query_scalar("SELECT state FROM work_leases WHERE id=?")
            .bind(first.lease.id)
            .fetch_one(&hub.state.db)
            .await
            .unwrap();
        if state == "passed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    // The machine retries its already approved offer as room becomes available.
    send(
        &mut socket,
        WorkerMessage::ProcessDecision {
            process: offers[2].process,
            accept: true,
            pending: false,
        },
    )
    .await;
    let HubMessage::Process(third) = next(&mut socket).await else {
        panic!("held approval should now get a lease")
    };
    assert_eq!(third.offer.process, offers[2].process);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_general_recipe_writes_a_paper_only_when_given_one() {
    let hub = hub(None).await;
    let ada = person(&hub, "ada", true).await;
    group(&hub, &ada).await;
    let mut socket = machine(&hub, &ada).await;
    let mut ask = ask_body("general", json!({"kind":"mine"}));
    ask["inputs"] = json!({"task": "Tidy the library's imports."});
    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/workshop/requests",
        Some(ask),
    )
    .await;
    let HubMessage::ProcessOffer(offer) = next(&mut socket).await else {
        panic!("offer")
    };
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
        panic!("lease")
    };
    let repositories: Vec<&str> = spec.repositories.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(repositories, ["/git/geometry/library.git"]);
}
