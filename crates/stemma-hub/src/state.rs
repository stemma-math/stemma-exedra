//! What every request handler shares.

use std::sync::{Arc, Mutex};

use sqlx::SqlitePool;
use stemma_core::api::Event;
use tokio::sync::broadcast;

use crate::config::Config;

pub type AppState = Arc<Hub>;

/// The running hub.
pub struct Hub {
    pub db: SqlitePool,
    pub config: Config,
    /// Every group's live updates; each listener keeps its group's.
    pub events: broadcast::Sender<GroupEvent>,
    /// The secret that creates the first account, while there is none.
    pub setup_token: Mutex<Option<String>>,
}

#[derive(Clone, Debug)]
pub struct GroupEvent {
    pub group_id: i64,
    pub event: Arc<Event>,
}

impl Hub {
    pub fn new(db: SqlitePool, config: Config) -> Hub {
        let (events, _) = broadcast::channel(256);
        Hub {
            db,
            config,
            events,
            setup_token: Mutex::new(None),
        }
    }

    /// Tells a group's listeners what happened.
    pub fn publish(&self, group_id: i64, event: Event) {
        // Nobody listening is fine.
        let _ = self.events.send(GroupEvent {
            group_id,
            event: Arc::new(event),
        });
    }
}
