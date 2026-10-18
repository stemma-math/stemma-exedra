//! Working papers, over HTTP: made from the template in the forge, their
//! authors, built after each push with the last good PDF kept, read as
//! source, referenced as `[[p/slug]]`, and edited by the workshop, with
//! the approval of one of their authors. The forge is a stand-in for
//! Forgejo, and the workshop machine one the test plays.

use std::path::Path;
use std::sync::Arc;

use reqwest::StatusCode;
use serde_json::{Value, json};
use stemma_core::api::{Isolation, NewAccount};
use stemma_core::protocol::{
    HubMessage, JobSpec, Outcome, PaperBuilt, TaskEnd, TaskOutcome, Usage, Work,
};
use stemma_fake_forge::FakeForge;
use stemma_hub::config::{ForgeConfig, ForgeSecrets};
use tokio::sync::mpsc::UnboundedReceiver;

struct Hub {
    url: String,
    state: stemma_hub::AppState,
    forge: FakeForge,
    data: tempfile::TempDir,
}

async fn hub() -> Hub {
    let forge = FakeForge::start().await;
    let data = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let config = stemma_hub::Config {
        data_dir: data.path().to_owned(),
        public_url: Some(url.clone()),
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
    let app = stemma_hub::app(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await });
    Hub {
        url,
        state,
        forge,
        data,
    }
}

/// Someone, with their personal token.
#[derive(Clone)]
struct Person {
    id: i64,
    token: String,
}

