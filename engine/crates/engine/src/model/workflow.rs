use serde::{Deserialize, Serialize};

use crate::model::predecessor::{DataFlowDef, SchedulingEdgeDef};
use crate::model::provider::ProviderDef;

/// Route policy configuration for a node.
///
/// When configured, `NodeShell` can override the node's stdout route based on
/// system state (e.g., `run_count`, cumulative duration). This enables cycle
/// termination without the node itself knowing about loop boundaries.
///
/// # Variants
///
/// - `MaxRuns`: After `max` executions, `NodeShell` overrides the `exit_reason`
///   to `then_route`, causing a different edge to fire and breaking the loop.
/// - `MaxDuration`: When cumulative execution time across all runs exceeds
///   `max_secs`, `NodeShell` overrides the `exit_reason` to `then_route`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RoutePolicyDef {
    /// Override the node's route after N runs.
    #[serde(rename = "max_runs")]
    MaxRuns {
        /// Run count threshold. When `run_count` >= max, the policy activates.
        max: u64,
        /// Route value to use when the policy activates.
        then_route: String,
    },
    /// Override the node's route after cumulative N seconds of execution.
    #[serde(rename = "max_duration")]
    MaxDuration {
        /// Cumulative execution time threshold in seconds.
        max_secs: u64,
        /// Route value to use when the policy activates.
        then_route: String,
    },
}

/// The top-level workflow definition, deserialized from JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDef {
    /// All nodes in this workflow.
    pub nodes: Vec<NodeDef>,

    /// Scheduling topology edges.
    #[serde(default)]
    pub edges: Vec<SchedulingEdgeDef>,

    /// Data flow topology edges.
    #[serde(default)]
    pub dataflows: Vec<DataFlowDef>,

    /// Global scripts directory for all nodes.
    ///
    /// Node-level [`NodeDef::scripts_dir`] takes precedence over this value.
    /// Falls back to `NEXUS_SCRIPTS_DIR` env var, then exe-relative search.
    #[serde(default)]
    pub scripts_dir: Option<String>,
}

/// Definition of a single node in the workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

    /// Route policy: when configured, `NodeShell` may override the node's
    /// stdout route based on system state (e.g., `run_count`) before edge
    /// matching. See `WORKFLOW_REFERENCE.md` §2.5.
    #[serde(default)]
    pub route_policy: Option<RoutePolicyDef>,

    /// Scripts directory for this node's executors.
    ///
    /// Overrides [`WorkflowDef::scripts_dir`] and the global fallback.
    /// If `None`, inherits the workflow-level or global default.
    #[serde(default)]
    pub scripts_dir: Option<String>,
}
