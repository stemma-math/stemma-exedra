//! Removing what holds work: a group, or an account. What goes is also
//! what hangs from it, in the database, on disk and in the forge. Jobs on
//! their way are stopped first, so no machine keeps working for nothing.

use crate::api::attachments;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::forge;
use crate::jobs;
use crate::papers;
use crate::state::AppState;

/// The files and repositories left once rows are gone.
#[derive(Default)]
pub struct Leftovers {
    /// Attachments, by their storage key.
    pub attachments: Vec<String>,
    /// Paper builds whose PDF is kept.
    pub pdfs: Vec<i64>,
    /// Repositories of the group's organization.
    pub repositories: Vec<String>,
}

impl Leftovers {
    /// Removes the files, and the repositories of `org`, logging what fails:
    /// the rows are already gone, and nothing points at them any more.
    pub async fn remove(self, state: &AppState, org: &str) {
        attachments::remove_files(state, &self.attachments).await;
        for build in self.pdfs {
            let path = papers::pdf_path(state, build);
            if let Err(error) = tokio::fs::remove_file(&path).await
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(%error, ?path, "could not remove a PDF");
            }
        }
        let Some(forge) = state.forge.as_ref() else {
            return;
        };
        for name in self.repositories {
            if let Err(error) = forge.delete_repository(org, &name).await {
                tracing::error!(%error, org, name, "could not remove a repository");
            }
        }
    }
}

/// Stops the jobs, among `ids`, that are not over.
pub async fn stop_jobs(state: &AppState, ids: Vec<i64>) {
    for id in ids {
        if let Err(error) = jobs::stop(state, id).await
            && error.status != axum::http::StatusCode::CONFLICT
        {
            tracing::warn!(id, message = %error.message, "could not stop a job");
        }
    }
}

/// Removes the group `group_id`, whose short name is `slug`, with its
/// threads, messages, papers, jobs, links and memberships; its files; and
/// its organization in the forge, with the library and the papers.
pub async fn group(state: &AppState, group_id: i64, slug: &str) -> ApiResult<()> {
    let waiting: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM jobs WHERE group_id = ? AND state IN ('queued', 'running', 'held')",
    )
    .bind(group_id)
    .fetch_all(&state.db)
    .await?;
    stop_jobs(state, waiting).await;

    let mut leftovers = Leftovers {
        attachments: sqlx::query_scalar("SELECT storage_key FROM attachments WHERE group_id = ?")
            .bind(group_id)
            .fetch_all(&state.db)
            .await?,
        pdfs: sqlx::query_scalar(
            "SELECT b.id FROM paper_builds b JOIN papers p ON p.id = b.paper_id
             WHERE p.group_id = ? AND b.pdf_size IS NOT NULL",
        )
        .bind(group_id)
        .fetch_all(&state.db)
        .await?,
        repositories: sqlx::query_scalar("SELECT repository FROM papers WHERE group_id = ?")
            .bind(group_id)
            .fetch_all(&state.db)
            .await?,
    };
    leftovers.repositories.push(forge::LIBRARY.into());

    sqlx::query("DELETE FROM groups WHERE id = ?")
        .bind(group_id)
        .execute(&state.db)
        .await?;
    leftovers.remove(state, slug).await;
    if let Some(forge) = state.forge.as_ref()
        && let Err(error) = forge.delete_organization(slug).await
    {
        tracing::error!(%error, org = slug, "could not remove an organization");
    }
    Ok(())
}

/// Removes the account `user_id`: its sign-in, tokens, memberships,
/// authorships and requests. What it wrote stays, by "a deleted account":
/// its row stays, with a username no one can have or sign in with.
///
/// It refuses to leave a group without an admin, or the hub without one.
pub async fn account(state: &AppState, user_id: i64) -> ApiResult<()> {
    let sole_admin: Vec<String> = sqlx::query_scalar(
        "SELECT g.name FROM groups g JOIN memberships m ON m.group_id = g.id
         WHERE m.user_id = ? AND m.role = 'admin'
           AND (SELECT COUNT(*) FROM memberships o
                WHERE o.group_id = g.id AND o.role = 'admin') = 1
         ORDER BY g.name",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    if !sole_admin.is_empty() {
        return Err(ApiError::conflict(format!(
            "This account is the only admin of {}. Make someone else an admin there first.",
            list(&sole_admin)
        )));
    }

    let waiting: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM jobs WHERE requester_id = ? AND state IN ('queued', 'running', 'held')",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    stop_jobs(state, waiting).await;
    let tokens: Vec<i64> = sqlx::query_scalar("SELECT id FROM api_tokens WHERE user_id = ?")
        .bind(user_id)
        .fetch_all(&state.db)
        .await?;
    for token in tokens {
        jobs::disconnect(state, token).await?;
    }
    let groups: Vec<i64> = sqlx::query_scalar("SELECT group_id FROM memberships WHERE user_id = ?")
        .bind(user_id)
        .fetch_all(&state.db)
        .await?;

    let now = db::now();
    let mut tx = state.db.begin().await?;
    for statement in [
        "DELETE FROM sessions WHERE user_id = ?",
        "DELETE FROM api_tokens WHERE user_id = ?",
        "DELETE FROM memberships WHERE user_id = ?",
        "DELETE FROM paper_authors WHERE user_id = ?",
        "DELETE FROM oidc_codes WHERE user_id = ?",
        "DELETE FROM oidc_tokens WHERE user_id = ?",
        "DELETE FROM group_requests WHERE user_id = ? AND status = 'open'",
    ] {
        sqlx::query(statement)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query(
        "UPDATE invitations SET revoked_at = ? WHERE created_by = ? AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    // A single write keeps the hub with an admin even when two are deleted
    // at once.
    let done = sqlx::query(
        "UPDATE users SET username = '~' || id, display_name = 'A deleted account',
                          password_hash = '', hub_admin = 0, deleted_at = ?
         WHERE id = ? AND deleted_at IS NULL
           AND (hub_admin = 0
                OR (SELECT COUNT(*) FROM users WHERE hub_admin = 1 AND deleted_at IS NULL) > 1)",
    )
    .bind(now)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::conflict("The hub needs at least one admin."));
    }
    tx.commit().await?;
    for group in groups {
        forge::sync_members_or_log(state, group).await;
    }
    Ok(())
}

/// Names joined as people write them: "a", "a and b", "a, b and c".
fn list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::list;

    #[test]
    fn lists_names() {
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(list(&names(&["Algebra"])), "Algebra");
        assert_eq!(list(&names(&["Algebra", "Logic"])), "Algebra and Logic");
        assert_eq!(list(&names(&["A", "B", "C"])), "A, B and C");
    }
}