/// Someone in `geometry`, with `role`; Ada, who administers the hub,
/// makes the group first.
async fn person(hub: &Hub, username: &str, role: &str) -> Person {
    let account = NewAccount {
        username: username.into(),
        display_name: capitalized(username),
        password: "password1".into(),
    };
    let id = stemma_hub::api::create_user(&hub.state, account, username == "ada")
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/api/v1/session", hub.url))
        .json(&json!({"username": username, "password": "password1"}))
        .send()
        .await
        .unwrap();
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let created: Value = client
        .post(format!("{}/api/v1/me/tokens", hub.url))
        .header("cookie", cookie)
        .json(&json!({"name": "tests"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let person = Person {
        id,
        token: created["secret"].as_str().unwrap().to_owned(),
    };
    if username == "ada" {
        let (status, body) = call(
            hub,
            &person,
            "POST",
            "/groups",
            Some(json!({"slug": "geometry", "name": "Geometry"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    } else {
        sqlx::query(
            "INSERT INTO memberships (group_id, user_id, role, joined_at)
             SELECT g.id, ?, ?, 0 FROM groups g WHERE g.slug = 'geometry'",
        )
        .bind(id)
        .bind(role)
        .execute(&hub.state.db)
        .await
        .unwrap();
    }
    person
}

fn capitalized(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

async fn call(
    hub: &Hub,
    person: &Person,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = reqwest::Client::new()
        .request(method, format!("{}/api/v1{path}", hub.url))
        .bearer_auth(&person.token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

async fn get(hub: &Hub, person: &Person, path: &str) -> Value {
    let (status, body) = call(hub, person, "GET", path, None).await;
    assert!(status.is_success(), "{path}: {status} {body}");
    body
}

/// Runs git in `dir`, as nobody in particular.
fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=Bob", "-c", "user.email=bob@example.org"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap()
}

/// A machine of Ada's in the workshop of `geometry`, with LaTeX and Lean,
/// isolating in containers, with Claude Code for the workshop, connected
/// as if over its WebSocket: what the hub sends it arrives on the
/// receiver.
async fn machine(hub: &Hub, ada: &Person) -> (i64, UnboundedReceiver<HubMessage>) {
    let (_, created) = call(
        hub,
        ada,
        "POST",
        "/me/machines",
        Some(json!({"name": "server"})),
    )
    .await;
    let id = created["id"].as_i64().unwrap();
    sqlx::query(
        "UPDATE workers SET last_seen_at = 1, lean = 1, latex = 1, isolation = 'docker'
         WHERE id = ?",
    )
    .bind(id)
    .execute(&hub.state.db)
    .await
    .unwrap();
    let (status, _) = call(
        hub,
        ada,
        "POST",
        "/groups/geometry/workshop/machines",
        Some(json!({"machine": id})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    call(
        hub,
        ada,
        "PUT",
        &format!("/me/machines/{id}/workshop-agents"),
        Some(json!([{"runner": "claude-code"}])),
    )
    .await;
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let workshop = stemma_hub::workshop::machines::groups_of(&hub.state, id)
        .await
        .unwrap();
    stemma_hub::jobs::connected(
        &hub.state,
        id,
        stemma_hub::jobs::Connected {
            personal: true,
            connection: id as u64,
            owner_id: ada.id,
            runners: vec!["claude-code".into()],
            lean: true,
            latex: true,
            isolation: Some(Isolation::Docker),
            workshop,
            concurrency: 4,
            preferred: false,
            running: Default::default(),
            sender,
        },
    )
    .await
    .unwrap();
    (id, receiver)
}

/// The jobs a machine was sent so far, in order.
fn sent(receiver: &mut UnboundedReceiver<HubMessage>) -> Vec<JobSpec> {
    let mut sent = Vec::new();
    while let Ok(message) = receiver.try_recv() {
        if let HubMessage::Job(spec) = message {
            sent.push(*spec);
        }
    }
    sent
}

/// The next build of a paper a machine is sent, waiting for it a while.
async fn next_job(receiver: &mut UnboundedReceiver<HubMessage>) -> JobSpec {
    for _ in 0..100 {
        if let Some(job) = sent(receiver)
            .into_iter()
            .find(|j| matches!(j.work, Work::PaperBuild(_)))
        {
            return job;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("no job was sent");
}

/// Uploads a PDF to where a build's job says, with `token`.
async fn upload(hub: &Hub, path: &str, token: &str, pdf: &[u8]) -> StatusCode {
    reqwest::Client::new()
        .put(format!("{}{path}", hub.url))
        .bearer_auth(token)
        .header("content-type", "application/pdf")
        .body(pdf.to_vec())
        .send()
        .await
        .unwrap()
        .status()
}

async fn built(hub: &Hub, worker: i64, job: i64, succeeded: bool, log: &str) {
    let outcome = Outcome::PaperBuilt(Box::new(PaperBuilt {
        succeeded,
        error: (!succeeded).then(|| "latexmk did not make the PDF: see the log".into()),
        log: log.into(),
        pdf: succeeded,
    }));
    stemma_hub::jobs::finished(&hub.state, worker, job, outcome, Usage::default())
        .await
        .unwrap();
}

async fn pdf(hub: &Hub, person: &Person) -> (StatusCode, Option<String>, Vec<u8>) {
    let response = reqwest::Client::new()
        .get(format!(
            "{}/api/v1/groups/geometry/papers/notes/pdf",
            hub.url
        ))
        .bearer_auth(&person.token)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let kind = response
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_owned());
    (status, kind, response.bytes().await.unwrap().to_vec())
}

#[tokio::test(flavor = "multi_thread")]
async fn papers_are_made_from_the_template_by_admins_and_members() {
    let hub = hub().await;
    let ada = person(&hub, "ada", "admin").await;
    let bob = person(&hub, "bob", "member").await;
    let cleo = person(&hub, "cleo", "member").await;
    let dave = person(&hub, "dave", "member").await;
    let gus = person(&hub, "gus", "guest").await;

    let new = |slug: &str, title: &str| Some(json!({"slug": slug, "title": title}));
    // Guests create no papers; slugs are slugs, and each is one paper's.
    let (status, _) = call(&hub, &gus, "POST", "/groups/geometry/papers", new("x", "X")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    for slug in ["Notes", "a--b", "", "a_b", "new"] {
        let (status, _) = call(
            &hub,
            &bob,
            "POST",
            "/groups/geometry/papers",
            new(slug, "X"),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{slug}");
    }
    let (status, paper) = call(
        &hub,
        &bob,
        "POST",
        "/groups/geometry/papers",
        new("notes", "Gluing & charts"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{paper}");
    let (status, _) = call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/papers",
        new("notes", "X"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Its repository, from the template, protected and hooked.
    assert_eq!(paper["slug"], "notes");
    assert_eq!(paper["repository"], "paper-notes");
    assert_eq!(paper["clone_path"], "/git/geometry/paper-notes.git");
    assert!(hub.forge.repository_exists("geometry", "paper-notes"));
    assert_eq!(hub.forge.hooks("geometry", "paper-notes").len(), 1);
    let main = hub
        .forge
        .file("geometry", "paper-notes", "main", "main.tex")
        .await
        .unwrap();
    assert!(main.contains("\\documentclass[11pt]{amsart}"), "{main}");
    assert!(main.contains("\\title{Gluing \\& charts}"), "{main}");
    assert!(main.contains("\\author{Bob}"), "{main}");
    assert!(main.contains("a working paper of Geometry"), "{main}");
    for file in ["references.bib", ".gitignore"] {
        assert!(
            hub.forge
                .file("geometry", "paper-notes", "main", file)
                .await
                .is_some(),
            "{file}"
        );
    }
    // Its creator is its author; its first build waits for a machine.
    assert_eq!(paper["authors"][0]["username"], "bob");
    assert_eq!(paper["created_by"]["username"], "bob");
    assert_eq!(paper["latest"]["state"], "queued");
    assert_eq!(
        paper["latest"]["progress"],
        "Waiting for a workshop machine with LaTeX to come online"
    );
    assert!(paper["pdf"].is_null());
    let listed = get(&hub, &gus, "/groups/geometry/papers").await;
    assert_eq!(listed[0]["slug"], "notes");

    // Its authors add others, who are not guests; an admin does too.
    let path = "/groups/geometry/papers/notes/authors";
    let (status, _) = call(&hub, &bob, "PUT", &format!("{path}/gus"), None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = call(&hub, &dave, "PUT", &format!("{path}/dave"), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, paper) = call(&hub, &bob, "PUT", &format!("{path}/cleo"), None).await;
    assert_eq!(status, StatusCode::OK);
    let authors: Vec<&str> = paper["authors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["username"].as_str().unwrap())
        .collect();
    assert_eq!(authors, ["bob", "cleo"]);
    let (status, _) = call(&hub, &cleo, "DELETE", &format!("{path}/bob"), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&hub, &ada, "PUT", &format!("{path}/bob"), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&hub, &ada, "DELETE", &format!("{path}/bob"), None).await;
    assert_eq!(status, StatusCode::OK);
    // A paper keeps an author.
    let (status, _) = call(&hub, &cleo, "DELETE", &format!("{path}/cleo"), None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(&hub, &ada, "PUT", &format!("{path}/bob"), None).await;
    assert_eq!(status, StatusCode::OK);

    // Its source, at main.
    let files = get(&hub, &gus, "/groups/geometry/papers/notes/files").await;
    let paths: Vec<&str> = files["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, [".gitignore", "main.tex", "references.bib"]);
    let file = get(&hub, &gus, "/groups/geometry/papers/notes/files/main.tex").await;
    assert_eq!(file["text"].as_str().unwrap().trim_end(), main);
    assert_eq!(file["commit"], files["commit"]);
    let (status, _) = call(
        &hub,
        &gus,
        "GET",
        "/groups/geometry/papers/notes/files/nothing.tex",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // `[[p/notes]]` references it; threads are about it.
    let (_, thread) = call(
        &hub,
        &bob,
        "POST",
        "/groups/geometry/threads",
        Some(json!({"title": "Charts", "body": "As [[p/notes]] says."})),
    )
    .await;
    assert_eq!(thread["thread"]["number"], 1);
    let resolved = get(&hub, &gus, "/groups/geometry/papers").await;
    assert_eq!(resolved.as_array().unwrap().len(), 1);
    let (_, resolved) = call(
        &hub,
        &gus,
        "POST",
        "/groups/geometry/resolve",
        Some(json!({"addresses": ["p/notes", "p/nope"]})),
    )
    .await;
    assert_eq!(resolved[0]["exists"], true);
    assert_eq!(resolved[0]["kind"], "paper");
    assert_eq!(resolved[0]["title"], "Gluing & charts");
    assert_eq!(resolved[0]["path"], "/g/geometry/p/notes");
    assert_eq!(resolved[1]["exists"], false);
    assert_eq!(resolved[1]["note"], "There is no such paper");
    let (status, about) = call(
        &hub,
        &bob,
        "POST",
        "/groups/geometry/threads",
        Some(json!({"title": "The charts section", "anchor": "p/notes"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(about["thread"]["anchor"], "p/notes");
    let (status, _) = call(
        &hub,
        &bob,
        "POST",
        "/groups/geometry/threads",
        Some(json!({"title": "Nothing", "anchor": "p/nope"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn each_push_is_compiled_and_the_last_good_pdf_is_kept() {
    let hub = hub().await;
    let ada = person(&hub, "ada", "admin").await;
    let bob = person(&hub, "bob", "member").await;
    let gus = person(&hub, "gus", "guest").await;
    call(
        &hub,
        &bob,
        "POST",
        "/groups/geometry/papers",
        Some(json!({"slug": "notes", "title": "Notes"})),
    )
    .await;
    let first = hub
        .forge
        .branch_commit("geometry", "paper-notes", "main")
        .await
        .unwrap();
    let (worker, mut receiver) = machine(&hub, &ada).await;

    // The first build goes to the machine with LaTeX.
    let job = next_job(&mut receiver).await;
    let Work::PaperBuild(compile) = &job.work else {
        panic!("not a paper's build: {:?}", job.work);
    };
    assert_eq!(compile.paper, "notes");
    assert_eq!(compile.repository, "/git/geometry/paper-notes.git");
    assert_eq!(compile.commit, first);
    assert_eq!(compile.main, "main.tex");
    let paper = get(&hub, &gus, "/groups/geometry/papers/notes").await;
    assert_eq!(paper["latest"]["state"], "running");

    // Only the build's own token uploads its PDF, and only a PDF.
    let one = b"%PDF-1.5 the first".to_vec();
    assert_eq!(
        upload(&hub, &compile.upload, &bob.token, &one).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        upload(&hub, &compile.upload, &job.token, b"<html>").await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        upload(&hub, &compile.upload, &job.token, &one).await,
        StatusCode::NO_CONTENT
    );
    built(&hub, worker, job.id, true, "Output written on main.pdf").await;
    let paper = get(&hub, &gus, "/groups/geometry/papers/notes").await;
    assert_eq!(paper["latest"]["state"], "succeeded");
    assert_eq!(paper["pdf"]["commit"], first.as_str());
    assert_eq!(paper["pdf"]["size"], one.len());
    assert_eq!(
        paper["pdf"]["url"],
        "/api/v1/groups/geometry/papers/notes/pdf"
    );
    assert_eq!(paper["log"], "Output written on main.pdf");
    let (status, kind, bytes) = pdf(&hub, &gus).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(kind.as_deref(), Some("application/pdf"));
    assert_eq!(bytes, one);
    let first_build = paper["pdf"]["build"].as_i64().unwrap();
    // The job is over: its token uploads no more.
    assert_ne!(
        upload(&hub, &compile.upload, &job.token, &one).await,
        StatusCode::NO_CONTENT
    );

    // A push to main, through the hub, is compiled.
    let work = tempfile::tempdir().unwrap();
    let host = hub.url.trim_start_matches("http://");
    let url = format!(
        "http://bob:{}@{host}/git/geometry/paper-notes.git",
        bob.token
    );
    let cloned = git(work.path(), &["clone", "-q", &url, "notes"]);
    assert!(cloned.status.success(), "{cloned:?}");
    let dir = work.path().join("notes");
    std::fs::write(
        dir.join("main.tex"),
        "\\documentclass{amsart}\\begin{document}\\oops",
    )
    .unwrap();
    git(&dir, &["commit", "-qam", "Break it"]);
    let pushed = git(&dir, &["push", "-q", "origin", "main"]);
    assert!(pushed.status.success(), "{pushed:?}");
    let second = String::from_utf8(git(&dir, &["rev-parse", "HEAD"]).stdout).unwrap();
    let job = next_job(&mut receiver).await;
    let Work::PaperBuild(compile) = &job.work else {
        panic!("not a paper's build");
    };
    assert_eq!(compile.commit, second.trim());

    // It fails, having uploaded a PDF: the paper keeps the last good one.
    let two = b"%PDF-1.5 the second".to_vec();
    assert_eq!(
        upload(&hub, &compile.upload, &job.token, &two).await,
        StatusCode::NO_CONTENT
    );
    built(&hub, worker, job.id, false, "! Undefined control sequence.").await;
    let paper = get(&hub, &gus, "/groups/geometry/papers/notes").await;
    assert_eq!(paper["latest"]["state"], "failed");
    assert_eq!(paper["latest"]["commit"], second.trim());
    assert_eq!(
        paper["latest"]["error"],
        "latexmk did not make the PDF: see the log"
    );
    assert_eq!(paper["log"], "! Undefined control sequence.");
    assert_eq!(paper["pdf"]["commit"], first.as_str());
    assert_eq!(pdf(&hub, &gus).await.2, one);
    let papers = hub.data.path().join("papers");
    let failed_build = paper["latest"]["id"].as_i64().unwrap();
    assert!(!papers.join(format!("{failed_build}.pdf")).exists());

    // Guests ask for no builds; members do. A new PDF replaces the last.
    let path = "/groups/geometry/papers/notes/builds";
    let (status, _) = call(&hub, &gus, "POST", path, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&hub, &bob, "POST", path, None).await;
    assert_eq!(status, StatusCode::CREATED);
    let job = next_job(&mut receiver).await;
    let Work::PaperBuild(compile) = &job.work else {
        panic!("not a paper's build");
    };
    let three = b"%PDF-1.5 the third".to_vec();
    upload(&hub, &compile.upload, &job.token, &three).await;
    built(&hub, worker, job.id, true, "Fine").await;
    let paper = get(&hub, &gus, "/groups/geometry/papers/notes").await;
    assert_eq!(paper["pdf"]["commit"], second.trim());
    assert_eq!(pdf(&hub, &gus).await.2, three);
    assert!(!papers.join(format!("{first_build}.pdf")).exists());
}

/// A task's run on a paper ends as a machine says, with `commit` pushed.
async fn finish(hub: &Hub, worker: i64, job: i64, commit: String) {
    let outcome = Outcome::Task(Box::new(TaskOutcome {
        end: TaskEnd::Done,
        reason: None,
        report: "Wrote the section on charts.".into(),
        commit: Some(commit),
        log: "Round 1".into(),
        built: None,
        runner: "claude-code".into(),
        model: None,
    }));
    stemma_hub::jobs::finished(&hub.state, worker, job, outcome, Usage::default())
        .await
        .unwrap();
}

/// The task a job runs, as the machine gets it.
fn task_of(job: &JobSpec) -> &stemma_core::protocol::TaskRun {
    match &job.work {
        Work::TaskRun(run) => run,
        other => panic!("not a task: {other:?}"),
    }
}

/// The next task a machine is sent, its builds aside.
async fn next_task(receiver: &mut UnboundedReceiver<HubMessage>) -> JobSpec {
    for _ in 0..100 {
        if let Some(job) = sent(receiver)
            .into_iter()
            .find(|j| matches!(j.work, Work::TaskRun(_)))
        {
            return job;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("no task was sent");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_workshop_edits_papers_with_an_authors_approval() {
    let hub = hub().await;
    let ada = person(&hub, "ada", "admin").await;
    let bob = person(&hub, "bob", "member").await;
    let cleo = person(&hub, "cleo", "member").await;
    let gus = person(&hub, "gus", "guest").await;
    call(
        &hub,
        &bob,
        "POST",
        "/groups/geometry/papers",
        Some(json!({"slug": "notes", "title": "Notes"})),
    )
    .await;
    let (worker, mut receiver) = machine(&hub, &ada).await;
    let edit = |title: &str| Some(json!({"title": title, "instructions": "Write up t1."}));

    // Guests ask for no edits; Cleo, who is not an author, does.
    let path = "/groups/geometry/papers/notes/tasks";
    let (status, _) = call(&hub, &gus, "POST", path, edit("No")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, task) = call(&hub, &cleo, "POST", path, edit("Write the charts")).await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    assert_eq!(task["architecture"], "write");
    assert_eq!(task["paper"], "notes");
    assert_eq!(task["origin"], "t1#1");
    let thread = get(&hub, &cleo, "/groups/geometry/threads/1").await;
    assert_eq!(thread["thread"]["anchor"], "p/notes");
    assert_eq!(thread["messages"][0]["body"], "Write up t1.");

    let job = next_task(&mut receiver).await;
    let run = task_of(&job);
    assert_eq!(run.paper.as_deref(), Some("notes"));
    assert_eq!(run.repository, "/git/geometry/paper-notes.git");
    assert_eq!(run.branch, "workshop/w1");
    assert_eq!(run.architecture.name, "write");
    assert_eq!(run.architecture.steps, ["work", "compile", "propose"]);
    assert!(run.architecture.instructions.contains("{paper}"));
    assert!(run.data_sorry.is_empty());

    // Its token pushes to its branch of the paper, and to no other
    // repository.
    let work = tempfile::tempdir().unwrap();
    let host = hub.url.trim_start_matches("http://");
    let library = format!("http://job:{}@{host}/git/geometry/library.git", job.token);
    let cloned = git(work.path(), &["clone", "-q", &library, "library"]);
    assert!(cloned.status.success(), "{cloned:?}");
    let dir = work.path().join("library");
    git(&dir, &["commit", "-q", "--allow-empty", "-m", "Elsewhere"]);
    let pushed = git(
        &dir,
        &["push", "-q", "origin", "HEAD:refs/heads/workshop/w1"],
    );
    assert!(!pushed.status.success(), "{pushed:?}");
    let paper = format!(
        "http://job:{}@{host}/git/geometry/paper-notes.git",
        job.token
    );
    let cloned = git(work.path(), &["clone", "-q", &paper, "notes"]);
    assert!(cloned.status.success(), "{cloned:?}");
    let dir = work.path().join("notes");
    std::fs::write(dir.join("charts.tex"), "\\section{Charts}\n").unwrap();
    git(&dir, &["add", "charts.tex"]);
    git(&dir, &["commit", "-qm", "Write the charts"]);
    let pushed = git(
        &dir,
        &["push", "-q", "origin", "HEAD:refs/heads/workshop/w1"],
    );
    assert!(pushed.status.success(), "{pushed:?}");
    let commit = String::from_utf8(git(&dir, &["rev-parse", "HEAD"]).stdout).unwrap();
    finish(&hub, worker, job.id, commit.trim().to_owned()).await;

    // Cleo asked; one of the paper's authors approves too.
    let task = get(&hub, &cleo, "/groups/geometry/tasks/1").await;
    assert_eq!(task["state"], "review", "{task}");
    assert_eq!(task["changes"]["repository"], "paper-notes");
    assert_eq!(task["changes"]["needs"], json!(["asker", "author"]));
    assert!(
        task["changes"]["url"]
            .as_str()
            .unwrap()
            .ends_with("/geometry/paper-notes/pulls/1/files")
    );
    let messages = get(&hub, &cleo, "/groups/geometry/threads/1/messages").await;
    let report = messages.as_array().unwrap().last().unwrap()["body"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        report.contains("once Cleo approves them, and one of the authors of [[p/notes]] too"),
        "{report}"
    );
    let approve = "/groups/geometry/tasks/1/approve";
    let (status, _) = call(&hub, &ada, "POST", approve, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&hub, &cleo, "POST", approve, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let task = get(&hub, &cleo, "/groups/geometry/tasks/1").await;
    assert_eq!(task["state"], "review");
    assert_eq!(task["changes"]["needs"], json!(["author"]));
    let (status, _) = call(&hub, &bob, "POST", approve, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let task = get(&hub, &cleo, "/groups/geometry/tasks/1").await;
    assert_eq!(task["state"], "done");
    let approvers: Vec<&str> = task["changes"]["approvals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["approver"].as_str().unwrap())
        .collect();
    assert_eq!(approvers, ["asker", "author"]);
    assert!(
        hub.forge
            .file("geometry", "paper-notes", "main", "charts.tex")
            .await
            .is_some()
    );
    let messages = get(&hub, &cleo, "/groups/geometry/threads/1/messages").await;
    let merged = messages.as_array().unwrap().last().unwrap()["body"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        merged.starts_with("**Merged:** [[w1]] is in [[p/notes]], approved by Cleo and Bob."),
        "{merged}"
    );
    // The merge, a push to main, is compiled.
    let merged_commit = task["changes"]["merged"].as_str().unwrap().to_owned();
    let mut compiled = false;
    for _ in 0..100 {
        let paper = get(&hub, &bob, "/groups/geometry/papers/notes").await;
        if paper["latest"]["commit"] == merged_commit.as_str() {
            compiled = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(compiled, "main's new commit is compiled");

    // An author asks, in the thread about the paper: they approve alone,
    // unless the change is big.
    let (status, _) = call(
        &hub,
        &bob,
        "POST",
        "/groups/geometry/threads/1/messages",
        Some(json!({"body": "@workshop drop the references, we cite none."})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let task = get(&hub, &bob, "/groups/geometry/tasks/2").await;
    assert_eq!(task["paper"], "notes");
    assert_eq!(task["architecture"], "write");
    let job = next_task(&mut receiver).await;
    assert_eq!(task_of(&job).paper.as_deref(), Some("notes"));
    let commit = hub
        .forge
        .push_branch(
            "geometry",
            "paper-notes",
            "workshop/w2",
            &[],
            &["references.bib"],
        )
        .await;
    finish(&hub, worker, job.id, commit).await;
    let task = get(&hub, &bob, "/groups/geometry/tasks/2").await;
    assert_eq!(task["changes"]["big"], true);
    assert_eq!(
        task["changes"]["why_big"],
        json!(["deletes references.bib"])
    );
    assert_eq!(task["changes"]["needs"], json!(["asker", "admin"]));

    // The architecture and the paper go together.
    let (status, _) = call(
        &hub,
        &bob,
        "POST",
        "/groups/geometry/tasks",
        Some(json!({"title": "X", "instructions": "Y", "origin": "t1#1",
                    "architecture": "write"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = call(
        &hub,
        &bob,
        "POST",
        "/groups/geometry/tasks",
        Some(json!({"title": "X", "instructions": "Y", "origin": "t1#1",
                    "architecture": "formalize", "paper": "notes"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, task) = call(
        &hub,
        &bob,
        "POST",
        "/groups/geometry/tasks",
        Some(json!({"title": "X", "instructions": "Y", "origin": "t1#1", "paper": "p/notes"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(task["architecture"], "write");
    let (status, _) = call(
        &hub,
        &bob,
        "POST",
        "/groups/geometry/tasks",
        Some(json!({"title": "X", "instructions": "Y", "origin": "t1#1", "paper": "nope"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
