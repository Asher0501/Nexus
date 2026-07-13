//! Edge definition types for the graph module.
//!
//! Each edge is a pair of two orthogonal functions: h_e (branch matching)
//! and g_e (strategy aggregation). See theory/NODE_TRANSFER.md for the
//! complete design philosophy.
//!
//! - [`EdgeDef`]: pure data, runtime read-only — defines the parameters of
//!   h_e (event_type, exit_reason, threshold) and g_e (strategy).
//! - [`EdgeState`]: runtime mutable state — tracks h_e's threshold counter
//!   (event_count). No triggered state — h_e is a pure function.

use petgraph::graph::NodeIndex;
use serde::{Deserialize, Serialize};

use crate::model::predecessor::EventType;

/// g_e strategy: how to aggregate multiple source-node readiness signals.
///
/// Corresponds to the g_e function in the h+g decomposition:
/// - `Any` (∨): single source ready → trigger downstream.
/// - `All` (∧): all sources ready → trigger downstream.
///
/// All/Any are complete (see DESIGN_PHILOSOPHY.md §〇 推论 3).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Strategy {
    /// All source nodes must signal readiness before the edge fires.
    All,
    /// Any single source node signal is sufficient.
    Any,
}

/// h_e parameters + g_e strategy — pure data, runtime read-only.
///
/// Represents a directed trigger edge e = (from, to, h_e, g_e).
/// - h_e: branch matching (event_type, exit_reason, threshold)
/// - g_e: strategy aggregation (strategy)
#[derive(Debug, Clone)]
pub struct EdgeDef {
    /// Source node index (`v ∈ V`).
    pub from: NodeIndex,
    /// Target node index (`w ∈ V`).
    pub to: NodeIndex,
    /// h_e: event type that this edge responds to.
    pub event_type: EventType,
    /// h_e: optional exit_reason filter (exact string match).
    pub exit_reason: Option<String>,
    /// h_e: threshold — number of matching events required before the edge fires.
    pub threshold: u64,
    /// g_e: strategy — Any (∨) or All (∧).
    pub strategy: Strategy,
}

/// Edge runtime state — h_e's threshold counter only.
///
/// No triggered state — h_e is a pure function. Every event independently
/// evaluates all matching edges. See theory/NODE_TRANSFER.md.
#[derive(Debug, Clone, Default)]
pub struct EdgeState {
    /// h_e: number of matching events received so far (for threshold).
    pub event_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that an `EdgeState` created with `Default::default()` has all
    /// fields at their zero/empty values.
    #[test]
    fn test_edge_state_defaults() {
        let state = EdgeState::default();
        assert_eq!(state.event_count, 0, "default event_count must be 0");
    }

    /// Verify that `Strategy` can be serialized and deserialized via serde.
    #[test]
    fn test_strategy_serde() {
        let all_json = serde_json::to_string(&Strategy::All).expect("serialize All");
        let any_json = serde_json::to_string(&Strategy::Any).expect("serialize Any");

        assert_eq!(all_json, "\"All\"");
        assert_eq!(any_json, "\"Any\"");

        let deserialized_all: Strategy =
            serde_json::from_str(&all_json).expect("deserialize All");
        let deserialized_any: Strategy =
            serde_json::from_str(&any_json).expect("deserialize Any");

        assert_eq!(deserialized_all, Strategy::All);
        assert_eq!(deserialized_any, Strategy::Any);
    }

    /// Verify that an `EdgeState` can be fully populated and inspected.
    #[test]
    fn test_edge_state_can_populate() {
        let mut state = EdgeState::default();
        assert_eq!(state.event_count, 0);

        state.event_count = 5;

        assert_eq!(state.event_count, 5);
    }

    #[test]
    fn test_event_type_serde() {
        use crate::model::predecessor::EventType;

        let complete_json = serde_json::to_string(&EventType::Complete).expect("serialize Complete");
        let failed_json = serde_json::to_string(&EventType::Failed).expect("serialize Failed");
        let timeout_json = serde_json::to_string(&EventType::Timeout).expect("serialize Timeout");

        assert_eq!(complete_json, "\"complete\"");
        assert_eq!(failed_json, "\"failed\"");
        assert_eq!(timeout_json, "\"timeout\"");

        let deserialized_complete: EventType =
            serde_json::from_str(&complete_json).expect("deserialize Complete");
        let deserialized_failed: EventType =
            serde_json::from_str(&failed_json).expect("deserialize Failed");
        let deserialized_timeout: EventType =
            serde_json::from_str(&timeout_json).expect("deserialize Timeout");

        assert_eq!(deserialized_complete, EventType::Complete);
        assert_eq!(deserialized_failed, EventType::Failed);
        assert_eq!(deserialized_timeout, EventType::Timeout);

        assert_eq!(
            serde_json::to_string(&deserialized_complete).unwrap(),
            complete_json
        );
        assert_eq!(
            serde_json::to_string(&deserialized_failed).unwrap(),
            failed_json
        );
        assert_eq!(
            serde_json::to_string(&deserialized_timeout).unwrap(),
            timeout_json
        );
    }

    /// Verify that `EdgeDef` can be constructed with both `Strategy` variants.
    #[test]
    fn test_edge_def_with_strategies() {
        let a = NodeIndex::new(0);
        let b = NodeIndex::new(1);

        let edge_all = EdgeDef {
            from: a,
            to: b,
            event_type: EventType::Complete,
            exit_reason: None,
            threshold: 1,
            strategy: Strategy::All,
        };
        assert_eq!(edge_all.strategy, Strategy::All);

        let edge_any = EdgeDef {
            from: a,
            to: b,
            event_type: EventType::Failed,
            exit_reason: Some("crash".into()),
            threshold: 3,
            strategy: Strategy::Any,
        };
        assert_eq!(edge_any.strategy, Strategy::Any);
    }
}
