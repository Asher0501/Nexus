use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Execution metadata passed to a node alongside its input data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMetadata {
    /// How many times this node has been executed so far (1-based).
    pub run_count: u64,
    /// Whether the previous execution of this node timed out.
    pub timed_out: bool,
}

/// Input context passed to a node from the engine.
#[derive(Debug, Clone, Serialize)]
pub struct NodeContext {
    /// Map of upstream node IDs to their output content strings
    /// (legacy: retained for non-template use; template rendering uses `upstream`).
    pub inputs: HashMap<String, String>,
    /// Extension parameters for the node (unused in v1, reserved).
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub extensions: HashMap<String, String>,
    /// Execution metadata.
    pub metadata: NodeMetadata,
    /// Map of upstream alias → full output (route + content),
    /// for `{{datarouter.<alias>.route}}` and `{{datarouter.<alias>.content}}`.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub upstream: HashMap<String, NodeOutput>,
}

impl Default for NodeContext {
    fn default() -> Self {
        Self {
            inputs: HashMap::new(),
            extensions: HashMap::new(),
            metadata: NodeMetadata {
                run_count: 1,
                timed_out: false,
            },
            upstream: HashMap::new(),
        }
    }
}

/// An output chunk emitted by a node during execution (streaming).
///
/// This is distinct from the final [`NodeOutput`], which is the complete
/// structured result. A node may emit multiple chunks as it runs.
#[derive(Debug, Clone)]
pub struct NodeChunk {
    /// The output text line.
    pub text: String,
}

/// A structured output from a node, carrying both the routing key
/// and the content payload. Emitted as JSON on stdout so the engine
/// can match edges by route and forward content via the DataRouter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeOutput {
    /// Logical route key for edge matching.
    pub route: String,
    /// The content payload.
    #[serde(default)]
    pub content: String,
}

/// Exit code sentinel values for the engine.
///
/// - `0`: node exited normally (exit 0)
/// - non-zero positive: node self-reported error (exit N)
/// - `-1`: wait failed or child produced no exit code
/// - `-9`: killed by engine due to timeout
pub mod exit_codes {
    /// Process exited normally with code 0.
    #[allow(dead_code)]
    pub const SUCCESS: i32 = 0;
    /// wait() failed or no exit code available.
    pub const WAIT_FAILED: i32 = -1;
    /// Killed by engine due to timeout.
    pub const TIMEOUT: i32 = -9;
}

/// The outcome of executing a node.
#[derive(Debug, Clone)]
pub struct NodeOutcome {
    /// Structured output produced by the node.
    pub output: NodeOutput,
    /// Process exit code:
    ///  0  = success
    ///  -1 = wait failed / no exit code
    ///  -9 = killed by timeout
    ///  N  = node self-reported error
    pub exit_code: i32,
    /// Exit reason extracted from stdout header.
    pub exit_reason: Option<String>,
}

impl NodeOutcome {
    /// Whether the node was killed due to timeout (exit_code == -9).
    #[must_use]
    pub fn timed_out(&self) -> bool {
        self.exit_code == exit_codes::TIMEOUT
    }
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
