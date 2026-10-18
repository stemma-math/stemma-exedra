//! Agents: each person's own, and each group's own.
//!
//! A person represents all their agents: `@claude` in a message is the
//! Claude of whoever wrote it, run on their machines with their own
//! sign-in. Everyone has one agent per runner (`@claude`, `@codex`,
//! `@opencode`, see [`PersonalAgent::defaults`]), which they may change,
//! and may add more, with handles of their own, in every group they are in.
//!
//! A group has its own agents too, which run on its workers: the
//! workshop's `@workshop`. A group's agent goes first: a handle that is
//! the group's names its agent, whoever writes it.

use sqlx::FromRow;
use stemma_core::api::{AgentProfile, AgentRole, PersonalAgent, runners};

use crate::db;
use crate::error::ApiResult;
use crate::state::AppState;

// ── A group's own ─────────────────────────────────────────────────────

#[derive(FromRow)]
struct GroupAgentRow {
    handle: String,
    display_name: String,
    runner: String,
    model: Option<String>,
    args: String,
    instructions: Option<String>,
    role: String,
}

impl From<GroupAgentRow> for AgentProfile {
    fn from(row: GroupAgentRow) -> Self {
        AgentProfile {
            handle: row.handle,
            display_name: row.display_name,
            runner: row.runner,
            model: row.model,
            args: serde_json::from_str(&row.args).unwrap_or_default(),
            instructions: row.instructions,
            role: AgentRole::parse(&row.role).unwrap_or_default(),
        }
    }
}

macro_rules! group_agent_query {
    () => {
        "SELECT handle, display_name, runner, model, args, instructions, role FROM group_agents"
    };
}

/// A group's own agents, by handle.
pub async fn of_group(state: &AppState, group_id: i64) -> ApiResult<Vec<AgentProfile>> {
    let rows: Vec<GroupAgentRow> = sqlx::query_as(concat!(
        group_agent_query!(),
        " WHERE group_id = ? ORDER BY handle"
    ))
    .bind(group_id)
    .fetch_all(&state.db)
    .await?;
    Ok(rows.into_iter().map(AgentProfile::from).collect())
}

/// The group's own agent with `handle`, if it has one.
pub async fn group_agent(
    state: &AppState,
    group_id: i64,
    handle: &str,
) -> ApiResult<Option<AgentProfile>> {
    let row: Option<GroupAgentRow> = sqlx::query_as(concat!(
        group_agent_query!(),
        " WHERE group_id = ? AND handle = ?"
    ))
    .bind(group_id)
    .bind(handle)
    .fetch_optional(&state.db)
    .await?;
    Ok(row.map(AgentProfile::from))
}

/// The agents every group starts with: the workshop, on the group's
/// workers, with OpenCode.
pub fn group_defaults() -> Vec<AgentProfile> {
    vec![AgentProfile {
        handle: "workshop".into(),
        display_name: "Workshop".into(),
        runner: runners::OPENCODE.into(),
        model: None,
        args: Vec::new(),
        instructions: None,
        role: AgentRole::Workshop,
    }]
}

/// Adds an agent to a group.
pub async fn insert_group_agent<'e>(
    db: impl sqlx::SqliteExecutor<'e>,
    group_id: i64,
    profile: &AgentProfile,
) -> Result<(), sqlx::Error> {
    let now = db::now();
    sqlx::query(
        "INSERT INTO group_agents (group_id, handle, display_name, runner, model, args,
                                   instructions, role, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(group_id)
    .bind(&profile.handle)
    .bind(&profile.display_name)
    .bind(&profile.runner)
    .bind(&profile.model)
    .bind(serde_json::to_string(&profile.args).unwrap_or_else(|_| "[]".into()))
    .bind(&profile.instructions)
    .bind(profile.role.as_str())
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;
    Ok(())
}

/// Whether someone in the group has an agent of their own with `handle`.
pub async fn someone_in_group_has(
    state: &AppState,
    group_id: i64,
    handle: &str,
) -> ApiResult<bool> {
    if PersonalAgent::default_for(handle).is_some() {
        return Ok(true);
    }
    let found: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM personal_agents p JOIN memberships m ON m.user_id = p.user_id
         WHERE m.group_id = ? AND p.handle = ? LIMIT 1",
    )
    .bind(group_id)
    .bind(handle)
    .fetch_optional(&state.db)
    .await?;
    Ok(found.is_some())
}

