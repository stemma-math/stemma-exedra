//! The forge: a Forgejo instance beside the hub, which keeps the groups'
//! repositories.
//!
//! Each group is an organization of the same name, with a team per role:
//! its admins are its owners, its members write to its repositories, and
//! its guests read them. Each person has one account, Stemma's: the hub
//! makes their Forgejo account, linked to the authentication source that
//! signs people in through the hub (see [`crate::oidc`]), and keeps the
//! teams as the group's roles are. The hub acts in Forgejo as an admin,
//! with a token that never leaves it.
//!
//! A group's library is the repository `library` of its organization, made
//! from the template when the group is. A webhook tells the hub of each
//! push to its `main` (see [`hook`]); and the hub serves its git to workers
//! and personal tokens, passing it on to Forgejo (see [`crate::git`]).

pub mod client;
pub mod hook;

use std::collections::{HashMap, HashSet};

use axum::http::StatusCode;
use stemma_core::api::Role;

pub use client::{Forge, ForgeError, ForgeResult};
use client::{NewUser, Permission};

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// The repository of a group's library.
pub const LIBRARY: &str = "library";

/// The branch that is built.
pub const MAIN: &str = "main";

/// Where Forgejo's webhooks reach the hub.
pub const HOOK_PATH: &str = "/forge/hook";

/// The team of a role in a group's organization.
pub fn team_name(role: Role) -> &'static str {
    match role {
        Role::Admin => "Owners",
        Role::Member => "Members",
        Role::Guest => "Guests",
    }
}

const ROLES: [Role; 3] = [Role::Admin, Role::Member, Role::Guest];

/// The address a person's Forgejo account has, which goes nowhere: Stemma
/// keeps no addresses.
pub fn email_of(username: &str) -> String {
    format!("{username}@stemma.invalid")
}

/// Names Forgejo keeps for itself, which neither an account nor a group
/// can have.
pub fn is_reserved(name: &str) -> bool {
    const RESERVED: &[&str] = &[
        "admin",
        "worker",
        "workshop",
        "g",
        "git",
        "groups",
        "hub",
        "invite",
        "settings",
        "setup",
        "sign-up",
        "logout",
        "me",
        "u",
        "api",
        "assets",
        "attachments",
        "avatar",
        "avatars",
        "captcha",
        "commits",
        "debug",
        "devtest",
        "error",
        "explore",
        "forgejo-actions",
        "ghost",
        "gitea-actions",
        "issues",
        "login",
        "metrics",
        "milestones",
        "new",
        "notifications",
        "org",
        "pulls",
        "raw",
        "repo",
        "repo-avatars",
        "search",
        "ssh_info",
        "user",
        "v2",
    ];
    const RESERVED_SUFFIXES: &[&str] = &[".keys", ".gpg", ".rss", ".atom", ".png"];
    RESERVED.contains(&name)
        || name.starts_with("stemma-deleted-")
        || RESERVED_SUFFIXES.iter().any(|s| name.ends_with(s))
}

/// The forge, or why a group cannot be made without it.
pub fn require(state: &AppState) -> ApiResult<&Forge> {
    state.forge.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "no_forge",
            "This hub has no forge, which keeps the groups' libraries: set up Forgejo \
             and the [forge] section of the hub's configuration",
        )
    })
}

/// What a failure in Forgejo is, for the API.
pub fn api_error(error: ForgeError) -> ApiError {
    tracing::error!(%error, "the forge failed");
    ApiError::new(StatusCode::BAD_GATEWAY, "forge", error.to_string())
}

// ── Accounts ──────────────────────────────────────────────────────────

/// The login of a person's Forgejo account, which is made if they have
/// none: linked to the hub's authentication source as their subject, so
/// that signing in to Forgejo through the hub lands on it.
pub async fn ensure_user(state: &AppState, forge: &Forge, user_id: i64) -> anyhow::Result<String> {
    let (username, display_name, subject, known): (String, String, String, Option<String>) =
        sqlx::query_as(
            "SELECT username, display_name, subject, forge_login FROM users WHERE id = ?",
        )
        .bind(user_id)
        .fetch_one(&state.db)
        .await?;
    if let Some(login) = known {
        return Ok(login);
    }
    let login = match forge.linked_user(&subject).await? {
        // Someone who signed in to Forgejo before the hub made theirs.
        Some(user) => user.login,
        None => {
            let password = crate::secret::token("");
            let made = forge
                .create_user(&NewUser {
                    username: &username,
                    full_name: &display_name,
                    email: &email_of(&username),
                    password: &password,
                    must_change_password: false,
                    send_notify: false,
                    source_id: forge.source_id(),
                    login_name: &subject,
                    visibility: "private",
                })
                .await;
            match made {
                Ok(user) => user.login,
                Err(error) if error.is_conflict() => anyhow::bail!(
                    "Forgejo has an account called {username} that is not this person's: \
                     rename or remove it there ({error})"
                ),
                Err(error) => return Err(error.into()),
            }
        }
    };
    sqlx::query("UPDATE users SET forge_login = ? WHERE id = ?")
        .bind(&login)
        .bind(user_id)
        .execute(&state.db)
        .await?;
    Ok(login)
}

