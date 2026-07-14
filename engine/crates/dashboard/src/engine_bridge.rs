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
/// If `cancel_flag` is provided, external callers can set it to `true` to
/// stop the engine at the next event-loop iteration.
pub async fn run_workflow(
    json: &str,
    config: EngineConfig,
    room: Arc<WsRoom>,
    run_id: &str,
    cancel_flag: Option<Arc<std::sync::atomic::AtomicBool>>,
) -> Result<(), String> {
    let def: WorkflowDef = serde_json::from_str(json).map_err(|e| format!("JSON parse error: {e}"))?;
    let dummy_cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    if let Err(errors) = validate(&def) {
        let msg: Vec<String> = errors.iter().map(std::string::ToString::to_string).collect();
        return Err(msg.join("; "));
    }

    // Ensure log directory exists
    let _ = std::fs::create_dir_all("log");
    let log_path = format!("log/run-{}.log", run_id);
    let log_file = std::sync::Mutex::new(
        std::fs::OpenOptions::new()
            .create(true).append(true)
            .open(&log_path)
            .unwrap_or_else(|_| std::fs::File::create(&log_path).unwrap()),
    );

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
            NodeEvent::NodeChunk { ref node_id, ref text } => {
                // Persist chunk to run log file
                use std::io::Write;
                if let Ok(mut f) = log_file.lock() {
                    let _ = writeln!(f, "[{node_id}] {text}");
                }
                ServerMessage::node_chunk(node_id.clone(), text.clone())
            }
            NodeEvent::Lifecycle(_) => return,
        };
        let room = room_clone.clone();
        let rid = run_id_owned.clone();
        tokio::spawn(async move {
            room.broadcast(&rid, msg).await;
        });
    });

    let (mut engine, engine_cancel) =
        Engine::new_with_cancel(def, config, Some(event_cb)).map_err(|errors| {
            errors
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        })?;

    // If external cancel is provided, poll it and propagate to engine.
    if let Some(ext_cf) = cancel_flag {
        let engine_cf = engine_cancel.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                if ext_cf.load(std::sync::atomic::Ordering::Relaxed) {
                    engine_cf.store(true, std::sync::atomic::Ordering::Relaxed);
                    break;
                }
            }
        });
    }

    engine.run().await.map_err(|e| format!("{e:?}"))
}
