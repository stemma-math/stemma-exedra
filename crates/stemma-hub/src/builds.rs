//! Builds coordinate here; all checkout, compilation and inspection runs on machines.
use crate::{
    auth::{Scope, Viewer},
    db,
    error::{ApiError, ApiResult},
    forge, lease,
    state::AppState,
};
use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    routing::post,
};
use std::collections::{BTreeMap, HashMap};
use stemma_core::{
    api::Event,
    protocol::{
        BuildKind, BuildResult, BuildSpec, GroupRef, GroupWorkshop, Hello, HubMessage, LeaseRef,
    },
};
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct Machine {
    pub connection: u64,
    pub owner: i64,
    pub name: String,
    pub version: String,
    pub general: GroupWorkshop,
    pub groups: BTreeMap<String, GroupWorkshop>,
    pub sender: mpsc::UnboundedSender<HubMessage>,
}
impl Machines {
    pub fn rename_group(&mut self, old: &str, new: &str) {
        for machine in self.0.values_mut() {
            if let Some(scope) = machine.groups.remove(old) {
                machine.groups.insert(new.into(), scope);
            }
        }
    }
}
impl Machine {
    pub(crate) fn scope(&self, slug: &str) -> &GroupWorkshop {
        self.groups.get(slug).unwrap_or(&self.general)
    }
}
#[derive(Default)]
pub struct Machines(pub HashMap<i64, Machine>);

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/builds/{work}/pdf", post(pdf))
        .route("/api/v1/builds/{work}/result", post(report))
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(state)
}

pub async fn connected(state: &AppState, id: i64, machine: Machine) -> ApiResult<()> {
    let _turn = state.build_changes.lock().await;
    let old = state.build_machines.lock().await.0.insert(id, machine);
    if let Some(old) = old {
        for (_, lease) in lease::release(state, Some((id, old.connection)), db::now()).await? {
            publish(state, lease.id).await?;
        }
    }
    drop(_turn);
    dispatch(state).await
}
pub async fn hello(state: &AppState, id: i64, connection: u64, hello: &Hello) -> ApiResult<()> {
    if let Some(m) = state
        .build_machines
        .lock()
        .await
        .0
        .get_mut(&id)
        .filter(|m| m.connection == connection)
    {
        m.general = hello.workshop.clone();
        m.groups = hello.workshop_groups.clone();
        m.version = hello.version.clone();
    }
    dispatch(state).await
}
pub async fn disconnect_token(state: &AppState, id: i64) -> ApiResult<()> {
    let connection = state
        .build_machines
        .lock()
        .await
        .0
        .get(&id)
        .map(|m| m.connection);
    if let Some(connection) = connection {
        disconnected(state, id, connection).await?;
    }
    Ok(())
}
pub async fn disconnected(state: &AppState, id: i64, connection: u64) -> ApiResult<()> {
    let _turn = state.build_changes.lock().await;
    let mut machines = state.build_machines.lock().await;
    if machines
        .0
        .get(&id)
        .is_some_and(|m| m.connection == connection)
    {
        machines.0.remove(&id);
    }
    drop(machines);
    for (_, lease) in lease::release(state, Some((id, connection)), db::now()).await? {
        publish(state, lease.id).await?;
    }
    drop(_turn);
    dispatch(state).await
}

/// Ask Forgejo under the queue lock: webhook delivery order cannot move main backwards.
pub async fn refresh(
    state: &AppState,
    group: i64,
    repository: &str,
    paper: Option<i64>,
) -> ApiResult<()> {
    let _turn = state.build_changes.lock().await;
    refresh_main(state, group, repository, paper).await?;
    drop(_turn);
    dispatch(state).await
}

