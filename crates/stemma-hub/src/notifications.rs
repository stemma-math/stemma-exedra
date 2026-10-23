//! What people are told: someone mentioned them, an agent answered them,
//! a note, or something happened to one of their groups.
//!
//! While unread, a mention or an answer gathers the next of its kind in the
//! same thread, as one notification that moves up. Read ones are kept as
//! long as their person chooses, and a person's listeners hear how many are
//! unread.

use std::collections::BTreeSet;

use stemma_core::api::{
    Notification, NotificationKind, NotificationSettings, NotifiedGroup, NotifiedThread,
    PersonalEvent, ThreadKind, User,
};

use crate::db;
use crate::error::ApiResult;
use crate::state::AppState;

/// How many notifications a page holds.
pub const PAGE: u32 = 30;
/// How long an excerpt of a message is, in characters.
const EXCERPT: usize = 160;

/// Something to tell, before it is told to anyone.
#[derive(Clone, Debug)]
pub struct Notice<'a> {
    pub kind: NotificationKind,
    pub group_id: Option<i64>,
    pub thread_id: Option<i64>,
    pub message: Option<u32>,
    pub actor_id: Option<i64>,
    pub agent: Option<&'a str>,
    pub text: Option<&'a str>,
}

impl<'a> Notice<'a> {
    pub fn new(kind: NotificationKind) -> Notice<'a> {
        Notice {
            kind,
            group_id: None,
            thread_id: None,
            message: None,
            actor_id: None,
            agent: None,
            text: None,
        }
    }

    /// About a message in a thread of a group.
    pub fn at(mut self, group_id: i64, thread_id: i64, message: Option<u32>) -> Notice<'a> {
        self.group_id = Some(group_id);
        self.thread_id = Some(thread_id);
        self.message = message;
        self
    }

    pub fn group(mut self, group_id: i64) -> Notice<'a> {
        self.group_id = Some(group_id);
        self
    }

    /// Done by `actor_id`, or by their agent.
    pub fn by(mut self, actor_id: i64, agent: Option<&'a str>) -> Notice<'a> {
        self.actor_id = Some(actor_id);
        self.agent = agent;
        self
    }

    pub fn text(mut self, text: &'a str) -> Notice<'a> {
        self.text = Some(text);
        self
    }
}

/// Whether unread notifications of `kind` in one thread become one.
fn gathers(kind: NotificationKind) -> bool {
    matches!(kind, NotificationKind::Mention | NotificationKind::Answer)
}

/// Tells each of `users` of `notice`, once.
pub async fn notify(state: &AppState, users: &[i64], notice: &Notice<'_>) -> ApiResult<()> {
    let users: BTreeSet<i64> = users.iter().copied().collect();
    for user in users {
        let mut tx = state.db.begin().await?;
        // The one it gathers goes, and comes back on top with its count.
        let mut count = 1;
        if gathers(notice.kind)
            && let Some(thread_id) = notice.thread_id
        {
            let gathered: Option<i64> = sqlx::query_scalar(
                "DELETE FROM notifications
                 WHERE user_id = ? AND kind = ? AND thread_id = ? AND read_at IS NULL
                 RETURNING count",
            )
            .bind(user)
            .bind(notice.kind.as_str())
            .bind(thread_id)
            .fetch_optional(&mut *tx)
            .await?;
            count += gathered.unwrap_or(0);
        }
        let now = db::now();
        sqlx::query(
            "INSERT INTO notifications
             (user_id, kind, group_id, thread_id, message, count, actor_id, agent, text, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(user)
        .bind(notice.kind.as_str())
        .bind(notice.group_id)
        .bind(notice.thread_id)
        .bind(notice.message)
        .bind(count)
        .bind(notice.actor_id)
        .bind(notice.agent)
        .bind(notice.text)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        changed(state, user).await?;
    }
    Ok(())
}

/// Tells `users` that are members of `group_id` and not `except` of
/// `notice`: those a message in that group may reach.
pub async fn notify_members(
    state: &AppState,
    group_id: i64,
    users: impl IntoIterator<Item = i64>,
    except: i64,
    notice: &Notice<'_>,
) -> ApiResult<()> {
    let mut members = Vec::new();
    for user in users.into_iter().filter(|&u| u != except) {
        let member: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM memberships m JOIN users u ON u.id = m.user_id
             WHERE m.group_id = ? AND m.user_id = ? AND u.deleted_at IS NULL)",
        )
        .bind(group_id)
        .bind(user)
        .fetch_one(&state.db)
        .await?;
        if member {
            members.push(user);
        }
    }
    notify(state, &members, notice).await
}

/// How many of `user`'s notifications are unread.
pub async fn unread(state: &AppState, user: i64) -> ApiResult<u32> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM notifications WHERE user_id = ? AND read_at IS NULL",
    )
    .bind(user)
    .fetch_one(&state.db)
    .await?;
    Ok(count as u32)
}

