//! Resolve prose on writes and render it at every read boundary.
use crate::error::ApiResult;
use crate::state::AppState;
use sqlx::SqliteConnection;
use std::collections::HashMap;
use stemma_core::text::{Object, StoredText, Target};
use stemma_core::{Address, Located};

pub async fn encode(db: &mut SqliteConnection, group: i64, text: &str) -> ApiResult<String> {
    let mut mentions = HashMap::new();
    for (_, handle) in stemma_core::mention::mention_spans(text) {
        let target = if stemma_core::api::AgentProfile::default_for(&handle).is_some() {
            Some(Target::Agent(handle.clone()))
        } else {
            sqlx::query_scalar::<_, i64>("SELECT id FROM users WHERE username = ?1 AND deleted_at IS NULL UNION SELECT u.id FROM user_names n JOIN users u ON u.id = n.user_id WHERE n.name = ?1 AND u.deleted_at IS NULL")
                .bind(&handle).fetch_optional(&mut *db).await?.map(Target::User)
        };
        if let Some(target) = target {
            mentions.insert(handle, target);
        }
    }
    let mut refs = HashMap::new();
    for found in stemma_core::references(text) {
        let Ok(located) = stemma_core::locate(found.raw) else {
            continue;
        };
        if let Some(target) = resolve(db, group, &located).await? {
            refs.insert(found.raw.trim().to_owned(), target);
        }
    }
    let stored = StoredText::resolve(
        text,
        |h| mentions.get(h).cloned(),
        |r| {
            let key = match &r.group {
                Some(g) => format!("{g}/{}", r.address),
                None => r.address.to_string(),
            };
            refs.get(&key).cloned().or_else(|| {
                refs.iter()
                    .find(|(raw, _)| stemma_core::locate(raw).as_ref() == Ok(r))
                    .map(|(_, t)| t.clone())
            })
        },
    );
    Ok(serde_json::to_string(&stored).expect("stored text serializes"))
}
async fn resolve(
    db: &mut SqliteConnection,
    default: i64,
    located: &Located,
) -> ApiResult<Option<Target>> {
    let group = match &located.group {
        Some(slug) => match sqlx::query_scalar("SELECT id FROM groups WHERE slug = ?1 UNION SELECT group_id FROM group_names WHERE name = ?1").bind(slug).fetch_optional(&mut *db).await? {
            Some(id) => id, None => return Ok(None),
        }, None => default,
    };
    let object = match &located.address {
        Address::Paper(slug) => {
            sqlx::query_scalar("SELECT id FROM papers WHERE group_id = ? AND slug = ?")
                .bind(group)
                .bind(slug)
                .fetch_optional(&mut *db)
                .await?
                .map(Object::Paper)
        }
        Address::Declaration(name) => {
            let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM library_declarations d JOIN library_builds b ON b.id = d.build_id WHERE b.group_id = ? AND d.name = ?)")
                .bind(group).bind(name).fetch_one(&mut *db).await?;
            exists.then(|| Object::Declaration(name.clone()))
        }
        a => {
            let row: Option<(i64, i64)> = sqlx::query_as(
                "SELECT id, last_message FROM threads WHERE group_id = ? AND number = ?",
            )
            .bind(group)
            .bind(a.thread().unwrap())
            .fetch_optional(&mut *db)
            .await?;
            row.filter(|(_, last)| match a {
                Address::Message { message, .. } | Address::Attachment { message, .. } => {
                    i64::from(*message) <= *last
                }
                Address::Messages { to, .. } => i64::from(*to) <= *last,
                _ => true,
            })
            .map(|(id, _)| Object::Thread {
                id,
                address: a.clone(),
            })
        }
    };
    Ok(object.map(|object| Target::Reference {
        group,
        object,
        qualified: located.group.is_some(),
    }))
}

pub async fn render(state: &AppState, stored: &str) -> ApiResult<(String, Vec<String>)> {
    render_with(&mut *state.db.acquire().await?, stored).await
}
pub async fn render_with(
    db: &mut SqliteConnection,
    stored: &str,
) -> ApiResult<(String, Vec<String>)> {
    let Ok(stored) = serde_json::from_str::<StoredText>(stored) else {
        return Ok((stored.into(), Vec::new()));
    };
    let mut names = Vec::new();
    for target in stored.targets() {
        if names.iter().any(|(t, _)| t == target) {
            continue;
        }
        names.push((target.clone(), current(db, target).await?));
    }
    Ok(stored.render(|target| {
        names
            .iter()
            .find(|(t, _)| t == target)
            .and_then(|(_, n)| n.clone())
    }))
}
async fn current(db: &mut SqliteConnection, target: &Target) -> ApiResult<Option<String>> {
    Ok(match target {
        Target::User(id) => sqlx::query_as::<_, (String, String, Option<i64>)>(
            "SELECT username, display_name, deleted_at FROM users WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&mut *db)
        .await?
        .map(|(username, name, deleted)| {
            if deleted.is_some() {
                name
            } else {
                format!("@{username}")
            }
        }),
        Target::Agent(kind) => Some(format!("@{kind}")),
        Target::Reference {
            group,
            object,
            qualified,
        } => {
            let Some(slug): Option<String> =
                sqlx::query_scalar("SELECT slug FROM groups WHERE id = ?")
                    .bind(group)
                    .fetch_optional(&mut *db)
                    .await?
            else {
                return Ok(None);
            };
            let address = match object {
                Object::Paper(id) => sqlx::query_scalar::<_, String>(
                    "SELECT slug FROM papers WHERE id = ? AND group_id = ?",
                )
                .bind(id)
                .bind(group)
                .fetch_optional(&mut *db)
                .await?
                .map(Address::Paper),
                Object::Thread { id, address } => {
                    let exists: bool = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM threads WHERE id = ? AND group_id = ?)",
                    )
                    .bind(id)
                    .bind(group)
                    .fetch_one(&mut *db)
                    .await?;
                    exists.then(|| address.clone())
                }
                Object::Declaration(name) => Some(Address::Declaration(name.clone())),
            };
            address.map(|address| {
                if *qualified {
                    format!("[[{slug}/{address}]]")
                } else {
                    format!("[[{address}]]")
                }
            })
        }
    })
}

/// Materialized prose feeds SQLite's search index. IDs remain the source.
pub async fn refresh(state: &AppState) -> ApiResult<()> {
    for (table, text, ids) in [
        ("messages", "body", "body_ids"),
        ("threads", "title", "title_ids"),
        ("papers", "title", "title_ids"),
        ("group_requests", "purpose", "purpose_ids"),
    ] {
        let rows: Vec<(i64, String)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT id, {ids} FROM {table} WHERE {ids} IS NOT NULL"
        )))
        .fetch_all(&state.db)
        .await?;
        for (id, stored) in rows {
            let (rendered, _) = render(state, &stored).await?;
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "UPDATE {table} SET {text} = ? WHERE id = ? AND {text} != ?"
            )))
            .bind(&rendered)
            .bind(id)
            .bind(&rendered)
            .execute(&state.db)
            .await?;
        }
    }
    Ok(())
}

pub async fn names_changed(state: &AppState) -> ApiResult<()> {
    refresh(state).await?;
    let groups: Vec<i64> = sqlx::query_scalar("SELECT id FROM groups")
        .fetch_all(&state.db)
        .await?;
    for id in groups {
        state.publish(id, stemma_core::api::Event::NamesChanged);
    }
    Ok(())
}
