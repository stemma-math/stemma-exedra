//! What every request handler shares.

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use sqlx::SqlitePool;
use stemma_core::api::Event;
use tokio::sync::broadcast;

use crate::config::Config;
use crate::forge::Forge;
use crate::jobs::Workers;
use crate::oidc::SigningKey;

pub type AppState = Arc<Hub>;

/// The running hub.
pub struct Hub {
    pub db: SqlitePool,
    pub config: Config,
    /// Every group's live updates; each listener keeps its group's.
    pub events: broadcast::Sender<GroupEvent>,
    /// The secret that creates the first account, while there is none.
    pub setup_token: Mutex<Option<String>>,
    /// The workers connected now.
    pub workers: tokio::sync::Mutex<Workers>,
    /// The number of the last worker connection.
    pub connections: AtomicU64,
    /// The forge that keeps the groups' repositories, if there is one.
    pub forge: Option<Arc<Forge>>,
    /// Taken while the hub changes the forge's teams.
    pub forge_sync: tokio::sync::Mutex<()>,
    /// The key the hub signs its OpenID Connect tokens with, when it has a
    /// forge to sign people in to.
    pub signing_key: Option<SigningKey>,
}

#[derive(Clone, Debug)]
pub struct GroupEvent {
    pub group_id: i64,
    pub event: Arc<Event>,
}

impl Hub {
    /// The hub over `db`; with a forge, its signing key is read from the
    /// data directory, or made there.
    pub fn new(db: SqlitePool, config: Config) -> anyhow::Result<Hub> {
        let (events, _) = broadcast::channel(256);
        let (forge, signing_key) = match &config.forge {
            Some(forge) => (
                Some(Arc::new(Forge::new(forge)?)),
                Some(SigningKey::load_or_create(&config.signing_key_path())?),
            ),
            None => (None, None),
        };
        Ok(Hub {
            db,
            config,
            events,
            setup_token: Mutex::new(None),
            workers: tokio::sync::Mutex::new(Workers::default()),
            connections: AtomicU64::new(0),
            forge,
            forge_sync: tokio::sync::Mutex::new(()),
            signing_key,
        })
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
