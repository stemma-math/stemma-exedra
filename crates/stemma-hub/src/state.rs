//! What every request handler shares.

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use sqlx::SqlitePool;
use stemma_core::api::{Event, PersonalEvent};
use tokio::sync::broadcast;

use crate::config::Config;
use crate::forge::Forge;
use crate::jobs::Workers;
use crate::oidc::SigningKey;

pub type AppState = Arc<Hub>;

/// The running hub.
pub struct Hub {
    pub db: SqlitePool,
    /// Ends long-lived responses before the HTTP server drains.
    pub shutdown: tokio_util::sync::CancellationToken,
    /// Upgraded connections outlive their HTTP requests.
    pub sockets: tokio_util::task::TaskTracker,
    pub config: Config,
    pub model_key: crate::model::Key,
    pub model_client: crate::model::Client,
    pub model_locks:
        tokio::sync::Mutex<std::collections::HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
    pub review_changes: tokio::sync::Mutex<()>,
    /// Every group's live updates; each listener keeps its group's.
    pub events: broadcast::Sender<GroupEvent>,
    /// Every person's own live updates; each listener keeps its person's.
    pub personal: broadcast::Sender<PersonEvent>,
    /// The secret that creates the first account, while there is none.
    pub setup_token: Mutex<Option<String>>,
    /// The workers connected now.
    pub workers: tokio::sync::Mutex<Workers>,
    pub build_machines: tokio::sync::Mutex<crate::builds::Machines>,
    pub process_changes: tokio::sync::Mutex<()>,
    pub build_changes: tokio::sync::Mutex<()>,
    /// The number of the last worker connection.
    pub connections: AtomicU64,
    /// The forge that keeps the groups' repositories, if there is one.
    pub forge: Option<Arc<Forge>>,
    /// Serializes requests and group creation, including their work in the forge.
    pub group_changes: tokio::sync::Mutex<()>,
    /// Taken while the hub changes the forge's teams.
    pub forge_sync: tokio::sync::Mutex<()>,
    /// The key the hub signs its OpenID Connect tokens with, when it has a
    /// forge to sign people in to.
    pub signing_key: Option<SigningKey>,
}

#[derive(Clone, Debug)]
pub struct PersonEvent {
    pub user_id: i64,
    pub event: Arc<PersonalEvent>,
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
        let (personal, _) = broadcast::channel(256);
        let (forge, signing_key) = match &config.forge {
            Some(forge) => (
                Some(Arc::new(Forge::new(forge)?)),
                Some(SigningKey::load_or_create(&config.signing_key_path())?),
            ),
            None => (None, None),
        };
        Ok(Hub {
            db,
            shutdown: tokio_util::sync::CancellationToken::new(),
            sockets: tokio_util::task::TaskTracker::new(),
            model_key: crate::model::Key::load_or_create(&config.data_dir.join("workshop.key"))?,
            model_client: crate::model::Client::new()?,
            model_locks: Default::default(),
            review_changes: Default::default(),
            config,
            events,
            personal,
            setup_token: Mutex::new(None),
            workers: tokio::sync::Mutex::new(Workers::default()),
            build_machines: Default::default(),
            process_changes: Default::default(),
            build_changes: Default::default(),
            connections: AtomicU64::new(0),
            forge,
            group_changes: tokio::sync::Mutex::new(()),
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

    /// Tells a person's own listeners what happened.
    pub fn tell(&self, user_id: i64, event: PersonalEvent) {
        let _ = self.personal.send(PersonEvent {
            user_id,
            event: Arc::new(event),
        });
    }
}
