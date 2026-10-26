//! Recipes and requests exposed by the workshop API and its tools.
use crate::{
    api::{Thread, User},
    protocol::Usage,
    recipe::{Recipe, RecipeError},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecipeFile {
    pub name: String,
    pub version: String,
    pub commit: String,
    pub url: String,
    pub recipe: Option<Recipe>,
    pub error: Option<RecipeError>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Assignment {
    Mine,
    Member { user: i64 },
    Anyone,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schedule {
    /// First slot, in Unix milliseconds. None means now.
    pub at: Option<i64>,
    /// The asker's IANA time zone, retained even for a one-off request.
    pub timezone: String,
    pub every: Option<Recurrence>,
    pub until: Option<i64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Recurrence {
    Day,
    Week,
    Hours { hours: u32 },
}
impl Schedule {
    pub fn validate(&self) -> Result<(), String> {
        self.timezone
            .parse::<chrono_tz::Tz>()
            .map_err(|_| "Use an IANA time zone, such as Europe/Madrid")?;
        if matches!(self.every, Some(Recurrence::Hours { hours: 0 })) {
            return Err("Hours must be positive".into());
        }
        if self.until.zip(self.at).is_some_and(|(u, a)| u < a) {
            return Err("The end must follow the first slot".into());
        }
        Ok(())
    }
    /// The first slot strictly after now. Missed slots coalesce into one run.
    /// Daily and weekly slots retain wall time across daylight saving changes;
    /// ambiguous wall times take the earlier instant, gaps move forward an hour.
    pub fn next_after(&self, slot: i64, now: i64) -> Option<i64> {
        use chrono::{TimeZone, Timelike};
        let zone: chrono_tz::Tz = self.timezone.parse().ok()?;
        let every = self.every.as_ref()?;
        let mut next;
        if let Recurrence::Hours { hours } = every {
            let interval = i64::from(*hours).checked_mul(3_600_000)?;
            if interval == 0 {
                return None;
            }
            next = slot.checked_add(((now - slot).max(0) / interval + 1).checked_mul(interval)?)?;
        } else {
            let days = if matches!(every, Recurrence::Week) {
                7
            } else {
                1
            };
            let start = zone.timestamp_millis_opt(slot).single()?;
            let today = zone
                .timestamp_millis_opt(now.max(slot))
                .single()?
                .date_naive();
            let elapsed = (today - start.date_naive()).num_days().max(0);
            let mut date = start.date_naive() + chrono::Duration::days(elapsed / days * days);
            loop {
                let naive = date.and_hms_opt(start.hour(), start.minute(), start.second())?;
                let local = zone.from_local_datetime(&naive).earliest().or_else(|| {
                    zone.from_local_datetime(&(naive + chrono::Duration::hours(1)))
                        .earliest()
                })?;
                next = local.timestamp_millis();
                if next > now && next > slot {
                    break;
                }
                date += chrono::Duration::days(days);
            }
        }
        self.until.is_none_or(|until| next <= until).then_some(next)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ask {
    pub recipe: String,
    pub title: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, String>,
    pub assignment: Assignment,
    /// Recipe runner handles: claude, codex or opencode.
    pub runner: String,
    pub model: Option<String>,
    pub schedule: Schedule,
    /// One hour, one day or one week, measured from each slot.
    pub wait_hours: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub thread: Thread,
    pub requester: User,
    pub recipe: RecipeFile,
    pub ask: Ask,
    pub state: String,
    pub paused: bool,
    pub next_at: Option<i64>,
    pub waiting: Option<String>,
    pub processes: Vec<Process>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Process {
    pub number: i64,
    /// A retry is a separate run on another machine.
    pub retry_of: Option<i64>,
    pub slot: i64,
    pub state: String,
    pub machine: Option<String>,
    pub owner: Option<User>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub branches: Vec<ProcessBranch>,
    pub usage: Usage,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessBranch {
    pub repository: String,
    pub branch: String,
    pub commit: Option<String>,
    pub pull_request: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missed_slots_coalesce_and_daylight_saving_keeps_wall_time() {
        use chrono::TimeZone;
        let zone = chrono_tz::Europe::Madrid;
        let slot = zone
            .with_ymd_and_hms(2026, 10, 24, 9, 0, 0)
            .unwrap()
            .timestamp_millis();
        let now = zone
            .with_ymd_and_hms(2026, 10, 27, 12, 0, 0)
            .unwrap()
            .timestamp_millis();
        let s = Schedule {
            at: Some(slot),
            timezone: "Europe/Madrid".into(),
            every: Some(Recurrence::Day),
            until: None,
        };
        assert_eq!(
            s.next_after(slot, now),
            Some(
                zone.with_ymd_and_hms(2026, 10, 28, 9, 0, 0)
                    .unwrap()
                    .timestamp_millis()
            )
        );
        assert!(
            Schedule {
                timezone: "invalid".into(),
                ..s
            }
            .validate()
            .is_err()
        );
    }
}