/// The groups of a person that have an agent of their own with `handle`,
/// by name.
pub async fn groups_with_agent(
    state: &AppState,
    user_id: i64,
    handle: &str,
) -> ApiResult<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT g.name FROM group_agents a JOIN groups g ON g.id = a.group_id
              JOIN memberships m ON m.group_id = a.group_id
         WHERE m.user_id = ? AND a.handle = ? ORDER BY g.name",
    )
    .bind(user_id)
    .bind(handle)
    .fetch_all(&state.db)
    .await?)
}

// ── A person's own ────────────────────────────────────────────────────

#[derive(FromRow)]
struct PersonalAgentRow {
    handle: String,
    display_name: String,
    runner: String,
    model: Option<String>,
    args: String,
}

impl From<PersonalAgentRow> for PersonalAgent {
    fn from(row: PersonalAgentRow) -> Self {
        PersonalAgent {
            default: PersonalAgent::default_for(&row.handle).is_some(),
            handle: row.handle,
            display_name: row.display_name,
            runner: row.runner,
            model: row.model,
            args: serde_json::from_str(&row.args).unwrap_or_default(),
        }
    }
}

/// A person's own agents: those everyone has, as they changed them, and
/// theirs, the defaults first.
pub async fn of_person(state: &AppState, user_id: i64) -> ApiResult<Vec<PersonalAgent>> {
    let rows: Vec<PersonalAgentRow> = sqlx::query_as(
        "SELECT handle, display_name, runner, model, args FROM personal_agents
         WHERE user_id = ? ORDER BY handle",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    let mut stored: Vec<PersonalAgent> = rows.into_iter().map(PersonalAgent::from).collect();
    let mut agents: Vec<PersonalAgent> = PersonalAgent::defaults()
        .into_iter()
        .map(
            |default| match stored.iter().position(|a| a.handle == default.handle) {
                Some(i) => stored.remove(i),
                None => default,
            },
        )
        .collect();
    agents.extend(stored);
    Ok(agents)
}

/// A person's own agent with `handle`, if they have one.
pub async fn personal_agent(
    state: &AppState,
    user_id: i64,
    handle: &str,
) -> ApiResult<Option<PersonalAgent>> {
    let row: Option<PersonalAgentRow> = sqlx::query_as(
        "SELECT handle, display_name, runner, model, args FROM personal_agents
         WHERE user_id = ? AND handle = ?",
    )
    .bind(user_id)
    .bind(handle)
    .fetch_optional(&state.db)
    .await?;
    Ok(row
        .map(PersonalAgent::from)
        .or_else(|| PersonalAgent::default_for(handle)))
}

/// The agent a handle names for someone writing in a group: the group's
/// own, or theirs.
pub enum Summoned {
    Group(AgentProfile),
    Personal(AgentProfile),
    /// Someone else in the group has an agent with the handle, and they
    /// do not.
    Missing,
    /// Nobody does: the handle is not an agent's.
    Nothing,
}

pub async fn resolve(
    state: &AppState,
    group_id: i64,
    user_id: i64,
    handle: &str,
) -> ApiResult<Summoned> {
    if let Some(profile) = group_agent(state, group_id, handle).await? {
        return Ok(Summoned::Group(profile));
    }
    if let Some(agent) = personal_agent(state, user_id, handle).await? {
        return Ok(Summoned::Personal(agent.profile()));
    }
    if someone_in_group_has(state, group_id, handle).await? {
        return Ok(Summoned::Missing);
    }
    Ok(Summoned::Nothing)
}
