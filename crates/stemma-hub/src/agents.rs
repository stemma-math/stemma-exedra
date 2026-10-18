//! People's agents, and the group's workshop.
//!
//! A person represents all their agents: `@claude` in a message is the
//! Claude of whoever wrote it, run on their machines with their own
//! sign-in. Everyone has one agent per runner (`@claude`, `@codex`,
//! `@opencode`, see [`PersonalAgent::defaults`]), which they may change,
//! and may add more, with handles of their own, in every group they are in.
//!
//! Every group has its workshop, `@workshop`: a message that mentions it
//! hands the workshop a task (see [`crate::workshop`]). Nobody's own agent
//! takes its handle.

use sqlx::FromRow;
use stemma_core::api::{AgentProfile, PersonalAgent, WORKSHOP};

use crate::error::ApiResult;
use crate::state::AppState;

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

/// What a handle names for someone writing in a group.
pub enum Summoned {
    /// The group's workshop.
    Workshop,
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
    if handle == WORKSHOP {
        return Ok(Summoned::Workshop);
    }
    if let Some(agent) = personal_agent(state, user_id, handle).await? {
        return Ok(Summoned::Personal(agent.profile()));
    }
    if someone_in_group_has(state, group_id, handle).await? {
        return Ok(Summoned::Missing);
    }
    Ok(Summoned::Nothing)
}
