//! The workshop's machines: a group's admins add machines of their own,
//! and each machine's owner keeps which agents it offers the workshop, in
//! order of preference, each with a daily limit. A task goes to the first
//! agent, on a free machine, that the machine can run and that is under
//! its limit.

use std::collections::HashSet;

use sqlx::FromRow;
use stemma_core::api::{Isolation, User, WorkshopAgent, WorkshopMachine, runners};

use crate::api::Membership;
use crate::api::agents::{args_of, optional, runner_of};
use crate::consumption;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::jobs;
use crate::state::AppState;

/// The most agents a machine offers the workshop.
const MAX_AGENTS: usize = 12;

/// One of a machine's agents for the workshop, as kept.
#[derive(Clone, Debug, FromRow)]
pub struct AgentRow {
    pub id: i64,
    pub runner: String,
    pub model: Option<String>,
    pub args: String,
    pub daily_jobs: Option<i64>,
    pub daily_minutes: Option<i64>,
}

impl AgentRow {
    pub fn args(&self) -> Vec<String> {
        serde_json::from_str(&self.args).unwrap_or_default()
    }

    fn to_api(&self, today: stemma_core::api::AgentToday) -> WorkshopAgent {
        WorkshopAgent {
            id: Some(self.id),
            runner: self.runner.clone(),
            model: self.model.clone(),
            args: self.args(),
            daily_jobs: self.daily_jobs.map(|n| n as u32),
            daily_minutes: self.daily_minutes.map(|n| n as u32),
            today: Some(today),
        }
    }

    /// Whether it may run one more job today.
    async fn under_limit(&self, state: &AppState) -> ApiResult<bool> {
        if self.daily_jobs.is_none() && self.daily_minutes.is_none() {
            return Ok(true);
        }
        let today = consumption::agent_today(state, self.id).await?;
        Ok(self.daily_jobs.is_none_or(|n| i64::from(today.jobs) < n)
            && self
                .daily_minutes
                .is_none_or(|n| i64::from(today.minutes) < n))
    }
}

async fn rows(state: &AppState, worker_id: i64) -> ApiResult<Vec<AgentRow>> {
    Ok(sqlx::query_as(
        "SELECT id, runner, model, args, daily_jobs, daily_minutes FROM workshop_agents
         WHERE worker_id = ? ORDER BY position",
    )
    .bind(worker_id)
    .fetch_all(&state.db)
    .await?)
}

/// A machine's agents for the workshop, in order, with what each did
/// today.
pub async fn agents(state: &AppState, worker_id: i64) -> ApiResult<Vec<WorkshopAgent>> {
    let mut agents = Vec::new();
    for row in rows(state, worker_id).await? {
        let today = consumption::agent_today(state, row.id).await?;
        agents.push(row.to_api(today));
    }
    Ok(agents)
}

