//! What jobs consume: each time a job runs on a machine, how long it took
//! and, when its runner says, its tokens and cost, by job, by workshop
//! agent and by machine. The workshop's daily limits are counted from it,
//! and its panel shows it, day by day.
//!
//! Days are UTC days.

use std::collections::BTreeMap;

use stemma_core::api::{AgentToday, Consumption, ConsumptionDay, ConsumptionRow, JobKind};
use stemma_core::protocol::Usage;

use crate::db;
use crate::error::ApiResult;
use crate::state::AppState;

const DAY_MILLIS: i64 = 24 * 60 * 60 * 1000;

/// When today began, in UTC.
pub fn today_start(now: i64) -> i64 {
    now - now.rem_euclid(DAY_MILLIS)
}

/// A run that starts.
pub struct Start<'a> {
    pub job_id: i64,
    pub group_id: i64,
    pub kind: JobKind,
    pub worker_id: i64,
    pub workshop_agent_id: Option<i64>,
    pub runner: &'a str,
    pub model: Option<&'a str>,
}

/// Keeps that a job started on a machine.
pub async fn started(state: &AppState, start: Start<'_>) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO job_runs (job_id, group_id, kind, worker_id, machine, workshop_agent_id,
                               runner, model, started_at)
         SELECT ?, ?, ?, id, name, ?, ?, ?, ? FROM workers WHERE id = ?",
    )
    .bind(start.job_id)
    .bind(start.group_id)
    .bind(start.kind.as_str())
    .bind(start.workshop_agent_id)
    .bind(start.runner)
    .bind(start.model)
    .bind(db::now())
    .bind(start.worker_id)
    .execute(&state.db)
    .await?;
    Ok(())
}

/// Keeps how a job's run ended (`done`, `failed`, `cancelled` or
/// `requeued`), with what its runner said it consumed.
pub async fn ended(
    state: &AppState,
    job_id: i64,
    outcome: &str,
    usage: Option<&Usage>,
) -> ApiResult<()> {
    let usage = usage.cloned().unwrap_or_default();
    sqlx::query(
        "UPDATE job_runs SET finished_at = ?, outcome = ?, input_tokens = ?, output_tokens = ?,
                             cache_read_tokens = ?, cache_write_tokens = ?, cost_usd = ?
         WHERE job_id = ? AND finished_at IS NULL",
    )
    .bind(db::now())
    .bind(outcome)
    .bind(usage.input_tokens.map(|t| t as i64))
    .bind(usage.output_tokens.map(|t| t as i64))
    .bind(usage.cache_read_tokens.map(|t| t as i64))
    .bind(usage.cache_write_tokens.map(|t| t as i64))
    .bind(usage.cost_usd)
    .bind(job_id)
    .execute(&state.db)
    .await?;
    Ok(())
}

/// What a workshop agent did today: its runs, requeued ones aside, and
/// their minutes, those still running counted until now.
pub async fn agent_today(state: &AppState, agent_id: i64) -> ApiResult<AgentToday> {
    let now = db::now();
    let (jobs, millis): (i64, Option<i64>) = sqlx::query_as(
        "SELECT COUNT(*), SUM(coalesce(finished_at, ?1) - started_at) FROM job_runs
         WHERE workshop_agent_id = ?2 AND started_at >= ?3
           AND coalesce(outcome, '') != 'requeued'",
    )
    .bind(now)
    .bind(agent_id)
    .bind(today_start(now))
    .fetch_one(&state.db)
    .await?;
    Ok(AgentToday {
        jobs: jobs as u32,
        minutes: (millis.unwrap_or(0).max(0) / 60_000) as u32,
    })
}

#[derive(sqlx::FromRow)]
struct DayRow {
    day: String,
    machine: String,
    runner: String,
    model: Option<String>,
    jobs: i64,
    millis: Option<i64>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_read_tokens: Option<i64>,
    cache_write_tokens: Option<i64>,
    cost_usd: Option<f64>,
}

fn add(total: &mut Consumption, row: &Consumption) {
    fn sum<T: Copy + std::ops::Add<Output = T>>(a: Option<T>, b: Option<T>) -> Option<T> {
        match (a, b) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        }
    }
    total.jobs += row.jobs;
    total.seconds += row.seconds;
    total.input_tokens = sum(total.input_tokens, row.input_tokens);
    total.output_tokens = sum(total.output_tokens, row.output_tokens);
    total.cache_read_tokens = sum(total.cache_read_tokens, row.cache_read_tokens);
    total.cache_write_tokens = sum(total.cache_write_tokens, row.cache_write_tokens);
    total.cost_usd = sum(total.cost_usd, row.cost_usd);
}

/// What a group's workshop consumed, its tasks' runs and its builds, in
/// each of the last `days` days, the newest first.
pub async fn workshop_days(
    state: &AppState,
    group_id: i64,
    days: u32,
) -> ApiResult<Vec<ConsumptionDay>> {
    let now = db::now();
    let since = today_start(now) - i64::from(days.saturating_sub(1)) * DAY_MILLIS;
    let rows: Vec<DayRow> = sqlx::query_as(
        "SELECT strftime('%Y-%m-%d', started_at / 1000, 'unixepoch') AS day, machine, runner,
                model, COUNT(*) AS jobs, SUM(coalesce(finished_at, ?1) - started_at) AS millis,
                SUM(input_tokens) AS input_tokens, SUM(output_tokens) AS output_tokens,
                SUM(cache_read_tokens) AS cache_read_tokens,
                SUM(cache_write_tokens) AS cache_write_tokens, SUM(cost_usd) AS cost_usd
         FROM job_runs
         WHERE group_id = ?2 AND kind IN ('task_run', 'library_build') AND started_at >= ?3
         GROUP BY day, machine, runner, model
         ORDER BY day DESC, machine, runner, model",
    )
    .bind(now)
    .bind(group_id)
    .bind(since)
    .fetch_all(&state.db)
    .await?;
    let mut by_day: BTreeMap<String, ConsumptionDay> = BTreeMap::new();
    for row in rows {
        let consumption = Consumption {
            jobs: row.jobs as u32,
            seconds: (row.millis.unwrap_or(0).max(0) / 1000) as u64,
            input_tokens: row.input_tokens.map(|t| t as u64),
            output_tokens: row.output_tokens.map(|t| t as u64),
            cache_read_tokens: row.cache_read_tokens.map(|t| t as u64),
            cache_write_tokens: row.cache_write_tokens.map(|t| t as u64),
            cost_usd: row.cost_usd,
        };
        let day = by_day
            .entry(row.day.clone())
            .or_insert_with(|| ConsumptionDay {
                day: row.day.clone(),
                total: Consumption::default(),
                rows: Vec::new(),
            });
        add(&mut day.total, &consumption);
        day.rows.push(ConsumptionRow {
            machine: row.machine,
            runner: row.runner,
            model: row.model,
            consumption,
        });
    }
    Ok(by_day.into_values().rev().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_begin_at_midnight_utc() {
        // 2026-10-18T13:45:00Z
        let now = 1_792_331_100_000;
        assert_eq!(today_start(now), 1_792_281_600_000);
        assert_eq!(today_start(today_start(now)), today_start(now));
    }
}
