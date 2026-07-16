use crate::config::Config;
use crate::db::Db;
use crate::models::ServerEvent;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub config: Arc<Config>,
    pub events: broadcast::Sender<ServerEvent>,
    pub update_lock: Arc<Mutex<()>>,
}

impl AppState {
    pub fn new(db: Db, config: Config) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            db,
            config: Arc::new(config),
            events,
            update_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn publish(&self, event: ServerEvent) {
        let _ = self.events.send(event);
    }
}
