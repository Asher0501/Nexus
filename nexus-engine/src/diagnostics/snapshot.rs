//! Runtime snapshots for workflow diagnosis.
//!
//! A [`EngineSnapshot`] is a point-in-time view of the engine's runtime state,
//! constructed from the [`Scheduler`]'s public API and caller-provided
//! metadata (running count, start instant).  It does NOT mutate or lock
//! any engine state.
//!
//! # Design
//!
//! The snapshot types mirror the internal runtime types intentionally:
//! they are decoupled so that diagnostics can evolve independently
//! without bloating engine structs with display concerns.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::graph::scheduler::{NodeCounters, NodeResult, NodeStatus, Scheduler};

/// A point-in-time view of a single node's runtime state.
#[derive(Debug, Clone)]
pub struct NodeSnapshot {
    /// Node identifier from the workflow definition.
    pub id: String,
    /// Current execution status.
    pub status: NodeStatus,
    /// Current result (if terminal).
    pub result: NodeResult,
    /// Per-event-type counters.
    pub counters: NodeCounters,
    /// Number of retries so far.
    pub retry_count: u64,
}

/// A point-in-time view of the entire engine's runtime state.
#[derive(Debug, Clone)]
pub struct EngineSnapshot {
    /// Nodes, keyed by node_id (workflow-definition ID).
    pub nodes: HashMap<String, NodeSnapshot>,
    /// Number of nodes currently executing.
    pub running_count: usize,
    /// Wall-clock time since the engine started.
    pub elapsed: Duration,
}

impl EngineSnapshot {
    /// Capture a snapshot from the scheduler and engine metadata.
    ///
    /// The snapshot is derived from the scheduler's public state API;
    /// no locks are acquired and no side effects are produced.
    #[must_use]
    pub fn capture(
        scheduler: &Scheduler,
        running_count: usize,
        started_at: Instant,
    ) -> Self {
        let mut nodes: HashMap<String, NodeSnapshot> = HashMap::new();

        for idx in scheduler.graph().node_indices() {
            let id = scheduler
                .graph()
                .node_weight(idx)
                .map(|nd| nd.id.clone())
                .unwrap_or_default();

            let status = scheduler
                .state()
                .states
                .get(&idx)
                .map(|ns| ns.status)
                .unwrap_or(NodeStatus::Pending);

            let result = scheduler
                .state()
                .states
                .get(&idx)
                .map(|ns| ns.result.clone())
                .unwrap_or(NodeResult::None);

            let counters = scheduler
                .state()
                .counters
                .get(&idx)
                .cloned()
                .unwrap_or_default();

            let retry_count = scheduler
                .state()
                .retry_counts
                .get(&idx)
                .copied()
                .unwrap_or(0);

            nodes.insert(
                id.clone(),
                NodeSnapshot {
                    id,
                    status,
                    result,
                    counters,
                    retry_count,
                },
            );
        }

        Self {
            nodes,
            running_count,
            elapsed: started_at.elapsed(),
        }
    }
}

/// A human-readable summary of the snapshot for log output.
impl std::fmt::Display for EngineSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "=== Engine Snapshot (elapsed: {:?}) ===", self.elapsed)?;
        writeln!(f, "  running_count: {}", self.running_count)?;
        writeln!(f, "  nodes:")?;

        let mut sorted_ids: Vec<_> = self.nodes.keys().collect();
        sorted_ids.sort();

        for id in sorted_ids {
            let ns = &self.nodes[id];
            write!(f, "    {id}: {:?}", ns.status)?;
            if !matches!(ns.result, NodeResult::None) {
                write!(f, " ({:?})", ns.result)?;
            }
            writeln!(
                f,
                "  [ok:{} fail:{} timeout:{} retry:{}]",
                ns.counters.complete, ns.counters.failed, ns.counters.timeout, ns.retry_count,
            )?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::scheduler::Scheduler;
    use crate::graph::graph_def::GraphDef;
    use petgraph::stable_graph::StableDiGraph;
    use crate::graph::edge::EdgeDef;
    use crate::graph::graph_def::{NodeData, NodeTransfer, NodeParams};
    use crate::model::predecessor::EventType;
    use std::collections::HashMap;
    use std::time::Instant;

    fn build_chain_graph_def() -> GraphDef {
        let mut graph = StableDiGraph::new();
        let a = graph.add_node(NodeData {
            id: "A".into(),
            providers: vec![],
            process_timeout_secs: 10,
            max_concurrency: 1,
        });
        let b = graph.add_node(NodeData {
            id: "B".into(),
            providers: vec![],
            process_timeout_secs: 10,
            max_concurrency: 1,
        });

        let mut index = HashMap::new();
        index.insert("A".into(), a);
        index.insert("B".into(), b);

        let edge = EdgeDef {
            from: a,
            to: b,
            event_type: EventType::Complete,
            exit_reason: None,
            threshold: 1,
            strategy: crate::graph::edge::Strategy::All,
        };

        let mut transfers = HashMap::new();
        transfers.insert(a, NodeTransfer { from: a, out_edge_indices: vec![0] });
        // B has no outgoing edges — empty out_edge_indices.
        transfers.insert(b, NodeTransfer { from: b, out_edge_indices: vec![] });

        let mut params = HashMap::new();
        params.insert(a, NodeParams { process_timeout_secs: 10 });
        params.insert(b, NodeParams { process_timeout_secs: 10 });

        GraphDef::from_components(graph, index, vec![edge], transfers, params, vec![a])
            .expect("valid chain graph")
    }

    #[test]
    fn test_snapshot_capture() {
        let scheduler = Scheduler::new(build_chain_graph_def());
        let started_at = Instant::now()
            .checked_sub(Duration::from_secs(5))
            .unwrap();

        let snapshot = EngineSnapshot::capture(&scheduler, 1, started_at);

        assert_eq!(snapshot.nodes.len(), 2);
        assert!(snapshot.nodes.contains_key("A"));
        assert!(snapshot.nodes.contains_key("B"));
        assert_eq!(snapshot.running_count, 1);
        assert!(snapshot.elapsed >= Duration::from_secs(5));
    }

    #[test]
    fn test_snapshot_display() {
        let scheduler = Scheduler::new(build_chain_graph_def());
        let snapshot = EngineSnapshot::capture(&scheduler, 0, Instant::now());
        let display = snapshot.to_string();
        assert!(display.contains("Engine Snapshot"));
        assert!(display.contains("A"));
        assert!(display.contains("B"));
    }
}
