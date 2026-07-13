//! WebSocket integration tests for nexus-dashboard.
//!
//! Each test spawns a fresh server on a random port, establishes a real WS
//! connection to `/ws/runs/{run_id}`, broadcasts messages through the WsRoom,
//! and verifies the client receives correctly-formatted JSON messages.
//!
//! Tests covered:
//!   WS-1  Connect and receive node_status broadcast
//!   WS-2  Connect and receive node_chunk broadcast
//!   WS-3  Connect and receive snapshot broadcast
//!   WS-4  Connect and receive workflow_done broadcast
//!   WS-5  Connect and receive error broadcast
//!   WS-6  Multiple clients in same room all receive broadcasts
//!   WS-7  Different run_ids are isolated
//!   WS-8  Client disconnect does not affect other clients
//!   WS-9  Connect and receive all message types round-trip

#![allow(missing_docs)]
#[expect(unused_imports)]
use futures as _futures;
#[expect(unused_imports)]
use nexus_engine as _nexus_engine;
#[expect(unused_imports)]
use reqwest as _reqwest;
#[expect(unused_imports)]
use rusqlite as _rusqlite;
#[expect(unused_imports)]
use serde as _serde;
#[expect(unused_imports)]
use tower as _tower;
#[expect(unused_imports)]
use tracing as _tracing;
#[expect(unused_imports)]
use tracing_subscriber as _tracing_subscriber;

use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use futures_util::StreamExt;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tower_http::cors::CorsLayer;

use nexus_dashboard::{api, db::Store, state::AppState, ws, ws::WsRoom};

/// Helper: start a test server, return (base_url, store, room).
async fn spawn_server() -> (String, Store, Arc<WsRoom>) {
    let dir = std::env::temp_dir().join(format!("nexus-int-ws-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).ok();
    let db_path = dir.join("test.db");

    let store = Store::new(Some(db_path)).expect("test db creation");
    let room = WsRoom::new();
    let state = AppState {
        store: store.clone(),
        room: room.clone(),
    };

    let app: Router = Router::new()
        .route("/ws/runs/{run_id}", get(ws::ws_handler))
        .nest("/api", api::routes())
        .with_state(state)
        .layer(CorsLayer::permissive());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind random port");
    let addr = listener.local_addr().expect("get local addr");
    let base_url = format!("http://{addr}");

    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("server runs");
    });

    (base_url, store, room)
}

/// Connect a WS client to the given run_id and return the stream.
async fn ws_connect(base_url: &str, run_id: &str) -> impl futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin {
    let ws_url = base_url.replace("http://", "ws://");
    let (ws_stream, _) = connect_async(format!("{ws_url}/ws/runs/{run_id}"))
        .await
        .expect("ws connect");
    ws_stream
}

