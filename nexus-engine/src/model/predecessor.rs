use serde::{Deserialize, Serialize};

/// Default threshold value (1 = trigger after first occurrence).
pub fn default_threshold() -> u64 {
    1
}

/// The combination logic for multiple predecessors on the same edge.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TriggerExpr {
    /// All listed predecessors must participate before counting begins.
    #[serde(rename = "all")]
    All,
    /// Any predecessor event is counted directly.
    #[serde(rename = "any")]
    Any,
}

/// The type of event that occurred during node execution.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventType {
    /// Process exited with code 0.
    #[serde(rename = "complete")]
    Complete,
    /// Process exited with non-zero code.
    #[serde(rename = "failed")]
    Failed,
    /// Process was killed due to timeout.
    #[serde(rename = "timeout")]
    Timeout,
}

/// A scheduling edge in the workflow graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchedulingEdgeDef {
    /// Source node ID.
    pub from: String,
    /// Target node ID.
    pub to: String,
    /// Combination logic (All or Any).
    pub trigger: TriggerExpr,
    /// Which event type triggers this edge.
    pub event: EventType,
    /// Optional exit_reason filter (string match).
    #[serde(default)]
    pub exit_reason: Option<String>,
    /// Number of matching events required before triggering.
    #[serde(default = "default_threshold")]
    pub threshold: u64,
}

/// A data flow edge connecting an output to an input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataFlowDef {
    /// Source node ID providing the data.
    pub from: String,
    /// Target node ID receiving the data.
    pub to: String,
    /// Key in the target node's inputs; defaults to the source node ID when absent.
    #[serde(default)]
    pub alias: Option<String>,
}

/// Declaration of a predecessor relationship from the workflow JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PredecessorDef {
    /// The upstream node ID.
    pub node_id: String,

    /// Combination logic (All or Any).
    pub trigger: TriggerExpr,

    /// Which event type triggers this edge.
    pub event: EventType,

    /// Optional exit_reason filter (string match).
    #[serde(default)]
    pub exit_reason: Option<String>,

    /// Number of matching events required before triggering.
    #[serde(default = "default_threshold")]
    pub threshold: u64,
}
