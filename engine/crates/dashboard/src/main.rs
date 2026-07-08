//! Nexus Dashboard — HTTP API server for workflow management and real-time monitoring.
//!
//! Serves REST API on port 48080 and WebSocket for live workflow status push.
//! Designed to be started via `nexus-cli dashboard` or standalone.

#![allow(missing_docs)]

// Silences `unused-crate-dependencies` — used in child modules.
#[expect(unused_imports)]
use futures as _futures;
#[cfg(test)]
#[expect(unused_imports)]
use futures_util as _futures_util;
#[cfg(test)]
#[expect(unused_imports)]
use tokio_tungstenite as _tokio_tungstenite;

mod api;
mod db;
mod engine_bridge;
mod models;
mod state;
mod ws;

use std::net::SocketAddr;

use axum::routing::get;
use axum::Router;
use tower_http::cors::CorsLayer;

use state::AppState;
use ws::WsRoom;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let store = db::Store::new(None).expect("SQLite store initialization failed");
    let room = WsRoom::new();
    let state = AppState { store, room };

    let app = Router::new()
        .route("/ws/runs/{run_id}", get(ws::ws_handler))
        .nest("/api", api::routes())
        .with_state(state)
        .layer(CorsLayer::permissive());

    let addr = SocketAddr::from(([127, 0, 0, 1], 48080));
    tracing::info!("[Dashboard.Server] listening on 127.0.0.1:48080");
    tracing::info!("[Dashboard.Server] REST API: /api/workflows, /api/runs");
    tracing::info!("[Dashboard.Server] WebSocket: /ws/runs/:run_id");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
