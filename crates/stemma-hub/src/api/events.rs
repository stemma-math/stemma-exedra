//! Live updates: a group's events as server-sent events.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use futures_util::{Stream, StreamExt};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use super::Membership;
use crate::auth::Viewer;
use crate::error::ApiResult;
use crate::state::AppState;

/// Streams the group's events, each a JSON [`stemma_core::api::Event`].
/// A listener that falls behind gets a `resync` event, and should fetch
/// what it shows again.
pub async fn stream(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
) -> ApiResult<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let group_id = group.group_id;
    let events = BroadcastStream::new(state.events.subscribe()).filter_map(move |item| {
        let event = match item {
            Ok(item) if item.group_id == group_id => {
                SseEvent::default().json_data(&*item.event).ok()
            }
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
