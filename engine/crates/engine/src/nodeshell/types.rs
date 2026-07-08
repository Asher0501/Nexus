use std::collections::HashMap;

use serde::Serialize;

/// Input context passed to a node from the engine.
#[derive(Debug, Clone, Serialize)]
pub struct NodeContext {
    /// Map of upstream node IDs to their output strings.
    pub inputs: HashMap<String, String>,
    /// Extension parameters for the node (unused in v1, reserved).
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub extensions: HashMap<String, String>,
}

/// The outcome of executing a node.
#[derive(Debug, Clone)]
pub struct NodeOutcome {
    /// Standard output produced by the node.
    pub output: String,
    /// Process exit code.
    pub exit_code: i32,
    /// Whether the node was killed due to timeout.
    pub timed_out: bool,
    /// Exit reason extracted from stdout header.
    pub exit_reason: Option<String>,
}

/// A single chunk of real-time output from a running node.
///
/// Emitted via the chunk channel during execution so the engine can
/// observe intermediate output (thinking, progress, partial results)
/// before the process exits. The full output is also accumulated
/// in [`NodeOutcome::output`].
#[derive(Debug, Clone)]
pub struct NodeChunk {
    /// The raw text line from stdout.
    pub text: String,
    /// The node that produced this chunk.
    pub node_id: String,
}

/// Error returned when a node cannot be spawned.
#[derive(Debug, Clone)]
pub struct SpawnError {
    /// Human-readable error description.
    pub message: String,
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "spawn error: {}", self.message)
    }
}

impl std::error::Error for SpawnError {}
