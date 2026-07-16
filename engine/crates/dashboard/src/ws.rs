use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// All WebSocket message variants sent from server to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ServerMessage {
    #[serde(rename = "node_status")]
    NodeStatus(NodeStatusPayload),
    #[serde(rename = "node_chunk")]
    NodeChunk(NodeChunkPayload),
    #[serde(rename = "snapshot")]
    Snapshot(SnapshotPayload),
    #[serde(rename = "workflow_done")]
    WorkflowDone(WorkflowDonePayload),
    #[serde(rename = "error")]
    Error(ErrorMessagePayload),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStatusPayload {
    pub node_id: String,
    pub status: String, // "Pending" | "Running" | "Completed" | "Failed" | "TimedOut"
    pub ts: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeChunkPayload {
    pub node_id: String,
    pub text: String,
    pub ts: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotPayload {
    pub running_count: usize,
    pub elapsed_secs: u64,
    pub nodes: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDonePayload {
    pub status: String, // "completed" | "failed" | "timeout"
    pub duration_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorMessagePayload {
    pub message: String,
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl ServerMessage {
    pub fn node_status(node_id: impl Into<String>, status: impl Into<String>) -> Self {
        Self::NodeStatus(NodeStatusPayload {
            node_id: node_id.into(),
            status: status.into(),
            ts: now_ts(),
        })
    }

    pub fn node_chunk(node_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self::NodeChunk(NodeChunkPayload {
            node_id: node_id.into(),
            text: text.into(),
            ts: now_ts(),
        })
    }

    #[must_use]
    pub const fn snapshot(
        running_count: usize,
        elapsed_secs: u64,
        nodes: HashMap<String, String>,
    ) -> Self {
        Self::Snapshot(SnapshotPayload {
            running_count,
            elapsed_secs,
            nodes,
        })
    }

    pub fn workflow_done(status: impl Into<String>, duration_secs: u64) -> Self {
        Self::WorkflowDone(WorkflowDonePayload {
            status: status.into(),
            duration_secs,
        })
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::Error(ErrorMessagePayload {
            message: message.into(),
        })
    }
}

// ---------------------------------------------------------------------------
// WsRoom — per-run_id broadcast channel
// ---------------------------------------------------------------------------

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, State};
use tokio::sync::{mpsc, Mutex};

/// Sender half of an unbounded channel for a single WS connection.
pub type Tx = mpsc::UnboundedSender<ServerMessage>;

/// WebSocket room manager: each `run_id` maps to a set of senders.
///
/// Disconnected senders (where `tx.send()` returns `Err`) are cleaned up
/// lazily during `broadcast()` to prevent unbounded memory growth.
///
/// # Usage (server side)
/// ```ignore
/// let room = WsRoom::new();
/// // In WS upgrade handler:
/// let mut rx = room.join("run-123").await;
/// // In engine callback:
/// room.broadcast("run-123", ServerMessage::node_status("n1", "Running")).await;
/// ```
pub struct WsRoom {
    rooms: Mutex<HashMap<String, Vec<Tx>>>,
}

impl WsRoom {
    /// Create a new shared room manager.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            rooms: Mutex::new(HashMap::new()),
        })
    }

    /// Join a room — returns a receiver that yields messages broadcast to `run_id`.
    pub async fn join(self: &Arc<Self>, run_id: &str) -> mpsc::UnboundedReceiver<ServerMessage> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.rooms
            .lock()
            .await
            .entry(run_id.to_string())
            .or_default()
            .push(tx);
        rx
    }

    /// Send a message to every connection in the room.
    ///
    /// Dead senders (whose receivers have been dropped) are pruned
    /// from the room during this call to prevent unbounded growth.
    pub async fn broadcast(&self, run_id: &str, msg: ServerMessage) {
        let mut rooms = self.rooms.lock().await;
        if let Some(senders) = rooms.get_mut(run_id) {
            senders.retain(|tx| tx.send(msg.clone()).is_ok());
            if senders.is_empty() {
                rooms.remove(run_id);
            }
        }
    }
}

/// WebSocket upgrade handler — upgrades HTTP to WS and pumps room messages.
pub async fn ws_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    Path(run_id): Path<String>,
    State(room): State<Arc<WsRoom>>,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, run_id, room))
}