async fn refresh_main(
    state: &AppState,
    group: i64,
    repository: &str,
    paper: Option<i64>,
) -> ApiResult<()> {
    let Some(forge) = state.forge.as_deref() else {
        return Ok(());
    };
    let slug: String = sqlx::query_scalar("SELECT slug FROM groups WHERE id = ?")
        .bind(group)
        .fetch_one(&state.db)
        .await?;
    let Some(head) = forge
        .branch_commit(&slug, repository, forge::MAIN)
        .await
        .map_err(forge::api_error)?
    else {
        return Ok(());
    };
    let known: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1
         FROM work_leases
         WHERE group_id = ?
         AND repository = ?
         AND commit_sha = ?
         AND state != 'cancelled')",
    )
    .bind(group)
    .bind(repository)
    .bind(&head)
    .fetch_one(&state.db)
    .await?;
    let recorded: Option<String> = if let Some(paper) = paper {
        sqlx::query_scalar("SELECT head FROM papers WHERE id = ?")
            .bind(paper)
            .fetch_one(&state.db)
            .await?
    } else {
        sqlx::query_scalar("SELECT head FROM libraries WHERE group_id = ?")
            .bind(group)
            .fetch_one(&state.db)
            .await?
    };
    if known && recorded.as_deref() == Some(&head) {
        return Ok(());
    }
    let mut tx = state.db.begin().await?;
    if let Some(paper) = paper {
        sqlx::query("UPDATE papers SET head = ? WHERE id = ?")
            .bind(&head)
            .bind(paper)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("UPDATE libraries SET head = ? WHERE group_id = ?")
            .bind(&head)
            .bind(group)
            .execute(&mut *tx)
            .await?;
    }
    let old: Vec<(i64, i64, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT id, attempts, holder, token_id
         FROM work_leases
         WHERE group_id = ?
         AND repository = ?
         AND commit_sha != ?
         AND state IN ('queued', 'running')",
    )
    .bind(group)
    .bind(repository)
    .bind(&head)
    .fetch_all(&mut *tx)
    .await?;
    for (id, _, _, token) in &old {
        sqlx::query("UPDATE work_leases SET state = 'cancelled' WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE api_tokens SET revoked_at = ? WHERE id = ?")
            .bind(db::now())
            .bind(token)
            .execute(&mut *tx)
            .await?;
        for table in ["library_builds", "paper_builds"] {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "UPDATE {table} SET accepted = 0, finished_at = ? WHERE work_id = ?"
            )))
            .bind(db::now())
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
    }
    let work = sqlx::query(
        "INSERT INTO work_leases (group_id, repository, commit_sha, kind, created_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(group)
    .bind(repository)
    .bind(&head)
    .bind(if paper.is_some() { "paper" } else { "library" })
    .bind(db::now())
    .execute(&mut *tx)
    .await?;
    if work.rows_affected() > 0 {
        let id = work.last_insert_rowid();
        if let Some(paper) = paper {
            sqlx::query(
                "INSERT INTO paper_builds (paper_id, commit_sha, work_id, created_at)
                 VALUES (?, ?, ?, ?)",
            )
            .bind(paper)
            .bind(&head)
            .bind(id)
            .bind(db::now())
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query(
                "INSERT INTO library_builds (group_id, commit_sha, work_id, created_at)
                 VALUES (?, ?, ?, ?)",
            )
            .bind(group)
            .bind(&head)
            .bind(id)
            .bind(db::now())
            .execute(&mut *tx)
            .await?;
        }
    }
    let resumed = sqlx::query(
        "UPDATE work_leases
         SET state = 'queued', holder = NULL, connection = NULL, attempts = 0,
         heartbeat = NULL
         WHERE group_id = ? AND repository = ? AND commit_sha = ? AND state = 'cancelled'",
    )
    .bind(group)
    .bind(repository)
    .bind(&head)
    .execute(&mut *tx)
    .await?;
    if resumed.rows_affected() > 0 {
        let table = if paper.is_some() {
            "paper_builds"
        } else {
            "library_builds"
        };
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE {table}
             SET accepted = 1, succeeded = NULL, error = NULL, finished_at = NULL
             WHERE work_id = (SELECT id
             FROM work_leases
             WHERE group_id = ?
             AND repository = ?
             AND commit_sha = ?)"
        )))
        .bind(group)
        .bind(repository)
        .bind(&head)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    let machines = state.build_machines.lock().await;
    for (id, attempt, holder, _) in old {
        if let Some(machine) = holder.and_then(|id| machines.0.get(&id)) {
            let _ = machine.sender.send(HubMessage::CancelBuild {
                lease: LeaseRef {
                    id,
                    attempt: attempt as u32,
                },
            });
        }
    }
    drop(machines);
    if let Some(work) = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM work_leases WHERE group_id = ? AND repository = ? AND commit_sha = ?",
    )
    .bind(group)
    .bind(repository)
    .bind(&head)
    .fetch_optional(&state.db)
    .await?
    {
        publish(state, work).await?;
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct Waiting {
    id: i64,
    group_id: i64,
    slug: String,
    name: String,
    repository: String,
    commit_sha: String,
    kind: String,
    attempts: i64,
}

pub async fn dispatch(state: &AppState) -> ApiResult<()> {
    // Where main is comes from the forge's push hook, and is asked again
    // when a build finishes: dispatching never asks the forge.
    let _turn = state.build_changes.lock().await;
    let expired = lease::release(state, None, db::now()).await?;
    let machines = state.build_machines.lock().await;
    for (holder, lease) in expired {
        publish(state, lease.id).await?;
        if let Some(machine) = machines.0.get(&holder) {
            let process: bool =
                sqlx::query_scalar("SELECT kind='process' FROM work_leases WHERE id=?")
                    .bind(lease.id)
                    .fetch_one(&state.db)
                    .await?;
            let _ = machine.sender.send(if process {
                HubMessage::CancelProcess { lease }
            } else {
                HubMessage::CancelBuild { lease }
            });
        }
    }
    let waiting: Vec<Waiting> = sqlx::query_as(
        "SELECT w.id, w.group_id, g.slug, g.name, w.repository, w.commit_sha, w.kind,
         w.attempts
         FROM work_leases w
         JOIN groups g ON g.id = w.group_id
         WHERE w.state = 'queued' AND w.kind != 'process'
         ORDER BY w.id",
    )
    .fetch_all(&state.db)
    .await?;
    for work in waiting {
        for (holder, machine) in &machines.0 {
            let scope = machine.scope(&work.slug);
            if !scope.enabled
                || !scope.build
                || scope.build_concurrency == 0
                || machine.sender.is_closed()
            {
                continue;
            }
            let eligible: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1
                 FROM memberships m
                 JOIN api_tokens t ON t.user_id = m.user_id
                 WHERE m.group_id = ?
                 AND m.user_id = ?
                 AND m.role IN ('admin', 'member')
                 AND t.id = ?
                 AND t.revoked_at IS NULL
                 AND (t.expires_at IS NULL OR t.expires_at > ?))",
            )
            .bind(work.group_id)
            .bind(machine.owner)
            .bind(holder)
            .bind(db::now())
            .fetch_one(&state.db)
            .await?;
            if !eligible {
                continue;
            }
            let running: i64 = sqlx::query_scalar(
                "SELECT count(*)
                 FROM work_leases
                 WHERE holder = ? AND state = 'running' AND kind != 'process'",
            )
            .bind(holder)
            .fetch_one(&state.db)
            .await?;
            if running >= scope.build_concurrency as i64 {
                continue;
            }
            let lease = LeaseRef {
                id: work.id,
                attempt: work.attempts as u32 + 1,
            };
            let secret = crate::secret::token("stb_");
            let mut tx = state.db.begin().await?;
            let token = sqlx::query(
                "INSERT INTO api_tokens (user_id, name, token_hash, prefix, created_at, kind,
                 group_id, build_work, lease_attempt, can_post, attachments, expires_at)
                 VALUES (?, 'Build', ?, ?, ?, 'build', ?, ?, ?, 0, 'none', ?)",
            )
            .bind(machine.owner)
            .bind(crate::secret::token_hash(&secret))
            .bind(&secret[..10])
            .bind(db::now())
            .bind(work.group_id)
            .bind(work.id)
            .bind(lease.attempt)
            .bind(db::now() + lease::EXPIRY_MILLIS)
            .execute(&mut *tx)
            .await?
            .last_insert_rowid();
            if !lease::acquire(
                &mut tx,
                lease,
                *holder,
                machine.connection,
                token,
                &machine.name,
            )
            .await?
            {
                tx.rollback().await?;
                continue;
            }
            tx.commit().await?;
            publish(state, work.id).await?;
            let _ = machine.sender.send(HubMessage::Build(Box::new(BuildSpec {
                lease,
                group: GroupRef {
                    id: work.group_id,
                    slug: work.slug.clone(),
                    name: work.name.clone(),
                },
                repository: format!("/git/{}/{}.git", work.slug, work.repository),
                commit: work.commit_sha.clone(),
                kind: if work.kind == "library" {
                    BuildKind::Library
                } else {
                    BuildKind::Paper
                },
                token: secret,
            })));
            break;
        }
    }
    Ok(())
}

