//! Shared application state for axum routing.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::FromRef;

use crate::db::Store;
use crate::ws::WsRoom;

/// Shared state provided to all handlers.
#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub room: Arc<WsRoom>,
    /// Cancel flags for running workflows, keyed by `run_id`.
    pub cancel_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

impl AppState {
    /// Register a cancel flag for a `run_id`.
    #[must_use]
    pub fn register_cancel(&self, run_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.cancel_flags
            .lock()
            .unwrap()
            .insert(run_id.to_string(), flag.clone());
        flag
    }

    /// Cancel a running workflow and remove its flag.
    #[must_use]
    pub fn cancel_run(&self, run_id: &str) -> bool {
        if let Some(flag) = self.cancel_flags.lock().unwrap().remove(run_id) {
            flag.store(true, Ordering::Relaxed);
            true
        } else {
            false
        }
    }
}

impl FromRef<AppState> for Store {
    fn from_ref(state: &AppState) -> Self {
        state.store.clone()
    }
}

impl FromRef<AppState> for Arc<WsRoom> {
    fn from_ref(state: &AppState) -> Self {
        state.room.clone()
    }
}