/// Pump broadcast messages from the room into the WebSocket connection.
pub async fn handle_socket(mut socket: WebSocket, run_id: String, room: Arc<WsRoom>) {
    let mut rx = room.join(&run_id).await;
    loop {
        tokio::select! {
            Some(msg) = rx.recv() => {
                let json = serde_json::to_string(&msg).unwrap_or_default();
                if socket.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
            Some(Ok(Message::Close(_))) | None = socket.recv() => {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Contract test: NodeStatus message format.
    #[test]
    fn test_ws_node_status_format() {
        let msg = ServerMessage::node_status("fetch", "Running");
        let json = serde_json::to_string_pretty(&msg).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["type"], "node_status");
        assert_eq!(parsed["data"]["node_id"], "fetch");
        assert_eq!(parsed["data"]["status"], "Running");
        assert!(parsed["data"]["ts"].is_u64());
    }

    /// Contract test: all valid status enum values.
    #[test]
    fn test_ws_node_status_all_values() {
        for status in &["Pending", "Running", "Completed", "Failed", "TimedOut"] {
            let msg = ServerMessage::node_status("n1", *status);
            let json = serde_json::to_string(&msg).unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed["data"]["status"].as_str(), Some(*status));
        }
    }

    /// Contract test: NodeChunk message format.
    #[test]
    fn test_ws_node_chunk_format() {
        let msg = ServerMessage::node_chunk("review", "正在审查代码...");
        let json = serde_json::to_string_pretty(&msg).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["type"], "node_chunk");
        assert_eq!(parsed["data"]["node_id"], "review");
        assert_eq!(parsed["data"]["text"], "正在审查代码...");
        assert!(parsed["data"]["ts"].is_u64());
    }

    /// Contract test: Snapshot message format.
    #[test]
    fn test_ws_snapshot_format() {
        let mut nodes = HashMap::new();
        nodes.insert("A".into(), "Completed".into());
        nodes.insert("B".into(), "Running".into());

        let msg = ServerMessage::snapshot(1, 12, nodes);
        let json = serde_json::to_string_pretty(&msg).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["type"], "snapshot");
        assert_eq!(parsed["data"]["running_count"], 1);
        assert_eq!(parsed["data"]["elapsed_secs"], 12);
        assert!(parsed["data"]["nodes"].is_object());
        assert_eq!(parsed["data"]["nodes"]["A"], "Completed");
        assert_eq!(parsed["data"]["nodes"]["B"], "Running");
    }

    /// Contract test: WorkflowDone message format.
    #[test]
    fn test_ws_workflow_done_format() {
        let msg = ServerMessage::workflow_done("completed", 45);
        let json = serde_json::to_string_pretty(&msg).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["type"], "workflow_done");
        assert_eq!(parsed["data"]["status"], "completed");
        assert_eq!(parsed["data"]["duration_secs"], 45);
    }

    /// Contract test: all workflow_done status enum values.
    #[test]
    fn test_ws_workflow_done_all_status() {
        for status in &["completed", "failed", "timeout"] {
            let msg = ServerMessage::workflow_done(*status, 10);
            let json = serde_json::to_string(&msg).unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed["data"]["status"].as_str(), Some(*status));
        }
    }

    /// Contract test: ErrorMessage format.
    #[test]
    fn test_ws_error_format() {
        let msg = ServerMessage::error("workflow not found");
        let json = serde_json::to_string_pretty(&msg).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["type"], "error");
        assert_eq!(parsed["data"]["message"], "workflow not found");
    }

    /// WsRoom: join and receive a broadcast message.
    #[tokio::test]
    async fn test_ws_room_broadcast() {
        let room = WsRoom::new();
        let mut rx = room.join("test-run").await;

        room.broadcast("test-run", ServerMessage::node_status("fetch", "Completed"))
            .await;

        let msg = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "node_status");
        assert_eq!(json["data"]["node_id"], "fetch");
        assert_eq!(json["data"]["status"], "Completed");
    }

    /// WsRoom: broadcast to a non-existent room is a no-op (does not panic).
    #[tokio::test]
    async fn test_ws_room_broadcast_noop() {
        let room = WsRoom::new();
        // Should not panic
        room.broadcast("nonexistent", ServerMessage::node_status("x", "Running"))
            .await;
    }

    /// WsRoom: multiple receivers in the same room all get the message.
    #[tokio::test]
    async fn test_ws_room_multiple_receivers() {
        let room = WsRoom::new();
        let mut rx1 = room.join("multi").await;
        let mut rx2 = room.join("multi").await;

        room.broadcast("multi", ServerMessage::node_status("n1", "Running"))
            .await;

        for rx in [&mut rx1, &mut rx2] {
            let msg = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
                .await
                .unwrap()
                .unwrap();
            let json = serde_json::to_value(&msg).unwrap();
            assert_eq!(json["data"]["node_id"], "n1");
        }
    }

    /// Contract test: serialized JSON round-trips back to the same type.
    #[test]
    fn test_ws_roundtrip_all_variants() {
        let cases = vec![
            ServerMessage::node_status("a", "Running"),
            ServerMessage::node_chunk("a", "text"),
            ServerMessage::snapshot(0, 0, HashMap::new()),
            ServerMessage::workflow_done("completed", 0),
            ServerMessage::error("err"),
        ];
        for original in cases {
            let json = serde_json::to_string(&original).unwrap();
            let recovered: ServerMessage = serde_json::from_str(&json).unwrap();
            // Verify discriminant is preserved
            let original_tag = serde_json::to_value(&original).unwrap();
            let recovered_tag = serde_json::to_value(&recovered).unwrap();
            assert_eq!(original_tag["type"], recovered_tag["type"]);
        }
    }
}
