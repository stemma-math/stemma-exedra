//! A person's notifications, their own live updates, and notes they or
//! their agents leave for others.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use futures_util::{Stream, StreamExt};
use stemma_core::api::{NotificationKind, NotificationPage, NotificationQuery, SendNote};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use super::{Json, Membership, required_text};
use crate::auth::Viewer;
use crate::error::{ApiError, ApiResult};
use crate::notifications::{self, Notice, Which};
use crate::state::AppState;
use crate::{api::forum, db};

/// The most people one note is for.
const MAX_RECIPIENTS: usize = 20;
/// The most notes one person, with their agents, leaves in an hour.
const NOTES_PER_HOUR: i64 = 60;

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
/// theirs in a session. An answer's agent answers instead.
pub async fn send_note(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    Json(body): Json<SendNote>,
) -> ApiResult<StatusCode> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let (agent, token_id) = match viewer.agent() {
        Some(scope) if scope.session => (Some(scope.agent.clone()), Some(scope.token_id)),
        Some(_) => {
            return Err(ApiError::forbidden(
                "An answer's agent cannot leave notes: it answers in its thread",
            ));
        }
        None => match body.agent.as_deref().map(str::trim) {
            Some(_) if !viewer.with_token => {
                return Err(ApiError::invalid("Only an API token writes for an agent"));
            }
            Some(agent) => (Some(required_text(agent, "The agent's name", 60)?), None),
            None => (None, None),
        },
    };
    if !group.role.can_write() {
        return Err(ApiError::forbidden("Guests cannot leave notes"));
    }
    let text = required_text(&body.body, "A note", 4_000)?;
    if body.to.is_empty() {
        return Err(ApiError::invalid("Say who the note is for"));
    }
    if body.to.len() > MAX_RECIPIENTS {
        return Err(ApiError::invalid(format!(
            "A note is for at most {MAX_RECIPIENTS} people"
        )));
    }
    let sent: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM notifications WHERE kind = 'note' AND actor_id = ? AND created_at > ?",
    )
    .bind(viewer.id)
    .bind(db::now() - 60 * 60 * 1000)
    .fetch_one(&state.db)
    .await?;
    if sent + body.to.len() as i64 > NOTES_PER_HOUR {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_notes",
            format!("At most {NOTES_PER_HOUR} notes an hour: try again later"),
        ));
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
        users.push(user.ok_or_else(|| {
            ApiError::invalid(format!("@{name} is not a member of {}", group.slug))
        })?);
    }
    let stored =
        crate::text::encode(&mut *state.db.acquire().await?, group.group_id, &text).await?;
    let mut notice = Notice::new(NotificationKind::Note)
        .group(group.group_id)
        .by(viewer.id, agent.as_deref())
        .text(&stored);
    notice.token_id = token_id;
    notifications::notify(&state, &users, &notice).await?;
    Ok(StatusCode::NO_CONTENT)
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
