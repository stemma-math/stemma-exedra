//! Approving the workshop's changes. A task's changes wait, in a pull
//! request of the forge, until those they need approve them in Stemma:
//! the person who asked for the task always, and for a big change (as its
//! architecture says: for the library, one that touches Lake's
//! configuration or the toolchain, or deletes or renames a file) an admin
//! too. One person may approve as both. Guests never approve. Approval
//! merges the pull request, and the push to `main` then builds it;
//! rejection, by the asker or an admin, closes it.
//!
//! Who approves is a list of [`Approver`]s, each a rule of who may give
//! it, so that other kinds (a paper's author, for its paper) come as more
//! rules.

use stemma_core::api::{Approval, Approver, Role, TaskState, User};

use super::{TaskRow, find, one_line, publish, say, set_state};
use crate::api::Membership;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::forge;
use crate::state::AppState;

/// The approvals a task's changes need.
pub(crate) fn required(row: &TaskRow) -> Vec<Approver> {
    let mut needed = vec![Approver::Asker];
    if row.big {
        needed.push(Approver::Admin);
    }
    needed
}

/// What someone with `role` in the group may approve a task's changes as.
pub(crate) fn may_approve_as(row: &TaskRow, user_id: i64, role: Role) -> Vec<Approver> {
    if role == Role::Guest {
        return Vec::new();
    }
    let mut kinds = Vec::new();
    if user_id == row.user_id {
        kinds.push(Approver::Asker);
    }
    if role.is_admin() {
        kinds.push(Approver::Admin);
    }
    kinds
}

/// The approvals a task's changes still need.
pub(crate) fn missing(row: &TaskRow, given: &[Approval]) -> Vec<Approver> {
    required(row)
        .into_iter()
        .filter(|kind| !given.iter().any(|a| a.approver == *kind))
        .collect()
}

#[derive(sqlx::FromRow)]
struct ApprovalRow {
    user_id: i64,
    username: String,
    display_name: String,
    approver: String,
    created_at: i64,
}

/// The approvals a task's changes were given, in order.
pub async fn approvals(state: &AppState, task_id: i64) -> ApiResult<Vec<Approval>> {
    let rows: Vec<ApprovalRow> = sqlx::query_as(
        "SELECT a.user_id, u.username, u.display_name, a.approver, a.created_at
         FROM task_approvals a JOIN users u ON u.id = a.user_id
         WHERE a.task_id = ? ORDER BY a.created_at, a.approver",
    )
    .bind(task_id)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(Approval {
                approver: Approver::parse(&row.approver)?,
                user: User {
                    id: row.user_id,
                    username: row.username,
                    display_name: row.display_name,
                },
                at: db::from_millis(row.created_at),
            })
        })
        .collect())
}

fn waits(row: &TaskRow) -> ApiResult<()> {
    if row.state() != TaskState::Review {
        return Err(ApiError::conflict(
            "This task has no changes waiting for approval",
        ));
    }
    Ok(())
}

/// Approves a task's changes, as all that the person may approve them as
/// and they still need; once they need nothing more, merges them.
pub async fn approve(
    state: &AppState,
    group: &Membership,
    user: &User,
    number: u32,
) -> ApiResult<()> {
    if group.role == Role::Guest {
        return Err(ApiError::forbidden("Guests never approve changes"));
    }
    let row = find(state, group.group_id, number).await?;
    waits(&row)?;
    let given = approvals(state, row.id).await?;
    let needed = missing(&row, &given);
    let mine = may_approve_as(&row, user.id, group.role);
    let giving: Vec<Approver> = mine
        .iter()
        .copied()
        .filter(|kind| needed.contains(kind))
        .collect();
    if giving.is_empty() {
        if given.iter().any(|a| a.user.id == user.id) {
            return Err(ApiError::conflict("You approved these changes already"));
        }
        let mut who = format!(
            "Only {} approves these changes",
            row.requester().display_name
        );
        if row.big {
            who.push_str(", and an admin, for they are big");
        }
        return Err(ApiError::forbidden(who));
    }
    let now = db::now();
    for kind in &giving {
        sqlx::query(
            "INSERT INTO task_approvals (task_id, user_id, approver, created_at)
             VALUES (?, ?, ?, ?) ON CONFLICT DO NOTHING",
        )
        .bind(row.id)
        .bind(user.id)
        .bind(kind.as_str())
        .bind(now)
        .execute(&state.db)
        .await?;
    }
    if needed.len() > giving.len() {
        publish(state, row.id).await?;
        return Ok(());
    }
    merge(state, group, &row, user).await
}