// ── Groups ────────────────────────────────────────────────────────────

/// Makes a group's organization, its teams and its library, from `files`.
/// If any of it fails, what was made is removed again.
pub async fn create_group(
    forge: &Forge,
    state: &AppState,
    slug: &str,
    name: &str,
    files: &[(String, String)],
) -> ApiResult<()> {
    if forge.organization_exists(slug).await.map_err(api_error)? {
        return Err(ApiError::conflict(format!(
            "Forgejo already has an account or organization called {slug}"
        )));
    }
    forge.ensure_workshop_users().await.map_err(api_error)?;
    forge
        .create_organization(slug, name)
        .await
        .map_err(|error| match error.is_conflict() {
            true => ApiError::conflict(format!(
                "Forgejo already has an account or organization called {slug}"
            )),
            false => api_error(error),
        })?;
    let made = async {
        ensure_teams(forge, slug).await?;
        create_repository(
            forge,
            state,
            slug,
            LIBRARY,
            &format!("The Lean library of {name}"),
            files,
        )
        .await
    }
    .await;
    if let Err(error) = made {
        if let Err(error) = forge.delete_organization(slug).await {
            tracing::error!(%error, group = slug, "could not remove a half-made organization");
        }
        return Err(api_error(error));
    }
    Ok(())
}

/// Makes a repository of a group's organization, its `main` with `files`,
/// protected from rewriting, and hooked to the hub. If any of it fails, the
/// repository is removed again.
pub async fn create_repository(
    forge: &Forge,
    state: &AppState,
    org: &str,
    name: &str,
    description: &str,
    files: &[(String, String)],
) -> ForgeResult<()> {
    forge.create_repository(org, name, description).await?;
    let made = async {
        forge
            .commit_files(org, name, MAIN, "Start the repository", files)
            .await?;
        forge.protect_branch(org, name, MAIN).await?;
        let hook = format!("{}{HOOK_PATH}", state.config.hub_url_for_forge());
        let secret = &state
            .config
            .forge
            .as_ref()
            .expect("a forge is configured")
            .secrets
            .webhook_secret;
        forge.add_push_hook(org, name, MAIN, &hook, secret).await
    }
    .await;
    if made.is_err()
        && let Err(error) = forge.delete_repository(org, name).await
    {
        tracing::error!(%error, org, name, "could not remove a half-made repository");
    }
    made
}

/// The ids of an organization's teams, by role, making those missing.
async fn ensure_teams(forge: &Forge, org: &str) -> ForgeResult<HashMap<Role, i64>> {
    let teams = forge.teams(org).await?;
    let mut ids = HashMap::new();
    for role in ROLES {
        let name = team_name(role);
        let id = match teams.iter().find(|t| t.name == name) {
            Some(team) => team.id,
            None => {
                let permission = match role {
                    Role::Guest => Permission::Read,
                    // Owners is made with the organization; were it missing,
                    // admins would still write.
                    _ => Permission::Write,
                };
                let description = match role {
                    Role::Admin => "The group's admins",
                    Role::Member => "The group's members: they write",
                    Role::Guest => "The group's guests: they read",
                };
                forge
                    .create_team(org, name, description, permission)
                    .await?
                    .id
            }
        };
        ids.insert(role, id);
    }
    forge.add_team_member(ids[&Role::Admin], "workshop").await?;
    forge.add_team_member(ids[&Role::Member], "worker").await?;
    Ok(ids)
}

