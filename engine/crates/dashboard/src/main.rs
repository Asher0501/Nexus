//! Nexus Dashboard — HTTP API server for workflow management and real-time monitoring.
//!
//! Serves REST API on port 48080 and WebSocket for live workflow status push.
//! Designed to be started via `nexus-cli dashboard` or standalone.

#![allow(missing_docs)]
// The dashboard crate has both a bin and a lib target.  The
// `unused_crate_dependencies` lint fires a false positive for the
// crate's own name when the binary target uses `mod` items from the
// library.  See rust-lang/rust#57274 for background.
#![allow(unused_crate_dependencies)]

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
mod static_files;
mod ws;

use std::net::SocketAddr;

use axum::routing::get;
use axum::Router;
use tower_http::cors::CorsLayer;

use state::AppState;
use ws::WsRoom;

#[tokio::main]
async fn main() {
    let _ = std::fs::create_dir_all("log");
    let log_file = std::fs::OpenOptions::new()
        .create(true).append(true)
        .open("log/dashboard.log")
        .expect("open log/dashboard.log");
    let log_file = std::sync::Mutex::new(log_file);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("nexus::nodeshell=debug,nexus::node=debug,info"));
    tracing_subscriber::fmt()
        .with_writer(log_file)
        .with_env_filter(filter)
        .with_target(true)
        .init();

    let store = db::Store::new(None).expect("SQLite store initialization failed");
    let room = WsRoom::new();
    let state = AppState {
        store,
        room,
        cancel_flags: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
    };

    let app = Router::new()
        .route("/ws/runs/{run_id}", get(ws::ws_handler))
        .nest("/api", api::routes())
        .with_state(state)
        .layer(CorsLayer::permissive());

    // Optional static file serving: if NEXUS_STATIC_DIR (default ./static)
    // exists, serve files from it as a fallback for non-API routes.
    let static_dir = std::env::var("NEXUS_STATIC_DIR").unwrap_or_else(|_| "./static".into());
    if std::path::Path::new(&static_dir).exists() {
        tracing::info!("[Dashboard.Server] serving static files from: {static_dir}");
    }
    let app = app.fallback(static_files::handler);

    let host = std::env::var("NEXUS_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port: u16 = std::env::var("NEXUS_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(48080);

    let addr = SocketAddr::from((host.parse::<std::net::Ipv4Addr>().unwrap_or(std::net::Ipv4Addr::new(127, 0, 0, 1)), port));
    tracing::info!("[Dashboard.Server] listening on {addr}");
    tracing::info!("[Dashboard.Server] REST API: /api/workflows, /api/runs");
    tracing::info!("[Dashboard.Server] WebSocket: /ws/runs/:run_id");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