/// Merges a task's approved changes.
async fn merge(state: &AppState, group: &Membership, row: &TaskRow, user: &User) -> ApiResult<()> {
    let forge = forge::require(state)?;
    let (Some(repository), Some(pull)) = (&row.repository, row.pull) else {
        return Err(ApiError::internal("changes without their pull request"));
    };
    let message = format!(
        "w{}: {}\n\nTask: w{}\nApproved-by: {}",
        row.number,
        one_line(&row.title),
        row.number,
        user.display_name
    );
    if let Err(error) = forge
        .merge_pull(&group.slug, repository, pull as u32, &message)
        .await
    {
        tracing::warn!(%error, task = row.number, "could not merge a task's changes");
        return Err(ApiError::conflict(format!(
            "The forge could not merge these changes, as {}: reject them, and ask for the task \
             again",
            error
        )));
    }
    let merged = forge
        .pull(&group.slug, repository, pull as u32)
        .await
        .ok()
        .and_then(|p| p.merge_commit_sha);
    sqlx::query("UPDATE tasks SET merged_commit = ? WHERE id = ?")
        .bind(&merged)
        .bind(row.id)
        .execute(&state.db)
        .await?;
    set_state(state, row.id, TaskState::Done, None).await?;
    let given = approvals(state, row.id).await?;
    let mut by: Vec<String> = given.iter().map(|a| a.user.display_name.clone()).collect();
    by.dedup();
    let text = format!(
        "**Merged:** [[w{}]] is in the library, approved by {}.",
        row.number,
        by.join(" and ")
    );
    say(state, row, &text).await?;
    publish(state, row.id).await
}

/// Rejects a task's changes: the asker, or an admin. Its pull request is
/// closed and its branch deleted.
pub async fn reject(
    state: &AppState,
    group: &Membership,
    user: &User,
    number: u32,
) -> ApiResult<()> {
    if group.role == Role::Guest {
        return Err(ApiError::forbidden("Guests never reject changes"));
    }
    let row = find(state, group.group_id, number).await?;
    if row.user_id != user.id && !group.role.is_admin() {
        return Err(ApiError::forbidden(format!(
            "Only {} or an admin rejects these changes",
            row.requester().display_name
        )));
    }
    waits(&row)?;
    let forge = forge::require(state)?;
    if let (Some(repository), Some(pull)) = (&row.repository, row.pull) {
        forge
            .close_pull(&group.slug, repository, pull as u32)
            .await
            .map_err(forge::api_error)?;
        if let Some(branch) = &row.branch
            && let Err(error) = forge.delete_branch(&group.slug, repository, branch).await
        {
            tracing::warn!(%error, task = row.number, "could not delete a task's branch");
        }
    }
    sqlx::query("UPDATE tasks SET rejected_by = ? WHERE id = ?")
        .bind(user.id)
        .bind(row.id)
        .execute(&state.db)
        .await?;
    set_state(state, row.id, TaskState::Rejected, None).await?;
    let text = format!(
        "**Rejected:** [[w{}]], by {}. Nothing was put in the library.",
        row.number, user.display_name
    );
    say(state, &row, &text).await?;
    publish(state, row.id).await
}