/// Receive one message from the WS stream, parse as JSON, return the Value.
async fn recv_json(
    stream: &mut (impl futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
) -> serde_json::Value {
    let msg = tokio::time::timeout(std::time::Duration::from_secs(3), stream.next())
        .await
        .expect("timeout waiting for WS message")
        .expect("stream ended")
        .expect("WS error");
    match msg {
        Message::Text(text) => {
            serde_json::from_str(&text).expect("parse WS message as JSON")
        }
        other => panic!("expected Text message, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// WS-1: Connect and receive node_status broadcast
// ---------------------------------------------------------------------------
#[tokio::test]
async fn ws_01_node_status() {
    let (base_url, _store, room) = spawn_server().await;
    let run_id = uuid::Uuid::new_v4().to_string();

    let mut ws = ws_connect(&base_url, &run_id).await;

    // Broadcast from the server side
    room.broadcast(&run_id, ws::ServerMessage::node_status("fetch", "Running"))
        .await;

    let val = recv_json(&mut ws).await;
    assert_eq!(val["type"], "node_status", "message type should be node_status");
    assert_eq!(val["data"]["node_id"], "fetch");
    assert_eq!(val["data"]["status"], "Running");
    assert!(val["data"]["ts"].is_u64(), "ts should be a u64");
}

// ---------------------------------------------------------------------------
// WS-2: Connect and receive node_chunk broadcast
// ---------------------------------------------------------------------------
#[tokio::test]
async fn ws_02_node_chunk() {
    let (base_url, _store, room) = spawn_server().await;
    let run_id = uuid::Uuid::new_v4().to_string();

    let mut ws = ws_connect(&base_url, &run_id).await;

    room.broadcast(&run_id, ws::ServerMessage::node_chunk("parse", "parsing input..."))
        .await;

    let val = recv_json(&mut ws).await;
    assert_eq!(val["type"], "node_chunk");
    assert_eq!(val["data"]["node_id"], "parse");
    assert_eq!(val["data"]["text"], "parsing input...");
    assert!(val["data"]["ts"].is_u64());
}

// ---------------------------------------------------------------------------
// WS-3: Connect and receive snapshot broadcast
// ---------------------------------------------------------------------------
#[tokio::test]
async fn ws_03_snapshot() {
    let (base_url, _store, room) = spawn_server().await;
    let run_id = uuid::Uuid::new_v4().to_string();

    let mut ws = ws_connect(&base_url, &run_id).await;

    let mut nodes = std::collections::HashMap::new();
    nodes.insert("A".into(), "Completed".into());
    nodes.insert("B".into(), "Running".into());
    room.broadcast(&run_id, ws::ServerMessage::snapshot(1, 42, nodes))
        .await;

    let val = recv_json(&mut ws).await;
    assert_eq!(val["type"], "snapshot");
    assert_eq!(val["data"]["running_count"], 1);
    assert_eq!(val["data"]["elapsed_secs"], 42);
    assert!(val["data"]["nodes"].is_object());
    assert_eq!(val["data"]["nodes"]["A"], "Completed");
    assert_eq!(val["data"]["nodes"]["B"], "Running");
}

// ---------------------------------------------------------------------------
// WS-4: Connect and receive workflow_done broadcast
// ---------------------------------------------------------------------------
#[tokio::test]
async fn ws_04_workflow_done() {
    let (base_url, _store, room) = spawn_server().await;
    let run_id = uuid::Uuid::new_v4().to_string();

    let mut ws = ws_connect(&base_url, &run_id).await;

    room.broadcast(&run_id, ws::ServerMessage::workflow_done("completed", 99))
        .await;

    let val = recv_json(&mut ws).await;
    assert_eq!(val["type"], "workflow_done");
    assert_eq!(val["data"]["status"], "completed");
    assert_eq!(val["data"]["duration_secs"], 99);
}

// ---------------------------------------------------------------------------
// WS-5: Connect and receive error broadcast
// ---------------------------------------------------------------------------
#[tokio::test]
async fn ws_05_error() {
    let (base_url, _store, room) = spawn_server().await;
    let run_id = uuid::Uuid::new_v4().to_string();

    let mut ws = ws_connect(&base_url, &run_id).await;

    room.broadcast(&run_id, ws::ServerMessage::error("something went wrong"))
        .await;

    let val = recv_json(&mut ws).await;
    assert_eq!(val["type"], "error");
    assert_eq!(val["data"]["message"], "something went wrong");
}

// ---------------------------------------------------------------------------
// WS-6: Multiple clients in same room all receive broadcasts
// ---------------------------------------------------------------------------
#[tokio::test]
async fn ws_06_multiple_clients() {
    let (base_url, _store, room) = spawn_server().await;
    let run_id = uuid::Uuid::new_v4().to_string();

    let mut ws1 = ws_connect(&base_url, &run_id).await;
    let mut ws2 = ws_connect(&base_url, &run_id).await;

    room.broadcast(&run_id, ws::ServerMessage::node_status("n1", "Running"))
        .await;

    let val1 = recv_json(&mut ws1).await;
    let val2 = recv_json(&mut ws2).await;

    assert_eq!(val1["data"]["node_id"], "n1");
    assert_eq!(val2["data"]["node_id"], "n1");
}

// ---------------------------------------------------------------------------
// WS-7: Different run_ids are isolated (messages go to correct room)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn ws_07_room_isolation() {
    let (base_url, _store, room) = spawn_server().await;
    let run_a = uuid::Uuid::new_v4().to_string();
    let run_b = uuid::Uuid::new_v4().to_string();

    let mut ws_a = ws_connect(&base_url, &run_a).await;
    let mut ws_b = ws_connect(&base_url, &run_b).await;

    // Broadcast only to run_a
    room.broadcast(&run_a, ws::ServerMessage::node_status("only-a", "Completed"))
        .await;

    // Client A receives it
    let val_a = recv_json(&mut ws_a).await;
    assert_eq!(val_a["data"]["node_id"], "only-a");

    // Client B should NOT receive anything — verify with timeout
    let result = tokio::time::timeout(std::time::Duration::from_millis(300), ws_b.next()).await;
    assert!(result.is_err(), "client B should not receive messages from room A");
}

// ---------------------------------------------------------------------------
// WS-8: Client disconnect does not affect other clients
// ---------------------------------------------------------------------------
#[tokio::test]
async fn ws_08_disconnect_isolation() {
    let (base_url, _store, room) = spawn_server().await;
    let run_id = uuid::Uuid::new_v4().to_string();

    let mut ws1 = ws_connect(&base_url, &run_id).await;
    let mut ws2 = ws_connect(&base_url, &run_id).await;

    // Broadcast first to confirm both are alive, then drop ws1.
    room.broadcast(&run_id, ws::ServerMessage::node_status("pre", "Running"))
        .await;
    let _v1 = recv_json(&mut ws1).await;
    let _v2 = recv_json(&mut ws2).await;

    // Drop ws1
    drop(ws1);

    // Broadcast again — ws2 should still get it
    room.broadcast(&run_id, ws::ServerMessage::node_status("post", "Completed"))
        .await;

    let val2 = recv_json(&mut ws2).await;
    assert_eq!(val2["data"]["node_id"], "post");
}

// ---------------------------------------------------------------------------
// WS-9: Connect and receive all message types round-trip
// ---------------------------------------------------------------------------
#[tokio::test]
async fn ws_09_all_message_types() {
    let (base_url, _store, room) = spawn_server().await;
    let run_id = uuid::Uuid::new_v4().to_string();

    let mut ws = ws_connect(&base_url, &run_id).await;

    // Send each message type and verify
    room.broadcast(&run_id, ws::ServerMessage::node_status("n1", "Running"))
        .await;
    let v = recv_json(&mut ws).await;
    assert_eq!(v["type"], "node_status");

    room.broadcast(&run_id, ws::ServerMessage::node_chunk("n1", "data"))
        .await;
    let v = recv_json(&mut ws).await;
    assert_eq!(v["type"], "node_chunk");

    let mut nodes = std::collections::HashMap::new();
    nodes.insert("n1".into(), "Running".into());
    room.broadcast(&run_id, ws::ServerMessage::snapshot(1, 5, nodes))
        .await;
    let v = recv_json(&mut ws).await;
    assert_eq!(v["type"], "snapshot");

    room.broadcast(&run_id, ws::ServerMessage::workflow_done("completed", 10))
        .await;
    let v = recv_json(&mut ws).await;
    assert_eq!(v["type"], "workflow_done");

    room.broadcast(&run_id, ws::ServerMessage::error("err"))
        .await;
    let v = recv_json(&mut ws).await;
    assert_eq!(v["type"], "error");
}