/// Makes the teams of a group's organization what its roles are: each
/// person in the team of their role, and in no other. The hub's own
/// account in Forgejo is left as it is.
pub async fn sync_members(state: &AppState, group_id: i64) -> anyhow::Result<()> {
    let Some(forge) = state.forge.as_deref() else {
        return Ok(());
    };
    // One at a time, so that one does not undo what another just did.
    let _turn = state.forge_sync.lock().await;
    let slug: String = sqlx::query_scalar("SELECT slug FROM groups WHERE id = ?")
        .bind(group_id)
        .fetch_one(&state.db)
        .await?;
    let members: Vec<(i64, String)> =
        sqlx::query_as("SELECT user_id, role FROM memberships WHERE group_id = ?")
            .bind(group_id)
            .fetch_all(&state.db)
            .await?;
    let hub = forge.admin_login().await?;
    let mut wanted: HashMap<Role, HashSet<String>> = HashMap::new();
    wanted
        .entry(Role::Admin)
        .or_default()
        .insert("workshop".into());
    wanted
        .entry(Role::Member)
        .or_default()
        .insert("worker".into());
    let mut failures = Vec::new();
    for (user_id, role) in members {
        let role = Role::parse(&role).unwrap_or(Role::Guest);
        match ensure_user(state, forge, user_id).await {
            Ok(login) => {
                wanted.entry(role).or_default().insert(login);
            }
            Err(error) => failures.push(error.to_string()),
        }
    }
    let teams = ensure_teams(forge, &slug).await?;
    for role in ROLES {
        let team = teams[&role];
        let wanted = wanted.remove(&role).unwrap_or_default();
        let present: HashSet<String> = forge.team_members(team).await?.into_iter().collect();
        for login in wanted.difference(&present) {
            if *login != hub {
                forge.add_team_member(team, login).await?;
            }
        }
        for login in present.difference(&wanted) {
            if *login != hub {
                forge.remove_team_member(team, login).await?;
            }
        }
    }
    anyhow::ensure!(failures.is_empty(), "{}", failures.join("; "));
    Ok(())
}

/// [`sync_members`], whose failure is only told: the hub's roles are what
/// counts, and the next change, or the hub's next start, tries again.
pub async fn sync_members_or_log(state: &AppState, group_id: i64) {
    if let Err(error) = sync_members(state, group_id).await {
        tracing::error!(group_id, %error, "could not keep the group's teams in Forgejo");
    }
}

/// Checks that Forgejo answers, and keeps every group's teams as its roles
/// are, as after the hub was away.
pub async fn reconcile(state: &AppState) {
    let Some(forge) = state.forge.as_deref() else {
        tracing::warn!("The hub has no forge: groups cannot be made, nor libraries reached");
        return;
    };
    match forge.admin_login().await {
        Ok(login) => tracing::info!(login, "Acting in Forgejo"),
        Err(error) => {
            tracing::error!(%error, "Forgejo does not answer as its admin");
            return;
        }
    }
    if let Err(error) = forge.ensure_workshop_users().await {
        tracing::error!(%error, "could not prepare workshop accounts");
        return;
    }
    let groups: Vec<i64> = match sqlx::query_scalar("SELECT id FROM groups")
        .fetch_all(&state.db)
        .await
    {
        Ok(groups) => groups,
        Err(error) => {
            tracing::error!(%error, "could not list the groups");
            return;
        }
    };
    for group_id in groups {
        sync_members_or_log(state, group_id).await;
        let repos: Result<Vec<(String, String)>, _> = sqlx::query_as(
            "SELECT g.slug, 'library' FROM groups g WHERE g.id = ?1 UNION ALL SELECT g.slug, p.repository FROM papers p JOIN groups g ON g.id = p.group_id WHERE g.id = ?1")
            .bind(group_id).fetch_all(&state.db).await;
        if let Ok(repos) = repos {
            for (org, repo) in repos {
                if let Err(error) = forge.protect_branch(&org, &repo, MAIN).await {
                    tracing::error!(%error, org, repo, "could not reconcile main protection");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_names() {
        for name in ["admin", "api", "explore", "user", "me.keys", "logo.png"] {
            assert!(is_reserved(name), "{name}");
        }
        for name in ["ada", "geometry", "number-theory", "administrator"] {
            assert!(!is_reserved(name), "{name}");
        }
    }

    #[test]
    fn roles_have_teams() {
        assert_eq!(team_name(Role::Admin), "Owners");
        assert_eq!(team_name(Role::Member), "Members");
        assert_eq!(team_name(Role::Guest), "Guests");
    }
}
