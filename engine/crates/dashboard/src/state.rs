//! Shared application state for axum routing.

use std::sync::Arc;

use axum::extract::FromRef;

use crate::db::Store;
use crate::ws::WsRoom;

/// Shared state provided to all handlers.
///
/// Handlers extract individual fields via `State(store): State<Store>` or
/// `State(room): State<Arc<WsRoom>>` thanks to `FromRef` impls below.
#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub room: Arc<WsRoom>,
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
