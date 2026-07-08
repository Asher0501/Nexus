use serde::{Deserialize, Serialize};

use crate::model::predecessor::{DataFlowDef, SchedulingEdgeDef};
use crate::model::provider::ProviderDef;

/// The top-level workflow definition, deserialized from JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowDef {
    /// All nodes in this workflow.
    pub nodes: Vec<NodeDef>,

    /// Scheduling topology edges.
    #[serde(default)]
    pub edges: Vec<SchedulingEdgeDef>,

    /// Data flow topology edges.
    #[serde(default)]
    pub dataflows: Vec<DataFlowDef>,
}

/// Definition of a single node in the workflow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeDef {
    /// Unique identifier for this node within the workflow.
    pub id: String,

    /// How this node is executed (one or more providers).
    pub providers: Vec<ProviderDef>,

    /// Maximum time in seconds before the process is killed.
    pub process_timeout_secs: u64,

    /// Declared return values for branch routing.
    #[serde(default)]
    pub returns: Vec<String>,

    /// Per-node max retries on failure (None = inherit global default 3).
    #[serde(default)]
    pub max_retries: Option<u64>,
}