/// The agent a machine runs a task with: the first, in its owner's order,
/// whose runner it has and that is under its daily limit.
pub async fn pick(
    state: &AppState,
    worker_id: i64,
    offered: &[String],
) -> ApiResult<Option<AgentRow>> {
    for row in rows(state, worker_id).await? {
        if offered.contains(&row.runner) && row.under_limit(state).await? {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

fn limit(value: Option<u32>, what: &str) -> ApiResult<Option<i64>> {
    match value {
        Some(0) => Err(ApiError::invalid(format!(
            "A daily limit of {what} is at least 1; leave it out for none"
        ))),
        Some(n) => Ok(Some(i64::from(n))),
        None => Ok(None),
    }
}

/// Sets a machine's agents for the workshop, in order: those with an id
/// are changed, the others added, and those left out removed. Only the
/// machine's owner does.
pub async fn set_agents(
    state: &AppState,
    owner_id: i64,
    worker_id: i64,
    agents: Vec<WorkshopAgent>,
) -> ApiResult<Vec<WorkshopAgent>> {
    let owner: Option<i64> =
        sqlx::query_scalar("SELECT owner_id FROM workers WHERE id = ? AND revoked_at IS NULL")
            .bind(worker_id)
            .fetch_optional(&state.db)
            .await?;
    if owner != Some(owner_id) {
        return Err(ApiError::not_found("Machine"));
    }
    if agents.len() > MAX_AGENTS {
        return Err(ApiError::invalid(format!(
            "A machine offers the workshop {MAX_AGENTS} agents at most"
        )));
    }
    let existing: HashSet<i64> = rows(state, worker_id)
        .await?
        .into_iter()
        .map(|r| r.id)
        .collect();
    let now = db::now();
    let mut kept = HashSet::new();
    let mut tx = state.db.begin().await?;
    // Out of the way of the new order first.
    sqlx::query("UPDATE workshop_agents SET position = -1 - position WHERE worker_id = ?")
        .bind(worker_id)
        .execute(&mut *tx)
        .await?;
    for (position, agent) in agents.into_iter().enumerate() {
        let runner = runner_of(&agent.runner)?;
        if !runners::WORKSHOP.contains(&runner.as_str()) {
            return Err(ApiError::invalid(format!(
                "{} does not work for the workshop",
                runners::name(&runner)
            )));
        }
        let model = optional(agent.model, "The model", 200)?;
        let args = serde_json::to_string(&args_of(agent.args)?).map_err(ApiError::internal)?;
        let jobs = limit(agent.daily_jobs, "jobs")?;
        let minutes = limit(agent.daily_minutes, "minutes")?;
        match agent.id {
            Some(id) if existing.contains(&id) => {
                sqlx::query(
                    "UPDATE workshop_agents SET position = ?, runner = ?, model = ?, args = ?,
                                                daily_jobs = ?, daily_minutes = ?, updated_at = ?
                     WHERE id = ?",
                )
                .bind(position as i64)
                .bind(&runner)
                .bind(&model)
                .bind(&args)
                .bind(jobs)
                .bind(minutes)
                .bind(now)
                .bind(id)
                .execute(&mut *tx)
                .await?;
                kept.insert(id);
            }
            Some(_) => return Err(ApiError::not_found("Workshop agent")),
            None => {
                sqlx::query(
                    "INSERT INTO workshop_agents (worker_id, position, runner, model, args,
                                                  daily_jobs, daily_minutes, created_at,
                                                  updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(worker_id)
                .bind(position as i64)
                .bind(&runner)
                .bind(&model)
                .bind(&args)
                .bind(jobs)
                .bind(minutes)
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            }
        }
    }
    for gone in existing.difference(&kept) {
        sqlx::query("DELETE FROM workshop_agents WHERE id = ?")
            .bind(gone)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    jobs::dispatch(state).await?;
    self::agents(state, worker_id).await
}

/// The groups whose workshop a machine works for, by id.
pub async fn groups_of(state: &AppState, worker_id: i64) -> ApiResult<HashSet<i64>> {
    Ok(
        sqlx::query_scalar("SELECT group_id FROM workshop_machines WHERE worker_id = ?")
            .bind(worker_id)
            .fetch_all(&state.db)
            .await?
            .into_iter()
            .collect(),
    )
}

/// The same, by slug.
pub async fn group_slugs_of(state: &AppState, worker_id: i64) -> ApiResult<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT g.slug FROM workshop_machines w JOIN groups g ON g.id = w.group_id
         WHERE w.worker_id = ? ORDER BY g.slug",
    )
    .bind(worker_id)
    .fetch_all(&state.db)
    .await?)
}

/// Makes one of an admin's own machines work for the group's workshop.
pub async fn add(
    state: &AppState,
    group: &Membership,
    viewer_id: i64,
    worker_id: i64,
) -> ApiResult<()> {
    group.require_admin()?;
    let found: Option<(i64, String, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT owner_id, name, last_seen_at, isolation FROM workers
         WHERE id = ? AND revoked_at IS NULL",
    )
    .bind(worker_id)
    .fetch_optional(&state.db)
    .await?;
    let Some((owner, name, seen, isolation)) = found else {
        return Err(ApiError::not_found("Machine"));
    };
    if owner != viewer_id {
        return Err(ApiError::forbidden(
            "An admin adds only their own machines to the workshop",
        ));
    }
    refuse_unisolated(&name, seen.is_some(), isolation.as_deref())?;
    let done = sqlx::query(
        "INSERT INTO workshop_machines (group_id, worker_id, added_by, added_at)
         VALUES (?, ?, ?, ?) ON CONFLICT DO NOTHING",
    )
    .bind(group.group_id)
    .bind(worker_id)
    .bind(viewer_id)
    .bind(db::now())
    .execute(&state.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::conflict(
            "This machine works for the workshop already",
        ));
    }
    jobs::workshop_changed(state, worker_id).await
}

/// Refuses a machine that cannot isolate the workshop's jobs, as it said
/// when it last connected: the workshop runs work others ask for, with
/// agents that run commands, each in a container of its own, unless the
/// machine's owner chose explicitly to run them on it.
fn refuse_unisolated(name: &str, seen: bool, isolation: Option<&str>) -> ApiResult<()> {
    if isolation.and_then(Isolation::parse).is_some() {
        return Ok(());
    }
    if !seen {
        return Err(ApiError::conflict(format!(
            "“{name}” has not connected yet, so it is not known how it would isolate the \
             workshop's jobs: serve it (`stemma machine run`) first"
        )));
    }
    Err(ApiError::conflict(format!(
        "“{name}” cannot isolate the workshop's jobs: it has no working container runtime \
         (Docker or Podman) with the workshop's image (`stemma machine workshop image`), and its \
         owner has not chosen to run them on it unisolated (`stemma machine workshop isolation \
         --unisolated`)"
    )))
}

/// Takes a machine from the group's workshop: an admin of the group, or
/// the machine's owner.
pub async fn remove(
    state: &AppState,
    group: &Membership,
    viewer_id: i64,
    worker_id: i64,
) -> ApiResult<()> {
    let owner: Option<i64> = sqlx::query_scalar(
        "SELECT k.owner_id FROM workshop_machines w JOIN workers k ON k.id = w.worker_id
         WHERE w.group_id = ? AND w.worker_id = ?",
    )
    .bind(group.group_id)
    .bind(worker_id)
    .fetch_optional(&state.db)
    .await?;
    let Some(owner) = owner else {
        return Err(ApiError::not_found("Workshop machine"));
    };
    if owner != viewer_id {
        group.require_admin()?;
    }
    sqlx::query("DELETE FROM workshop_machines WHERE group_id = ? AND worker_id = ?")
        .bind(group.group_id)
        .bind(worker_id)
        .execute(&state.db)
        .await?;
    jobs::workshop_changed(state, worker_id).await
}

#[derive(FromRow)]
struct MachineRow {
    id: i64,
    name: String,
    owner_id: i64,
    username: String,
    display_name: String,
    lean: bool,
    latex: bool,
    isolation: Option<String>,
    runners: String,
    added_at: i64,
}

/// The machines of a group's workshop, with their agents.
pub async fn of_group(state: &AppState, group_id: i64) -> ApiResult<Vec<WorkshopMachine>> {
    let rows: Vec<MachineRow> = sqlx::query_as(
        "SELECT k.id, k.name, k.owner_id, u.username, u.display_name, k.lean, k.latex,
                k.isolation, k.runners, w.added_at
         FROM workshop_machines w JOIN workers k ON k.id = w.worker_id
              JOIN users u ON u.id = k.owner_id
         WHERE w.group_id = ? AND k.revoked_at IS NULL ORDER BY w.added_at",
    )
    .bind(group_id)
    .fetch_all(&state.db)
    .await?;
    let mut machines = Vec::with_capacity(rows.len());
    for row in rows {
        let online = state.workers.lock().await.is_online(row.id);
        machines.push(WorkshopMachine {
            id: row.id,
            name: row.name,
            owner: User {
                id: row.owner_id,
                username: row.username,
                display_name: row.display_name,
            },
            online,
            lean: row.lean,
            latex: row.latex,
            isolation: row.isolation.as_deref().and_then(Isolation::parse),
            runners: serde_json::from_str(&row.runners).unwrap_or_default(),
            agents: agents(state, row.id).await?,
            added_at: db::from_millis(row.added_at),
        });
    }
    Ok(machines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_machines_that_do_not_isolate() {
        assert!(refuse_unisolated("server", true, Some("docker")).is_ok());
        assert!(refuse_unisolated("server", true, Some("podman")).is_ok());
        // The owner's explicit choice.
        assert!(refuse_unisolated("server", true, Some("host")).is_ok());
        let error = refuse_unisolated("server", true, None).unwrap_err();
        assert!(error.message.contains("--unisolated"), "{}", error.message);
        let error = refuse_unisolated("server", false, None).unwrap_err();
        assert!(
            error.message.contains("stemma machine run"),
            "{}",
            error.message
        );
        assert!(refuse_unisolated("server", true, Some("chroot")).is_err());
    }
}
