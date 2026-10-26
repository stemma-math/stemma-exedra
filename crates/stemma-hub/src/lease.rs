//! Durable leases shared by workshop work. Connections and attempts fence old holders.
use crate::{db, error::ApiResult, state::AppState};
use stemma_core::protocol::LeaseRef;

pub const HEARTBEAT_MILLIS: i64 = 15_000;
pub const EXPIRY_MILLIS: i64 = 60_000;
pub const MAX_ATTEMPTS: i64 = 3;

/// Claim only a queued generation, in the caller's transaction with its scoped token.
pub async fn acquire(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    lease: LeaseRef,
    holder: i64,
    connection: u64,
    token: i64,
    name: &str,
) -> ApiResult<bool> {
    Ok(sqlx::query(
        "UPDATE work_leases
         SET state = 'running', holder = ?, connection = ?, attempts = ?, heartbeat = ?,
         token_id = ?, machine = ?
         WHERE id = ? AND state = 'queued' AND attempts = ?",
    )
    .bind(holder)
    .bind(connection as i64)
    .bind(lease.attempt)
    .bind(db::now())
    .bind(token)
    .bind(name)
    .bind(lease.id)
    .bind(lease.attempt - 1)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        == 1)
}

pub async fn heartbeat(
    state: &AppState,
    holder: i64,
    connection: u64,
    lease: LeaseRef,
) -> ApiResult<()> {
    sqlx::query(
        "UPDATE work_leases
         SET heartbeat = ?
         WHERE id = ?
         AND attempts = ?
         AND holder = ?
         AND connection = ?
         AND state = 'running'
         AND heartbeat > ?",
    )
    .bind(db::now())
    .bind(lease.id)
    .bind(lease.attempt)
    .bind(holder)
    .bind(connection as i64)
    .bind(db::now() - EXPIRY_MILLIS)
    .execute(&state.db)
    .await?;
    Ok(())
}

/// Ends leases on loss of their holder, or after a missed heartbeat. The third
/// lost build lease fails permanently. A lost process ends that run and may
/// create one retry process. Tokens die in the same transaction.
pub async fn release(
    state: &AppState,
    holder: Option<(i64, u64)>,
    now: i64,
) -> ApiResult<Vec<(i64, LeaseRef)>> {
    let mut tx = state.db.begin().await?;
    let rows: Vec<(i64, i64, i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, attempts, holder, token_id
         FROM work_leases
         WHERE state = 'running'
         AND ((?1 IS NULL AND heartbeat <= ?2) OR (holder = ?1 AND connection = ?3))",
    )
    .bind(holder.map(|h| h.0))
    .bind(now - EXPIRY_MILLIS)
    .bind(holder.map(|h| h.1 as i64))
    .fetch_all(&mut *tx)
    .await?;
    let mut ended = Vec::new();
    for (id, attempt, machine, token) in rows {
        let kind: String = sqlx::query_scalar("SELECT kind FROM work_leases WHERE id=?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        let process = kind == "process";
        let failed = process || attempt >= MAX_ATTEMPTS;
        let error = if process {
            "The machine lost its process lease"
        } else {
            "The machine lost its build lease three times"
        };
        sqlx::query(
            "UPDATE work_leases
             SET state = ?, error = ?, holder = NULL, connection = NULL
             WHERE id = ?",
        )
        .bind(if failed { "failed" } else { "queued" })
        .bind(if failed { Some(error) } else { None })
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE api_tokens SET revoked_at = ? WHERE id = ?")
            .bind(now)
            .bind(token)
            .execute(&mut *tx)
            .await?;
        if failed {
            for table in ["library_builds", "paper_builds"] {
                sqlx::query(sqlx::AssertSqlSafe(format!(
                    "UPDATE {table}
                     SET succeeded = 0, error = 'The machine lost its build lease three times',
                     finished_at = ?
                     WHERE work_id = ?"
                )))
                .bind(now)
                .bind(id)
                .execute(&mut *tx)
                .await?;
            }
        }
        if process {
            crate::workshop::retry(&mut tx, id).await?;
        }
        ended.push((
            machine,
            LeaseRef {
                id,
                attempt: attempt as u32,
            },
        ));
    }
    tx.commit().await?;
    Ok(ended)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn heartbeats_expiry_reconnection_and_attempts_are_fenced() {
        let data = tempfile::tempdir().unwrap();
        let config = crate::Config {
            data_dir: data.path().to_owned(),
            ..Default::default()
        };
        let state = std::sync::Arc::new(
            crate::Hub::new(crate::db::in_memory().await.unwrap(), config).unwrap(),
        );
        sqlx::query(
            "INSERT INTO users (id, username, display_name, password_hash, subject,
             created_at)
             VALUES (1, 'ada', 'Ada', '', 'ada', 0)",
        )
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO groups (id, slug, name, created_at) VALUES (1, 'algebra', 'Algebra', 0)",
        )
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO api_tokens (id, user_id, name, token_hash, prefix, created_at)
             VALUES (1, 1, 'Machine', 'hash', 'stm_', 0)",
        )
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO work_leases (id, group_id, repository, commit_sha, kind, state,
             holder, connection, attempts, heartbeat, created_at)
             VALUES (1, 1, 'library', 'a', 'library', 'running', 1, 10, 1, ?, 0)",
        )
        .bind(db::now())
        .execute(&state.db)
        .await
        .unwrap();
        let first = LeaseRef { id: 1, attempt: 1 };
        heartbeat(&state, 1, 10, first).await.unwrap();
        assert!(
            release(&state, None, db::now() + 59_000)
                .await
                .unwrap()
                .is_empty()
        );
        // An old socket's disconnection cannot steal the new connection's lease.
        assert!(
            release(&state, Some((1, 9)), db::now())
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            release(&state, Some((1, 10)), db::now()).await.unwrap(),
            [(1, first)]
        );
        sqlx::query(
            "UPDATE work_leases
             SET state = 'running', holder = 1, connection = 11, attempts = 2, heartbeat = ?
             WHERE id = 1",
        )
        .bind(db::now())
        .execute(&state.db)
        .await
        .unwrap();
        let before: i64 = sqlx::query_scalar("SELECT heartbeat FROM work_leases WHERE id = 1")
            .fetch_one(&state.db)
            .await
            .unwrap();
        heartbeat(&state, 1, 10, first).await.unwrap();
        let after: i64 = sqlx::query_scalar("SELECT heartbeat FROM work_leases WHERE id = 1")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(before, after);
        assert_eq!(
            release(&state, None, after + EXPIRY_MILLIS).await.unwrap()[0]
                .1
                .attempt,
            2
        );
        sqlx::query(
            "UPDATE work_leases
             SET state = 'running', holder = 1, connection = 12, attempts = 3, heartbeat = ?
             WHERE id = 1",
        )
        .bind(db::now())
        .execute(&state.db)
        .await
        .unwrap();
        release(&state, Some((1, 12)), db::now()).await.unwrap();
        let (state, error): (String, String) =
            sqlx::query_as("SELECT state, error FROM work_leases WHERE id = 1")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(state, "failed");
        assert!(error.contains("three times"));
    }
}