/// Tells `user`'s listeners how many are unread now.
pub async fn changed(state: &AppState, user: i64) -> ApiResult<()> {
    let unread = unread(state, user).await?;
    state.tell(user, PersonalEvent::Notifications { unread });
    Ok(())
}

/// Marks `user`'s notifications read: one, those of a thread, or all.
pub async fn read(state: &AppState, user: i64, which: Which) -> ApiResult<()> {
    let now = db::now();
    let query = match which {
        Which::One(id) => sqlx::query(
            "UPDATE notifications SET read_at = ? WHERE user_id = ? AND id = ? AND read_at IS NULL",
        )
        .bind(now)
        .bind(user)
        .bind(id),
        Which::Thread(thread_id) => sqlx::query(
            "UPDATE notifications SET read_at = ?
             WHERE user_id = ? AND thread_id = ? AND read_at IS NULL",
        )
        .bind(now)
        .bind(user)
        .bind(thread_id),
        Which::All => sqlx::query(
            "UPDATE notifications SET read_at = ? WHERE user_id = ? AND read_at IS NULL",
        )
        .bind(now)
        .bind(user),
    };
    if query.execute(&state.db).await?.rows_affected() > 0 {
        changed(state, user).await?;
    }
    Ok(())
}

/// Which notifications to mark read.
#[derive(Clone, Copy, Debug)]
pub enum Which {
    One(i64),
    Thread(i64),
    All,
}

/// Forgets what `user` read longer ago than they keep it.
pub async fn prune(state: &AppState, user: i64) -> ApiResult<()> {
    let Some(days) = settings(state, user).await?.keep_read_days else {
        return Ok(());
    };
    let before = db::now() - i64::from(days) * 24 * 60 * 60 * 1000;
    sqlx::query("DELETE FROM notifications WHERE user_id = ? AND read_at < ?")
        .bind(user)
        .bind(before)
        .execute(&state.db)
        .await?;
    Ok(())
}

/// What `user` accepts as notes, and how long they keep what they read.
pub async fn settings(state: &AppState, user: i64) -> ApiResult<NotificationSettings> {
    let (notes_per_hour, notes_from_agents, keep_read_days): (Option<i64>, bool, Option<i64>) =
        sqlx::query_as(
            "SELECT notes_per_hour, notes_from_agents, keep_read_days FROM users WHERE id = ?",
        )
        .bind(user)
        .fetch_one(&state.db)
        .await?;
    Ok(NotificationSettings {
        notes_per_hour: notes_per_hour.map(|n| n as u32),
        notes_from_agents,
        keep_read_days: keep_read_days.map(|n| n as u32),
    })
}

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    kind: String,
    count: i64,
    agent: Option<String>,
    text: Option<String>,
    read_at: Option<i64>,
    updated_at: i64,
    group_slug: Option<String>,
    group_name: Option<String>,
    /// Whether the person is still in the group.
    member: bool,
    thread_number: Option<i64>,
    thread_kind: Option<String>,
    thread_title: Option<String>,
    message: Option<i64>,
    message_body: Option<String>,
    message_deleted: Option<i64>,
    actor_id: Option<i64>,
    actor_username: Option<String>,
    actor_name: Option<String>,
}

