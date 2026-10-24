//! Removing what holds work: a thread, a paper, a group, or an account.
//! What goes is also
//! what hangs from it, in the database, on disk and in the forge. Jobs on
//! their way are stopped first, so no machine keeps working for nothing.

use stemma_core::address::Address;
use stemma_core::api::Contents;

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

/// What deleting the threads `ids` would take with them.
async fn contents_of(state: &AppState, ids: &[i64]) -> ApiResult<Contents> {
    let mut contents = Contents::default();
    for &id in ids {
        let (messages, files): (i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM messages WHERE thread_id = ?1 AND deleted_at IS NULL),
                    (SELECT COUNT(*) FROM attachments a JOIN messages m ON m.id = a.message_id
                     WHERE m.thread_id = ?1)",
        )
        .bind(id)
        .fetch_one(&state.db)
        .await?;
        contents.messages += messages as u32;
        contents.files += files as u32;
    }
    Ok(contents)
}

/// The threads anchored to the paper `slug` of `group_id`: its own.
async fn paper_threads(state: &AppState, group_id: i64, slug: &str) -> ApiResult<Vec<i64>> {
    Ok(
        sqlx::query_scalar("SELECT id FROM threads WHERE group_id = ? AND anchor = ?")
            .bind(group_id)
            .bind(Address::Paper(slug.to_owned()).to_string())
            .fetch_all(&state.db)
            .await?,
    )
}

/// The numbers of a paper's own threads, which go with it.
pub async fn paper_thread_numbers(
    state: &AppState,
    paper: &papers::PaperRow,
) -> ApiResult<Vec<u32>> {
    Ok(
        sqlx::query_scalar("SELECT number FROM threads WHERE group_id = ? AND anchor = ?")
            .bind(paper.group_id)
            .bind(Address::Paper(paper.slug.clone()).to_string())
            .fetch_all(&state.db)
            .await?,
    )
}

/// What deleting the thread `id` would take with it.
pub async fn thread_contents(state: &AppState, id: i64) -> ApiResult<Contents> {
    contents_of(state, &[id]).await
}

/// What deleting a paper would take with it: its own threads, and theirs.
pub async fn paper_contents(state: &AppState, paper: &papers::PaperRow) -> ApiResult<Contents> {
    let threads = paper_threads(state, paper.group_id, &paper.slug).await?;
    Ok(Contents {
        threads: threads.len() as u32,
        ..contents_of(state, &threads).await?
    })
}

/// Removes the thread `id`, with its messages, their files and its jobs.
/// Its number is never given again: the group counts its threads apart.
pub async fn thread(state: &AppState, id: i64) -> ApiResult<()> {
    let waiting: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM jobs WHERE thread_id = ? AND state IN ('queued', 'running', 'held')",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    stop_jobs(state, waiting).await;
    let keys: Vec<String> = sqlx::query_scalar(
        "SELECT a.storage_key FROM attachments a JOIN messages m ON m.id = a.message_id
         WHERE m.thread_id = ?",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    sqlx::query("DELETE FROM threads WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await?;
    crate::text::refresh(state, crate::text::Changed::Thread(id)).await?;
    attachments::remove_files(state, &keys).await;
    Ok(())
}

/// Removes a paper, with its own threads, its builds and PDFs, and its
/// repository in the forge.
pub async fn paper(state: &AppState, paper: &papers::PaperRow, org: &str) -> ApiResult<()> {
    let _builds = state.build_changes.lock().await;
    for thread_id in paper_threads(state, paper.group_id, &paper.slug).await? {
        thread(state, thread_id).await?;
    }
    let leftovers = Leftovers {
        pdfs: sqlx::query_scalar(
            "SELECT id FROM paper_builds WHERE paper_id = ? AND pdf_size IS NOT NULL",
        )
        .bind(paper.id)
        .fetch_all(&state.db)
        .await?,
        repositories: vec![paper.repository.clone()],
        ..Leftovers::default()
    };
    crate::builds::forget_repository(state, paper.group_id, &paper.repository).await?;
    sqlx::query("DELETE FROM papers WHERE id = ?")
        .bind(paper.id)
        .execute(&state.db)
        .await?;
    crate::text::refresh(state, crate::text::Changed::Paper(paper.id)).await?;
    drop(_builds);
    leftovers.remove(state, org).await;
    Ok(())
}

/// Removes the group `group_id`, whose short name is `slug`, with its
/// threads, messages, papers, jobs, links and memberships; its files; and
/// its organization in the forge, with the library and the papers.
pub async fn group(state: &AppState, group_id: i64, slug: &str) -> ApiResult<()> {
    let _builds = state.build_changes.lock().await;
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

    for repository in &leftovers.repositories {
        crate::builds::forget_repository(state, group_id, repository).await?;
    }
    sqlx::query("DELETE FROM groups WHERE id = ?")
        .bind(group_id)
        .execute(&state.db)
        .await?;
    crate::text::refresh(state, crate::text::Changed::Group(group_id)).await?;
    drop(_builds);
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

    // Release the forge's current username too, preserving the external
    // account and any history under an unclaimable tombstone name.
    let (known, subject, admin): (Option<String>, String, bool) =
        sqlx::query_as("SELECT forge_login, subject, hub_admin FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_one(&state.db)
            .await?;
    if admin {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM users WHERE hub_admin = 1 AND deleted_at IS NULL",
        )
        .fetch_one(&state.db)
        .await?;
        if count <= 1 {
            return Err(ApiError::conflict("The hub needs at least one admin."));
        }
    }
    let retired = if let Some(forge) = &state.forge {
        let _guard = state.forge_sync.lock().await;
        let login = match known {
            Some(login) => Some(login),
            None => forge
                .linked_user(&subject)
                .await
                .map_err(forge::api_error)?
                .map(|u| u.login),
        };
        if let Some(login) = login {
            let retired = format!(
                "stemma-deleted-{}",
                &crate::secret::token_hash(&subject)[..20]
            );
            forge
                .rename_user(&login, &retired)
                .await
                .map_err(forge::api_error)?;
            Some(retired)
        } else {
            None
        }
    } else {
        None
    };
    let now = db::now();
    let mut tx = state.db.begin().await?;
    for statement in [
        "DELETE FROM user_names WHERE user_id = ?",
        "DELETE FROM sessions WHERE user_id = ?",
        "DELETE FROM notifications WHERE user_id = ?",
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
                          password_hash = '', hub_admin = 0, forge_login = ?, deleted_at = ?
         WHERE id = ? AND deleted_at IS NULL
           AND (hub_admin = 0
                OR (SELECT COUNT(*) FROM users WHERE hub_admin = 1 AND deleted_at IS NULL) > 1)",
    )
    .bind(retired)
    .bind(now)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::conflict("The hub needs at least one admin."));
    }
    tx.commit().await?;
    crate::text::refresh(state, crate::text::Changed::User(user_id)).await?;
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
