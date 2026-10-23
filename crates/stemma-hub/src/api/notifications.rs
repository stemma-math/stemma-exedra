//! A person's notifications, their own live updates, and notes they or
//! their agents leave for others.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use futures_util::{Stream, StreamExt};
use stemma_core::api::{
    NotificationKind, NotificationPage, NotificationQuery, NotificationSettings, SendNote,
};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use super::{Json, Membership, required_text};
use crate::auth::Viewer;
use crate::error::{ApiError, ApiResult};
use crate::notifications::{self, Notice, Which};
use crate::state::AppState;
use crate::{api::forum, db};

pub async fn list(
    State(state): State<AppState>,
    viewer: Viewer,
    Query(query): Query<NotificationQuery>,
) -> ApiResult<Json<NotificationPage>> {
    viewer.require_full()?;
    notifications::prune(&state, viewer.id).await?;
    let (items, more) = notifications::page(&state, viewer.id, query.before).await?;
    Ok(Json(NotificationPage {
        items,
        unread: notifications::unread(&state, viewer.id).await?,
        more,
    }))
}

pub async fn read_one(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    notifications::read(&state, viewer.id, Which::One(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn read_all(State(state): State<AppState>, viewer: Viewer) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    notifications::read(&state, viewer.id, Which::All).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Marks read what the viewer was told about a thread they now read.
pub async fn read_thread(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, number)): Path<(String, u32)>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    let thread = forum::find_thread(&state, group.group_id, number).await?;
    notifications::read(&state, viewer.id, Which::Thread(thread.id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Leaves a note for some of the group's members: a person, or an agent of
/// theirs in a session. An answer's agent answers instead. Each of them
/// decides what they accept; the note goes to all of them, or to none.
pub async fn send_note(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    Json(body): Json<SendNote>,
) -> ApiResult<StatusCode> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let agent = match viewer.agent() {
        Some(scope) if scope.session => Some(scope.agent.clone()),
        Some(_) => {
            return Err(ApiError::forbidden(
                "An answer's agent cannot leave notes: it answers in its thread",
            ));
        }
        None => match body.agent.as_deref().map(str::trim) {
            Some(_) if !viewer.with_token => {
                return Err(ApiError::invalid("Only an API token writes for an agent"));
            }
            Some(agent) => Some(required_text(agent, "The agent's name", 60)?),
            None => None,
        },
    };
    if !group.role.can_write() {
        return Err(ApiError::forbidden("Guests cannot leave notes"));
    }
    let text = required_text(&body.body, "A note", 4_000)?;
    if body.to.is_empty() {
        return Err(ApiError::invalid("Say who the note is for"));
    }
    let mut users = Vec::new();
    for name in &body.to {
        let name = name.trim().trim_start_matches('@');
        let user: Option<i64> = sqlx::query_scalar(
            "SELECT u.id FROM users u JOIN memberships m ON m.user_id = u.id
             WHERE m.group_id = ? AND u.username = ? AND u.deleted_at IS NULL",
        )
        .bind(group.group_id)
        .bind(name)
        .fetch_optional(&state.db)
        .await?;
        let user = user.ok_or_else(|| {
            ApiError::invalid(format!("@{name} is not a member of {}", group.slug))
        })?;
        let accepts = notifications::settings(&state, user).await?;
        if agent.is_some() && !accepts.notes_from_agents {
            return Err(ApiError::forbidden(format!(
                "@{name} accepts notes from people only, not from their agents"
            )));
        }
        if let Some(limit) = accepts.notes_per_hour {
            let received: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM notifications
                 WHERE kind = 'note' AND user_id = ? AND created_at > ?",
            )
            .bind(user)
            .bind(db::now() - 60 * 60 * 1000)
            .fetch_one(&state.db)
            .await?;
            if received >= i64::from(limit) {
                return Err(ApiError::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "too_many_notes",
                    format!("@{name} accepts at most {limit} notes an hour: try again later"),
                ));
            }
        }
        users.push(user);
    }
    let stored =
        crate::text::encode(&mut *state.db.acquire().await?, group.group_id, &text).await?;
    let notice = Notice::new(NotificationKind::Note)
        .group(group.group_id)
        .by(viewer.id, agent.as_deref())
        .text(&stored);
    notifications::notify(&state, &users, &notice).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn settings(
    State(state): State<AppState>,
    viewer: Viewer,
) -> ApiResult<Json<NotificationSettings>> {
    viewer.require_full()?;
    Ok(Json(notifications::settings(&state, viewer.id).await?))
}

pub async fn update_settings(
    State(state): State<AppState>,
    viewer: Viewer,
    Json(body): Json<NotificationSettings>,
) -> ApiResult<Json<NotificationSettings>> {
    viewer.require_full()?;
    if body.notes_per_hour == Some(0) {
        return Err(ApiError::invalid(
            "Accept at least one note an hour, or no limit",
        ));
    }
    if body.keep_read_days == Some(0) {
        return Err(ApiError::invalid(
            "Keep what you read a day at least, or always",
        ));
    }
    sqlx::query(
        "UPDATE users SET notes_per_hour = ?, notes_from_agents = ?, keep_read_days = ? WHERE id = ?",
    )
    .bind(body.notes_per_hour)
    .bind(body.notes_from_agents)
    .bind(body.keep_read_days)
    .bind(viewer.id)
    .execute(&state.db)
    .await?;
    notifications::prune(&state, viewer.id).await?;
    Ok(Json(notifications::settings(&state, viewer.id).await?))
}

/// Streams the viewer's own events, each a JSON
/// [`stemma_core::api::PersonalEvent`]; `resync` when they fell behind.
pub async fn stream(
    State(state): State<AppState>,
    viewer: Viewer,
) -> ApiResult<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>> {
    viewer.require_full()?;
    let user = viewer.id;
    let events = BroadcastStream::new(state.personal.subscribe()).filter_map(move |item| {
        let event = match item {
            Ok(item) if item.user_id == user => SseEvent::default().json_data(&*item.event).ok(),
            Ok(_) => None,
            Err(BroadcastStreamRecvError::Lagged(_)) => {
                Some(SseEvent::default().event("resync").data("{}"))
            }
        };
        std::future::ready(event.map(Ok))
    });
    // An event with no data is never dispatched: each carries `{}`.
    let opening = futures_util::stream::once(std::future::ready(Ok(SseEvent::default()
        .event("ready")
        .data("{}"))));
    Ok(Sse::new(
        opening
            .chain(events)
            .take_until(state.shutdown.clone().cancelled_owned()),
    )
    .keep_alive(KeepAlive::new().interval(Duration::from_secs(20))))
}