pub async fn heartbeat(
    state: &AppState,
    holder: i64,
    connection: u64,
    lease: LeaseRef,
) -> ApiResult<()> {
    lease::heartbeat(state, holder, connection, lease).await?;
    sqlx::query(
        "UPDATE api_tokens
         SET expires_at = ?
         WHERE id = (SELECT token_id
         FROM work_leases
         WHERE id = ?
         AND holder = ?
         AND connection = ?
         AND attempts = ?
         AND state = 'running'
         AND heartbeat > ?)
         AND revoked_at IS NULL",
    )
    .bind(db::now() + lease::EXPIRY_MILLIS)
    .bind(lease.id)
    .bind(holder)
    .bind(connection as i64)
    .bind(lease.attempt)
    .bind(db::now() - lease::EXPIRY_MILLIS)
    .execute(&state.db)
    .await?;
    Ok(())
}

fn clean(text: &str, max: usize) -> String {
    let text = stemma_core::secrets::hide(text);
    let mut start = text.len().saturating_sub(max);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_owned()
}

pub async fn finished(
    state: &AppState,
    holder: i64,
    connection: u64,
    lease: LeaseRef,
    result: BuildResult,
) -> ApiResult<()> {
    // Check the forge again before publication, even if its webhook is delayed.
    if let Some((group, repository)) = sqlx::query_as::<_, (i64, String)>(
        "SELECT group_id, repository FROM work_leases WHERE id = ?",
    )
    .bind(lease.id)
    .fetch_optional(&state.db)
    .await?
    {
        let paper: Option<i64> =
            sqlx::query_scalar("SELECT id FROM papers WHERE group_id = ? AND repository = ?")
                .bind(group)
                .bind(&repository)
                .fetch_optional(&state.db)
                .await?;
        refresh(state, group, &repository, paper).await?;
    }
    let _turn = state.build_changes.lock().await;
    let row: Option<(i64, String, Option<i64>)> = sqlx::query_as(
        "SELECT group_id, kind, token_id
         FROM work_leases
         WHERE id = ?
         AND holder = ?
         AND connection = ?
         AND attempts = ?
         AND state IN ('running', 'cancelled')",
    )
    .bind(lease.id)
    .bind(holder)
    .bind(connection as i64)
    .bind(lease.attempt)
    .fetch_optional(&state.db)
    .await?;
    let Some((_group, kind, token)) = row else {
        return Ok(());
    };
    let mut tx = state.db.begin().await?;
    let succeeded = result.succeeded
        && if kind == "library" {
            result.report.is_some()
        } else {
            sqlx::query_scalar::<_, Option<i64>>(
                "SELECT pdf_size FROM paper_builds WHERE work_id = ?",
            )
            .bind(lease.id)
            .fetch_one(&mut *tx)
            .await?
            .is_some()
        };
    let error = result
        .error
        .as_deref()
        .map(|s| clean(s, 2000))
        .or_else(|| (!succeeded).then(|| "The build did not produce its report or PDF".into()));
    let table = if kind == "library" {
        "library_builds"
    } else {
        "paper_builds"
    };
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE {table} SET succeeded = ?, error = ?, log = ?, finished_at = ? WHERE work_id = ?"
    )))
    .bind(succeeded)
    .bind(&error)
    .bind(clean(&result.log, 200_000))
    .bind(db::now())
    .bind(lease.id)
    .execute(&mut *tx)
    .await?;
    if kind == "library" {
        if succeeded && let Some(report) = &result.report {
            let id: i64 = sqlx::query_scalar("SELECT id FROM library_builds WHERE work_id = ?")
                .bind(lease.id)
                .fetch_one(&mut *tx)
                .await?;
            crate::library::store_report(&mut tx, id, report).await?;
            sqlx::query("UPDATE library_builds SET lean = ? WHERE id = ?")
                .bind(&report.lean)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
    } else if succeeded {
        sqlx::query(
            "UPDATE papers
             SET pdf_build = (SELECT id FROM paper_builds WHERE work_id = ?1)
             WHERE id = (SELECT paper_id
             FROM paper_builds
             WHERE work_id = ?1
             AND accepted = 1
             AND commit_sha = papers.head)",
        )
        .bind(lease.id)
        .execute(&mut *tx)
        .await?;
    }
    // A cancelled build's result is history only: its lease stays
    // cancelled, so that main coming back to its commit builds it again.
    sqlx::query(
        "UPDATE work_leases
         SET state = CASE state WHEN 'cancelled' THEN state ELSE ? END, error = ?
         WHERE id = ?",
    )
    .bind(if succeeded { "passed" } else { "failed" })
    .bind(error)
    .bind(lease.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE api_tokens SET revoked_at = ? WHERE id = ?")
        .bind(db::now())
        .bind(token)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    publish(state, lease.id).await?;
    drop(_turn);
    dispatch(state).await
}

async fn token_lease(
    state: &AppState,
    viewer: &Viewer,
    work: i64,
) -> ApiResult<(i64, u64, LeaseRef)> {
    let Scope::Build {
        work: allowed,
        attempt,
        ..
    } = viewer.scope
    else {
        return Err(ApiError::forbidden("A build token is required"));
    };
    if allowed != work {
        return Err(ApiError::forbidden("This token belongs to another build"));
    }
    let (holder, connection): (i64, i64) = sqlx::query_as(
        "SELECT holder, connection
         FROM work_leases
         WHERE id = ? AND attempts = ? AND state = 'running'",
    )
    .bind(work)
    .bind(attempt)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::forbidden("This build lease ended"))?;
    Ok((holder, connection as u64, LeaseRef { id: work, attempt }))
}
async fn report(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(work): Path<i64>,
    axum::Json(result): axum::Json<BuildResult>,
) -> ApiResult<axum::http::StatusCode> {
    let (holder, connection, lease) = token_lease(&state, &viewer, work).await?;
    finished(&state, holder, connection, lease, result).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
async fn pdf(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(work): Path<i64>,
    bytes: Bytes,
) -> ApiResult<axum::http::StatusCode> {
    let _turn = state.build_changes.lock().await;
    token_lease(&state, &viewer, work).await?;
    if !bytes.starts_with(b"%PDF-") {
        return Err(ApiError::invalid("The build did not upload a PDF"));
    }
    let id: i64 = sqlx::query_scalar("SELECT id FROM paper_builds WHERE work_id = ?")
        .bind(work)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::forbidden("Only a paper build uploads a PDF"))?;
    let path = crate::papers::pdf_path(&state, id);
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .map_err(ApiError::internal)?;
    tokio::fs::write(path, &bytes)
        .await
        .map_err(ApiError::internal)?;
    sqlx::query("UPDATE paper_builds SET pdf_size = ? WHERE id = ?")
        .bind(bytes.len() as i64)
        .bind(id)
        .execute(&state.db)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Called while deleting a repository, with build_changes held by the caller.
pub async fn forget_repository(state: &AppState, group: i64, repository: &str) -> ApiResult<()> {
    let work: Vec<(i64, i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, attempts, holder FROM work_leases WHERE group_id = ? AND repository = ?",
    )
    .bind(group)
    .bind(repository)
    .fetch_all(&state.db)
    .await?;
    let machines = state.build_machines.lock().await;
    for (id, attempt, holder) in work {
        if let Some(machine) = holder.and_then(|h| machines.0.get(&h)) {
            let _ = machine.sender.send(HubMessage::CancelBuild {
                lease: LeaseRef {
                    id,
                    attempt: attempt as u32,
                },
            });
        }
    }
    sqlx::query(
        "UPDATE api_tokens
         SET revoked_at = ?
         WHERE kind = 'build'
         AND build_work IN (SELECT id
         FROM work_leases
         WHERE group_id = ?
         AND repository = ?)",
    )
    .bind(db::now())
    .bind(group)
    .bind(repository)
    .execute(&state.db)
    .await?;
    sqlx::query("DELETE FROM work_leases WHERE group_id = ? AND repository = ?")
        .bind(group)
        .bind(repository)
        .execute(&state.db)
        .await?;
    Ok(())
}

async fn publish(state: &AppState, work: i64) -> ApiResult<()> {
    let row: Option<(i64, String, String)> = sqlx::query_as(
        "SELECT group_id, COALESCE(repository,''), kind FROM work_leases WHERE id = ?",
    )
    .bind(work)
    .fetch_optional(&state.db)
    .await?;
    let Some((group, repository, kind)) = row else {
        return Ok(());
    };
    if kind == "process" {
        return Ok(());
    }
    if kind == "library" {
        let id: i64 = sqlx::query_scalar("SELECT id FROM library_builds WHERE work_id = ?")
            .bind(work)
            .fetch_one(&state.db)
            .await?;
        let (build, _) = crate::library::build(state, group, id).await?;
        state.publish(group, Event::BuildUpdated { build });
    } else if let Some(row) = crate::papers::by_repository(state, group, &repository).await? {
        state.publish(
            group,
            Event::PaperUpdated {
                paper: Box::new(row.to_api(state).await?),
            },
        );
    }
    Ok(())
}

/// On a hub restart no connection survives; all running work loses its lease.
pub async fn recover(state: &AppState) -> ApiResult<()> {
    lease::release(state, None, db::now() + lease::EXPIRY_MILLIS).await?;
    Ok(())
}
