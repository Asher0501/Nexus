//! Edge definition types for the graph module.
//!
//! This module provides [`EdgeDef`] (pure data, runtime read-only), [`EdgeState`]
//! (runtime mutable state tracked by the scheduler), and [`Strategy`] (trigger
//! combination logic).

use std::collections::HashSet;

use petgraph::graph::NodeIndex;
use serde::{Deserialize, Serialize};

use crate::model::predecessor::EventType;

/// Trigger strategy for an edge.
///
/// Determines when an edge should be evaluated:
/// - `All`: All `from_nodes` must signal before the event is counted.
/// - `Any`: Any single `from_node` signal suffices.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Strategy {
    /// All `from_nodes` must participate before counting.
    All,
    /// Any single `from_node` signal is sufficient.
    Any,
}

/// Edge definition — pure data, runtime read-only.
///
/// Represents a directed trigger edge in the graph. When a node emits an event
/// matching this edge's criteria and the strategy/threshold conditions are met,
/// the target node (`to`) is enqueued for execution.
#[derive(Debug, Clone)]
pub struct EdgeDef {
    /// Source node indices whose events drive this edge.
    pub from_nodes: Vec<NodeIndex>,
    /// Target node index (the node triggered by this edge).
    pub to: NodeIndex,
    /// Event type that this edge responds to.
    pub event_type: EventType,
    /// Optional `exit_reason` filter. When `Some`, only events whose reason
    /// matches this string are accepted.
    pub exit_reason: Option<String>,
    /// Number of matching events required before the edge fires.
    pub threshold: u64,
    /// How `from_nodes` events are combined.
    pub strategy: Strategy,
}

/// Edge runtime state — mutated by the scheduler during graph execution.
///
/// Tracks whether this edge has already fired, how many matching events have
/// been received, and which `from_nodes` have signalled (for `Strategy::All`).
#[derive(Debug, Clone, Default)]
pub struct EdgeState {
    /// Whether this edge has already fired.
    pub triggered: bool,
    /// Number of matching events received so far.
    pub event_count: u64,
    /// Set of `from_nodes` that have signalled (used by `Strategy::All`).
    pub received: HashSet<NodeIndex>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that an `EdgeState` created with `Default::default()` has all
    /// fields at their zero/empty values.
    #[test]
    fn test_edge_state_defaults() {
        let state = EdgeState::default();
        assert!(!state.triggered, "default triggered must be false");
        assert_eq!(state.event_count, 0, "default event_count must be 0");
        assert!(
            state.received.is_empty(),
            "default received set must be empty"
        );
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
        assert_eq!(state.triggered, false);

        state.triggered = true;
        state.event_count = 5;
        state.received.insert(NodeIndex::new(0));
        state.received.insert(NodeIndex::new(1));

        assert!(state.triggered);
        assert_eq!(state.event_count, 5);
        assert_eq!(state.received.len(), 2);
        assert!(state.received.contains(&NodeIndex::new(0)));
        assert!(state.received.contains(&NodeIndex::new(1)));
    }

    /// Verify that `EdgeDef` can be constructed with both `Strategy` variants.
    #[test]
    fn test_edge_def_with_strategies() {
        let a = NodeIndex::new(0);
        let b = NodeIndex::new(1);

        let edge_all = EdgeDef {
            from_nodes: vec![a],
            to: b,
            event_type: EventType::Complete,
            exit_reason: None,
            threshold: 1,
            strategy: Strategy::All,
        };
        assert_eq!(edge_all.strategy, Strategy::All);

        let edge_any = EdgeDef {
            from_nodes: vec![a, NodeIndex::new(2)],
            to: b,
            event_type: EventType::Failed,
            exit_reason: Some("crash".into()),
            threshold: 3,
            strategy: Strategy::Any,
        };
        assert_eq!(edge_any.strategy, Strategy::Any);
        assert_eq!(edge_any.from_nodes.len(), 2);
    }
}