/// A page of `user`'s notifications, newest first, before the one
/// `before`; and whether there are more.
pub async fn page(
    state: &AppState,
    user: i64,
    before: Option<i64>,
) -> ApiResult<(Vec<Notification>, bool)> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT n.id, n.kind, n.count, n.agent, n.text, n.read_at, n.updated_at,
                g.slug AS group_slug, g.name AS group_name,
                EXISTS(SELECT 1 FROM memberships m WHERE m.group_id = n.group_id AND m.user_id = n.user_id) AS member,
                t.number AS thread_number, t.kind AS thread_kind, t.title AS thread_title,
                n.message, msg.body AS message_body, msg.deleted_at AS message_deleted,
                a.id AS actor_id, a.username AS actor_username, a.display_name AS actor_name
         FROM notifications n
         LEFT JOIN groups g ON g.id = n.group_id
         LEFT JOIN threads t ON t.id = n.thread_id
         LEFT JOIN messages msg ON msg.thread_id = n.thread_id AND msg.number = n.message
         LEFT JOIN users a ON a.id = n.actor_id
         WHERE n.user_id = ?1 AND (?2 IS NULL OR n.id < ?2)
         ORDER BY n.id DESC
         LIMIT ?3",
    )
    .bind(user)
    .bind(before)
    .bind(PAGE + 1)
    .fetch_all(&state.db)
    .await?;
    let more = rows.len() > PAGE as usize;
    let mut items = Vec::new();
    for row in rows.into_iter().take(PAGE as usize) {
        items.push(notification(state, row).await?);
    }
    Ok((items, more))
}

async fn notification(state: &AppState, row: Row) -> ApiResult<Notification> {
    let kind = NotificationKind::parse(&row.kind).unwrap_or(NotificationKind::Note);
    let about_thread = matches!(
        kind,
        NotificationKind::Mention
            | NotificationKind::Answer
            | NotificationKind::Held
            | NotificationKind::Failed
    );
    // A thread of a group they have left is no longer theirs to read.
    let thread = match (row.thread_number, row.member) {
        (Some(number), true) => Some(NotifiedThread {
            number: number as u32,
            kind: match row.thread_kind.as_deref() {
                Some("chat") => ThreadKind::Chat,
                _ => ThreadKind::Discussion,
            },
            title: row.thread_title.unwrap_or_default(),
            message: row.message.map(|n| n as u32),
            excerpt: row
                .message_body
                .filter(|_| row.message_deleted.is_none())
                .map(|body| excerpt(&body))
                .filter(|e| !e.is_empty()),
        }),
        _ => None,
    };
    let gone = about_thread
        && (thread.is_none() || (row.message.is_some() && row.message_deleted.is_some()));
    let text = match (kind, row.text) {
        (NotificationKind::Note, Some(stored)) => {
            Some(crate::text::render(state, &stored).await?.0)
        }
        (_, text) => text,
    };
    Ok(Notification {
        id: row.id,
        kind,
        count: row.count as u32,
        actor: match (row.actor_id, row.actor_username, row.actor_name) {
            (Some(id), Some(username), Some(display_name)) => Some(User {
                id,
                username,
                display_name,
            }),
            _ => None,
        },
        agent: row.agent,
        group: match (row.group_slug, row.group_name) {
            (Some(slug), Some(name)) => Some(NotifiedGroup { slug, name }),
            _ => None,
        },
        thread,
        text,
        gone,
        read: row.read_at.is_some(),
        updated_at: db::from_millis(row.updated_at),
    })
}

/// The start of a message, on one line.
fn excerpt(body: &str) -> String {
    let line = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= EXCERPT {
        return line;
    }
    let mut cut: String = line.chars().take(EXCERPT).collect();
    if let Some(space) = cut.rfind(' ') {
        cut.truncate(space);
    }
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::excerpt;

    #[test]
    fn an_excerpt_is_one_line_cut_at_a_word() {
        assert_eq!(excerpt("Look at\n\nthis  lemma"), "Look at this lemma");
        let long = "word ".repeat(60);
        let cut = excerpt(&long);
        assert!(cut.ends_with("word…") && cut.chars().count() <= 161);
    }
}
