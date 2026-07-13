//! Nexus Dashboard — library re-exports for integration testing.
//!
//! Binary crates cannot be imported from `tests/*.rs` integration tests.
//! This `lib.rs` re-exports the crate's public surface so integration
//! tests can construct a server without depending on `main.rs` internals.

#![allow(missing_docs)]

// Silences `unused-crate-dependencies` — used in the binary target.
#[expect(unused_imports)]
use futures as _futures;
#[expect(unused_imports)]
use tower_http as _tower_http;
#[expect(unused_imports)]
use tracing_subscriber as _tracing_subscriber;
#[expect(unused_imports)]
use rusqlite as _rusqlite;
#[expect(unused_imports)]
use serde as _serde;
#[expect(unused_imports)]
use serde_json as _serde_json;
#[expect(unused_imports)]
use uuid as _uuid;
#[cfg(test)]
#[expect(unused_imports)]
use futures_util as _futures_util;
#[cfg(test)]
#[expect(unused_imports)]
use tokio_tungstenite as _tokio_tungstenite;
#[cfg(test)]
#[expect(unused_imports)]
use reqwest as _reqwest;

pub mod api;
pub mod db;
pub mod engine_bridge;
pub mod models;
pub mod state;
pub mod ws;

// Re-export key types used by test helpers.
pub use db::Store;
pub use state::AppState;
pub use ws::WsRoom;
