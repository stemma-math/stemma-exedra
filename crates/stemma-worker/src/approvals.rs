//! Approval authority is a local file, shared by the service, CLI and desktop.
//! No hub endpoint can create, extend or revoke an approval.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};
use stemma_core::{
    api::User,
    protocol::{Others, ProcessOffer},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pending {
    pub hub: String,
    pub owner: i64,
    pub offer: ProcessOffer,
    pub decision: String,
    pub approval: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Approval {
    pub id: u64,
    pub hub: String,
    pub owner: i64,
    pub group: i64,
    pub recipe: String,
    pub version: String,
    pub requester: i64,
    /// When set, covers only this request's recurring series.
    pub request: Option<u32>,
    pub until: Option<i64>,
    pub max_runs: Option<u32>,
    pub minutes_per_run: Option<u32>,
    pub runs: u32,
    pub revoked: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub pending: Vec<Pending>,
    #[serde(default)]
    pub approvals: Vec<Approval>,
    #[serde(default)]
    pub last_id: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Limits {
    pub until: Option<i64>,
    pub runs: Option<u32>,
    pub minutes: Option<u32>,
}
pub fn now() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}
fn lock(state: &Path) -> anyhow::Result<File> {
    fs::create_dir_all(state)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(state.join("workshop-approvals.lock"))?;
    file.lock()?;
    Ok(file)
}
fn load(state: &Path) -> anyhow::Result<Store> {
    match fs::read_to_string(state.join("workshop-approvals.toml")) {
        Ok(text) => Ok(toml::from_str(&text)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(e) => Err(e.into()),
    }
}
fn change<T>(state: &Path, f: impl FnOnce(&mut Store) -> anyhow::Result<T>) -> anyhow::Result<T> {
    let _lock = lock(state)?;
    let mut store = load(state)?;
    let out = f(&mut store)?;
    let path = state.join(format!(
        ".workshop-approvals-{}.toml",
        rand::random::<u64>()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    use std::io::Write;
    let mut file = options.open(&path)?;
    file.write_all(toml::to_string(&store)?.as_bytes())?;
    file.sync_all()?;
    fs::rename(path, state.join("workshop-approvals.toml"))?;
    Ok(out)
}
pub fn read(state: &Path) -> anyhow::Result<Store> {
    let _lock = lock(state)?;
    load(state)
}
pub fn log(state: &Path, event: &str, details: impl Serialize) -> anyhow::Result<()> {
    use std::io::Write;
    fs::create_dir_all(state.join("logs"))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(state.join("logs/workshop.jsonl"))?;
    file.lock()?;
    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({"at":now(),"event":event,"details":details}))?
    )?;
    Ok(())
}
fn covers(a: &Approval, hub: &str, owner: i64, offer: &ProcessOffer, reserved: u32) -> bool {
    !a.revoked
        && a.hub == hub
        && a.owner == owner
        && a.group == offer.group.id
        && a.recipe == offer.recipe.name
        && a.version == offer.recipe.version
        && a.requester == offer.requester.id
        && a.request
            .is_none_or(|n| offer.recurring && n == offer.request.number)
        && a.until.is_none_or(|u| u > now())
        && a.max_runs.is_none_or(|n| a.runs + reserved < n)
        && a.minutes_per_run.is_none_or(|m| {
            offer
                .recipe
                .recipe
                .as_ref()
                .is_some_and(|r| r.front.timeout_minutes <= m)
        })
}
/// Retains an immutable offer for display. The service calls this only after
/// matching locally configured group, runner, model, version and capacity.
pub fn receive(
    state: &Path,
    hub: &str,
    owner: &User,
    offer: ProcessOffer,
    others: Others,
) -> anyhow::Result<String> {
    change(state, |s| {
        s.pending
            .retain(|p| !(p.hub == hub && p.offer.process == offer.process));
        let approval = s
            .approvals
            .iter()
            .find(|a| {
                let reserved = s
                    .pending
                    .iter()
                    .filter(|p| p.approval == Some(a.id) && p.decision == "approved")
                    .count() as u32;
                covers(a, hub, owner.id, &offer, reserved)
            })
            .map(|a| a.id);
        let decision = if offer.expires_at <= now()
            || (offer.requester.id != owner.id && others == Others::Never)
        {
            "declined"
        } else if offer.requester.id == owner.id || approval.is_some() {
            "approved"
        } else {
            "pending"
        };
        log(
            state,
            decision,
            serde_json::json!({"hub":hub,"process":offer.process,"version":offer.recipe.version,"approval":approval}),
        )?;
        s.pending.push(Pending {
            hub: hub.into(),
            owner: owner.id,
            offer,
            decision: decision.into(),
            approval,
        });
        Ok(decision.into())
    })
}
pub fn approve(
    state: &Path,
    hub: &str,
    process: i64,
    mode: &str,
    limits: Limits,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        ["once", "series", "recipe"].contains(&mode),
        "Approval must be once, series or recipe"
    );
    anyhow::ensure!(
        limits.runs != Some(0)
            && limits.minutes != Some(0)
            && limits.until.is_none_or(|u| u > now()),
        "Approval limits must be positive and in the future"
    );
    change(state, |s| {
        let index = s
            .pending
            .iter()
            .position(|p| p.hub == hub && p.offer.process == process && p.decision == "pending")
            .ok_or_else(|| anyhow::anyhow!("No pending offer {process}"))?;
        let pending = &s.pending[index];
        anyhow::ensure!(pending.offer.expires_at > now(), "This offer expired");
        let mut id = None;
        if mode != "once" {
            anyhow::ensure!(
                mode != "series" || pending.offer.recurring,
                "This is not a recurring request"
            );
            let minutes = pending
                .offer
                .recipe
                .recipe
                .as_ref()
                .map(|r| r.front.timeout_minutes)
                .unwrap_or(u32::MAX);
            anyhow::ensure!(
                limits.minutes.is_none_or(|m| minutes <= m),
                "This recipe exceeds the minutes per run limit"
            );
            s.last_id += 1;
            id = Some(s.last_id);
            s.approvals.push(Approval {
                id: s.last_id,
                hub: hub.into(),
                owner: pending.owner,
                group: pending.offer.group.id,
                recipe: pending.offer.recipe.name.clone(),
                version: pending.offer.recipe.version.clone(),
                requester: pending.offer.requester.id,
                request: (mode == "series").then_some(pending.offer.request.number),
                until: limits.until,
                max_runs: limits.runs,
                minutes_per_run: limits.minutes,
                runs: 0,
                revoked: false,
            });
        }
        s.pending[index].decision = "approved".into();
        s.pending[index].approval = id;
        log(
            state,
            "approved",
            serde_json::json!({"process":process,"mode":mode,"approval":id}),
        )?;
        Ok(())
    })
}
pub fn decline(state: &Path, hub: &str, process: i64) -> anyhow::Result<()> {
    change(state, |s| {
        let p = s
            .pending
            .iter_mut()
            .find(|p| p.hub == hub && p.offer.process == process)
            .ok_or_else(|| anyhow::anyhow!("No pending offer {process}"))?;
        p.decision = "declined".into();
        log(state, "declined", process)
    })
}
pub fn revoke(state: &Path, hub: &str, id: u64) -> anyhow::Result<()> {
    change(state, |s| {
        let a = s
            .approvals
            .iter_mut()
            .find(|a| a.hub == hub && a.id == id)
            .ok_or_else(|| anyhow::anyhow!("No approval {id}"))?;
        a.revoked = true;
        for p in &mut s.pending {
            if p.approval == Some(id) && p.decision == "approved" {
                p.decision = "pending".into();
                p.approval = None;
            }
        }
        log(state, "revoked", id)
    })
}
pub fn taken(state: &Path, hub: &str, process: i64) -> anyhow::Result<()> {
    change(state, |s| {
        s.pending
            .retain(|p| !(p.hub == hub && p.offer.process == process));
        log(state, "taken", process)
    })
}
/// Check again after receiving the lease: a revoked approval, changed contract
/// or expired limit cannot race an acceptance into starting a model.
pub fn start(state: &Path, hub: &str, owner: &User, offer: &ProcessOffer) -> anyhow::Result<()> {
    change(state, |s| {
        let index = s
            .pending
            .iter()
            .position(|p| {
                p.hub == hub
                    && p.owner == owner.id
                    && p.offer.process == offer.process
                    && p.decision == "approved"
            })
            .ok_or_else(|| anyhow::anyhow!("This process has no local approval"))?;
        let p = &s.pending[index];
        anyhow::ensure!(
            serde_json::to_value(&p.offer)? == serde_json::to_value(offer)?,
            "The accepted offer changed"
        );
        if let Some(id) = p.approval {
            let a = s
                .approvals
                .iter_mut()
                .find(|a| a.id == id)
                .ok_or_else(|| anyhow::anyhow!("Approval disappeared"))?;
            anyhow::ensure!(
                covers(a, hub, owner.id, offer, 0),
                "Approval no longer covers this process"
            );
            a.runs += 1;
        }
        s.pending.remove(index);
        log(
            state,
            "started",
            serde_json::json!({"hub":hub,"process":offer.process,"owner":owner.id,"version":offer.recipe.version}),
        )?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use stemma_core::{
        api::ThreadKind,
        protocol::{GroupRef, ThreadRef},
        recipe::Recipe,
        workshop::RecipeFile,
    };
    fn person(id: i64) -> User {
        User {
            id,
            username: format!("user{id}"),
            display_name: format!("Person {id}"),
        }
    }
    fn offer(process: i64, requester: i64, version: &str) -> ProcessOffer {
        ProcessOffer {
            process,
            group: GroupRef {
                id: 1,
                slug: "algebra".into(),
                name: "Algebra".into(),
            },
            request: ThreadRef {
                number: 42,
                title: "Prove a statement".into(),
                kind: ThreadKind::Request,
            },
            requester: person(requester),
            assigned_by: person(requester),
            recipe: RecipeFile {
                name: "prove".into(),
                version: version.into(),
                commit: "commit".into(),
                url: "http://forge/algebra/recipes".into(),
                recipe: Some(
                    Recipe::parse(
                        "prove.md",
                        include_str!("../../../recipes/template/prove.md"),
                    )
                    .unwrap(),
                ),
                error: None,
            },
            inputs: Default::default(),
            prompt: "Prove True".into(),
            runner: "codex".into(),
            model: None,
            recurring: true,
            expires_at: now() + 3_600_000,
        }
    }
    #[test]
    fn simultaneous_runs_keep_each_log_record_intact() {
        let dir = tempfile::tempdir().unwrap();
        let tasks: Vec<_> = (0..4)
            .map(|writer| {
                let path = dir.path().to_owned();
                std::thread::spawn(move || {
                    for record in 0..25 {
                        log(
                            &path,
                            "test",
                            serde_json::json!({"writer":writer,"record":record}),
                        )
                        .unwrap();
                    }
                })
            })
            .collect();
        for task in tasks {
            task.join().unwrap();
        }
        let text = std::fs::read_to_string(dir.path().join("logs/workshop.jsonl")).unwrap();
        assert_eq!(text.lines().count(), 100);
        for line in text.lines() {
            serde_json::from_str::<serde_json::Value>(line).unwrap();
        }
    }
    #[test]
    fn own_requests_run_and_others_wait_for_local_approval() {
        let dir = tempfile::tempdir().unwrap();
        let owner = person(1);
        let own = offer(1, 1, "v1");
        assert_eq!(
            receive(dir.path(), "hub", &owner, own.clone(), Others::Ask).unwrap(),
            "approved"
        );
        start(dir.path(), "hub", &owner, &own).unwrap();
        let other = offer(2, 2, "v1");
        assert_eq!(
            receive(dir.path(), "hub", &owner, other.clone(), Others::Ask).unwrap(),
            "pending"
        );
        assert!(start(dir.path(), "hub", &owner, &other).is_err());
        approve(dir.path(), "hub", 2, "once", Limits::default()).unwrap();
        start(dir.path(), "hub", &owner, &other).unwrap();
        assert!(start(dir.path(), "hub", &owner, &other).is_err());
        assert!(dir.path().join("logs/workshop.jsonl").exists());
    }
    #[test]
    fn recipe_version_limits_revocation_and_never() {
        let dir = tempfile::tempdir().unwrap();
        let owner = person(1);
        let first = offer(1, 2, "v1");
        receive(dir.path(), "hub", &owner, first.clone(), Others::Ask).unwrap();
        assert!(
            approve(
                dir.path(),
                "hub",
                1,
                "recipe",
                Limits {
                    minutes: Some(1),
                    ..Default::default()
                }
            )
            .is_err()
        );
        approve(
            dir.path(),
            "hub",
            1,
            "recipe",
            Limits {
                runs: Some(2),
                minutes: Some(60),
                ..Default::default()
            },
        )
        .unwrap();
        start(dir.path(), "hub", &owner, &first).unwrap();
        assert_eq!(
            receive(dir.path(), "hub", &owner, offer(2, 2, "v2"), Others::Ask).unwrap(),
            "pending"
        );
        let next = offer(3, 2, "v1");
        assert_eq!(
            receive(dir.path(), "hub", &owner, next.clone(), Others::Ask).unwrap(),
            "approved"
        );
        // An accepted-but-not-started offer reserves the last remaining run.
        assert_eq!(
            receive(dir.path(), "hub", &owner, offer(4, 2, "v1"), Others::Ask).unwrap(),
            "pending"
        );
        start(dir.path(), "hub", &owner, &next).unwrap();
        assert_eq!(
            receive(dir.path(), "hub", &owner, offer(5, 2, "v1"), Others::Ask).unwrap(),
            "pending"
        );
        approve(dir.path(), "hub", 5, "series", Limits::default()).unwrap();
        revoke(dir.path(), "hub", 2).unwrap();
        assert!(start(dir.path(), "hub", &owner, &offer(5, 2, "v1")).is_err());
        assert_eq!(
            receive(dir.path(), "hub", &owner, offer(6, 2, "v1"), Others::Never).unwrap(),
            "declined"
        );
        assert_eq!(
            receive(
                dir.path(),
                "other-hub",
                &owner,
                offer(7, 2, "v1"),
                Others::Ask
            )
            .unwrap(),
            "pending"
        );
    }
    #[test]
    fn changed_contract_cannot_race_an_acceptance() {
        let dir = tempfile::tempdir().unwrap();
        let owner = person(1);
        let original = offer(1, 1, "v1");
        receive(dir.path(), "hub", &owner, original.clone(), Others::Ask).unwrap();
        let mut changed = original.clone();
        changed.prompt = "Different work".into();
        assert!(start(dir.path(), "hub", &owner, &changed).is_err());
        start(dir.path(), "hub", &owner, &original).unwrap();
    }
}
