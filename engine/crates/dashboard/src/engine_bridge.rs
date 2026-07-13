//! Bridge between the Dashboard HTTP layer and `nexus-engine`.
//!
//! Provides `validate_workflow` for definition validation and
//! `run_workflow` for asynchronous execution with WebSocket event push.

use std::sync::Arc;

use nexus_engine::graph::validate;
use nexus_engine::model::{EngineConfig, WorkflowDef};
use nexus_engine::runtime::{Engine, NodeEvent, NodeEventCb};

use crate::ws::{ServerMessage, WsRoom};

/// Validate a workflow definition JSON string.
///
/// Returns `Ok(())` on success, or `Err` with a list of human-readable error messages.
/// This function is part of the public API; callers include the REST layer
/// and external embedders that use the dashboard as a library.
#[allow(dead_code)]
pub fn validate_workflow(json: &str) -> Result<(), Vec<String>> {
    let def: WorkflowDef =
        serde_json::from_str(json).map_err(|e| vec![format!("JSON parse error: {e}")])?;
    validate(&def).map_err(|errors| errors.iter().map(std::string::ToString::to_string).collect())
}

/// Run a workflow definition and broadcast events to the given WsRoom.
///
/// The JSON definition is parsed and validated before execution begins.
/// Node events (status changes, chunk output) are broadcast to the room
/// so all WebSocket clients subscribed to this `run_id` receive them.
pub async fn run_workflow(
    json: &str,
    config: EngineConfig,
    room: Arc<WsRoom>,
    run_id: &str,
) -> Result<(), String> {
    let def: WorkflowDef = serde_json::from_str(json).map_err(|e| format!("JSON parse error: {e}"))?;
    if let Err(errors) = validate(&def) {
        let msg: Vec<String> = errors.iter().map(std::string::ToString::to_string).collect();
        return Err(msg.join("; "));
    }

    let room_clone = room.clone();
    let run_id_owned = run_id.to_string();
    let event_cb: NodeEventCb = Arc::new(move |event| {
        let msg = match event {
            NodeEvent::NodeRunning { node_id, .. } => {
                ServerMessage::node_status(node_id, "Running")
            }
            NodeEvent::NodeCompleted { node_id } => {
                ServerMessage::node_status(node_id, "Completed")
            }
            NodeEvent::NodeFailed { node_id } => {
                ServerMessage::node_status(node_id, "Failed")
            }
            NodeEvent::NodeTimedOut { node_id } => {
                ServerMessage::node_status(node_id, "TimedOut")
            }
            NodeEvent::NodeChunk { node_id, text } => {
                ServerMessage::node_chunk(node_id, text)
            }
            NodeEvent::Lifecycle(_) => return,
        };
        let room = room_clone.clone();
        let rid = run_id_owned.clone();
        tokio::spawn(async move {
            room.broadcast(&rid, msg).await;
        });
    });

    let mut engine =
        Engine::new(def, config, Some(event_cb)).map_err(|errors| {
            errors
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        })?;

    engine.run().await.map_err(|e| format!("{e:?}"))?;
    Ok(())
}
