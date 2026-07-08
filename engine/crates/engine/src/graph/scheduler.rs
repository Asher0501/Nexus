//! Graph execution scheduler implementing the NODE_TRANSFER local closure.
//!
//! The [`Scheduler`] manages runtime state for graph traversal, processing
//! events via [`Scheduler::handle_event`], maintaining a ready queue, and
//! tracking per-node status, retry counts, and convergence.

use std::collections::{HashMap, VecDeque};

use petgraph::graph::NodeIndex;

use crate::graph::edge::{EdgeDef, EdgeState, Strategy};
use crate::graph::graph_def::{GraphDef, NodeTransfer};
use crate::model::predecessor::EventType;

/// Status of a single node during graph execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeStatus {
    /// Node has not started execution yet.
    Pending,
    /// Node is currently executing.
    Running,
    /// Node completed successfully.
    Completed,
    /// Node failed during execution.
    Failed,
    /// Node timed out.
    TimedOut,
    /// Node has been skipped (bypassed due to edge conditions).
    Skipped,
}

/// The result of executing a single node.
#[derive(Debug, Clone)]
pub enum NodeResult {
    /// Node has not produced a result yet.
    None,
    /// Node completed successfully.
    Completed,
    /// Node failed with the given error reason.
    Failed(String),
    /// Node timed out.
    TimedOut,
}

impl NodeResult {
    /// Returns `true` if the node is in a terminal state.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed(_) | Self::TimedOut)
    }
}

/// Runtime state for a single node.
#[derive(Debug, Clone)]
pub struct NodeState {
    /// Current status of the node.
    pub status: NodeStatus,
    /// The result produced by the node (if any).
    pub result: NodeResult,
}

impl Default for NodeState {
    fn default() -> Self {
        Self {
            status: NodeStatus::Pending,
            result: NodeResult::None,
        }
    }
}

/// Per-event-type counters for a node.
#[derive(Debug, Clone, Default)]
pub struct NodeCounters {
    /// Number of times the node completed successfully.
    pub complete: u64,
    /// Number of times the node failed.
    pub failed: u64,
    /// Number of times the node timed out.
    pub timeout: u64,
}

/// Aggregate runtime state for the entire graph execution.
///
/// All fields are `pub(crate)` — visible within the crate for diagnostics
/// and testing, but not to external consumers (nexus-cli, nexus-mcp-server).
/// Use the accessor methods on [`Scheduler`] instead.
#[derive(Debug, Clone)]
pub struct RuntimeState {
    /// Per-node execution states.
    pub(crate) states: HashMap<NodeIndex, NodeState>,
    /// Per-node event and execution counters.
    pub(crate) counters: HashMap<NodeIndex, NodeCounters>,
    /// Per-node retry attempt counts.
    pub(crate) retry_counts: HashMap<NodeIndex, u64>,
    /// Per-edge runtime state (indexed parallel to [`GraphDef::edges`]).
    pub(crate) edge_states: Vec<EdgeState>,
    /// Queue of nodes ready to execute.
    pub(crate) ready_queue: VecDeque<NodeIndex>,
    /// Tracks how many All-strategy incoming edges have not yet fired for each
    /// downstream node. When this reaches 0, the node is ready to enqueue.
    pub(crate) fan_in_pending: HashMap<NodeIndex, usize>,
}

/// The graph scheduler that drives node execution via event handling.
///
/// The scheduler implements the NODE_TRANSFER model:
/// - Nodes emit events (complete, failed, timeout)
/// - Events propagate through edges based on strategy (All/Any), threshold,
///   exit_reason filters, and event type matching
/// - When an edge fires, the target node is enqueued for execution
#[derive(Debug, Clone)]
pub struct Scheduler {
    /// The static graph definition (immutable during execution).
    graph: GraphDef,
    /// Mutable runtime state.
    state: RuntimeState,
}

impl Scheduler {
    /// Create a new `Scheduler` from a [`GraphDef`].
    ///
    /// Initialises runtime state with:
    /// - All nodes in `Pending` state with `None` result
    /// - Zero counters and retry counts
    /// - Edge states matching the number of edges in the graph
    /// - Empty ready queue
    #[must_use]
    pub fn new(graph: GraphDef) -> Self {
        let mut states: HashMap<NodeIndex, NodeState> = HashMap::new();
        let mut counters: HashMap<NodeIndex, NodeCounters> = HashMap::new();
        let mut retry_counts: HashMap<NodeIndex, u64> = HashMap::new();

        for idx in graph.node_indices() {
            states.insert(idx, NodeState::default());
            counters.insert(idx, NodeCounters::default());
            retry_counts.insert(idx, 0);
        }

        let edge_states: Vec<EdgeState> = (0..graph.edge_count())
            .map(|_| EdgeState::default())
            .collect();

        // Build fan_in_pending: for each node, count how many incoming All-strategy
        // edges point to it. A node becomes ready (enqueued) only when all its
        // All-strategy incoming edges have fired (pending drops to 0).
        let mut fan_in_pending: HashMap<NodeIndex, usize> = HashMap::new();
        for edge in graph.edges() {
            if edge.strategy == Strategy::All {
                *fan_in_pending.entry(edge.to).or_insert(0) += 1;
            }
        }

        Self {
            graph,
            state: RuntimeState {
                states,
                counters,
                retry_counts,
                edge_states,
                ready_queue: VecDeque::new(),
                fan_in_pending,
            },
        }
    }

    /// Handle an event emitted by a node.
    ///
    /// Processes the event through all outgoing edges of the given node.
    /// When an edge's conditions are satisfied (strategy, threshold,
    /// exit_reason), the edge fires and the target node is returned in the
    /// result vector.
    ///
    /// # Algorithm
    ///
    /// For each outgoing edge of `node`:
    /// 1. Skip if the edge has already triggered
    /// 2. Skip if the edge's event type does not match the incoming event
    /// 3. Skip if the edge has an exit_reason filter that does not match
    /// 4. Increment the edge's event counter
    /// 5. If the counter meets or exceeds the threshold and the edge hasn't
    ///    fired: mark as triggered and apply the strategy:
    ///    - `Strategy::Any`: immediately enqueue the target node
    ///    - `Strategy::All`: decrement `fan_in_pending[target]`; enqueue only
    ///      when `fan_in_pending[target]` reaches 0 (all incoming All-edges
    ///      have fired)
    ///
    /// # Returns
    ///
    /// A vector of [`NodeIndex`] values for nodes that should be enqueued as a
    /// result of this event.
    pub fn handle_event(
        &mut self,
        node: NodeIndex,
        event: EventType,
        exit_reason: Option<&str>,
    ) -> Vec<NodeIndex> {
        let mut ready: Vec<NodeIndex> = Vec::new();

        // Update node state based on event type.
        if let Some(ns) = self.state.states.get_mut(&node) {
            match event {
                EventType::Complete => {
                    ns.status = NodeStatus::Completed;
                    if matches!(ns.result, NodeResult::None) {
                        ns.result = NodeResult::Completed;
                    }
                }
                EventType::Failed => {
                    ns.status = NodeStatus::Failed;
                    ns.result = NodeResult::Failed(
                        exit_reason.unwrap_or("failed").to_string(),
                    );
                }
                EventType::Timeout => {
                    ns.status = NodeStatus::TimedOut;
                    ns.result = NodeResult::TimedOut;
                }
            }
        }

        // Increment per-event-type counter.
        if let Some(counter) = self.state.counters.get_mut(&node) {
            match event {
                EventType::Complete => counter.complete += 1,
                EventType::Failed => counter.failed += 1,
                EventType::Timeout => counter.timeout += 1,
            }
        }

        // Look up the transfer for this node.
        let transfer: Option<&NodeTransfer> = self.graph.transfers().get(&node);
        let transfer = match transfer {
            Some(t) => t,
            None => return ready,
        };

        let edges: &[EdgeDef] = self.graph.edges();

        for &edge_idx in &transfer.out_edge_indices {
            let edge: &EdgeDef = &edges[edge_idx];
            let es: &mut EdgeState = &mut self.state.edge_states[edge_idx];

            // 1. Already triggered — skip.
            if es.triggered {
                continue;
            }

            // 2. Event type mismatch — skip.
            if edge.event_type != event {
                continue;
            }

            // 3. Exit reason filter mismatch — skip.
            if let Some(ref expected_reason) = edge.exit_reason {
                match exit_reason {
                    Some(actual) if actual == expected_reason.as_str() => {}
                    _ => continue,
                }
            }

            // 4. Increment counter.
            es.event_count += 1;

            // 5. Check threshold and trigger.
            if es.event_count >= edge.threshold {
                es.triggered = true;

                match edge.strategy {
                    Strategy::Any => {
                        // Any: immediately enqueue the downstream node.
                        self.state.ready_queue.push_back(edge.to);
                        ready.push(edge.to);
                    }
                    Strategy::All => {
                        // All: decrement the fan_in_pending counter for the target.
                        if let Some(pending) = self.state.fan_in_pending.get_mut(&edge.to) {
                            if *pending > 0 {
                                *pending -= 1;
                            }
                        }
                        // Only enqueue when ALL All-strategy incoming edges have fired.
                        let all_ready = self.state.fan_in_pending
                            .get(&edge.to)
                            .map(|&p| p == 0)
                            .unwrap_or(true);
                        if all_ready {
                            self.state.ready_queue.push_back(edge.to);
                            ready.push(edge.to);
                        }
                        // pending > 0 → still waiting for other incoming edges.
                    }
                }
            }
        }

        ready
    }

    /// Dequeue the next node ready for execution.
    ///
    /// Returns `None` if the ready queue is empty.
    #[must_use]
    pub fn dequeue(&mut self) -> Option<NodeIndex> {
        self.state.ready_queue.pop_front()
    }

    /// Drain all currently ready nodes from the queue.
    ///
    /// Used by [`Engine`] to collect all nodes that became ready after a single
    /// event was processed, since [`handle_event`] now pushes directly to the
    /// scheduler's ready queue instead of returning them in a `Vec`.
    #[must_use]
    pub fn dequeue_all(&mut self) -> Vec<NodeIndex> {
        self.state.ready_queue.drain(..).collect()
    }

    /// Returns `true` if the ready queue is non-empty.
    #[must_use]
    pub fn has_ready(&self) -> bool {
        !self.state.ready_queue.is_empty()
    }

    /// Enqueue a node for execution.
    pub fn enqueue(&mut self, node: NodeIndex) {
        self.state.ready_queue.push_back(node);
    }

    /// Enqueue all entry nodes.
    ///
    /// Call this at the start of graph execution to seed the ready queue.
    pub fn enqueue_entries(&mut self) {
        let entries: Vec<NodeIndex> = self.graph.entry_nodes().to_vec();
        for &entry in &entries {
            self.enqueue(entry);
        }
    }

    /// Attempt to retry a failed node.
    ///
    /// Resets the node's state to `Pending` and `NodeResult::None`, resets
    /// the outgoing edge states so they can fire again, and enqueues the
    /// node for re-execution. Returns `false` if the retry limit has been
    /// reached.
    pub fn retry_node(&mut self, node: NodeIndex, max_retries: u64) -> bool {
        let count = self.state.retry_counts.get_mut(&node);
        let count = match count {
            Some(c) => c,
            None => return false,
        };

        if *count >= max_retries {
            return false;
        }

        *count += 1;

        // Reset node state.
        if let Some(ns) = self.state.states.get_mut(&node) {
            ns.status = NodeStatus::Pending;
            ns.result = NodeResult::None;
        }

        // Reset outgoing edge states so they can fire again on re-execution.
        if let Some(transfer) = self.graph.transfers().get(&node) {
            for &edge_idx in &transfer.out_edge_indices {
                if let Some(es) = self.state.edge_states.get_mut(edge_idx) {
                    es.triggered = false;
                    es.event_count = 0;
                }
                // Restore fan_in_pending for All-strategy edges — the downstream
                // node is now waiting for this edge to fire again.
                if let Some(edge) = self.graph.edges().get(edge_idx) {
                    if edge.strategy == Strategy::All {
                        if let Some(pending) = self.state.fan_in_pending.get_mut(&edge.to) {
                            *pending += 1;
                        }
                    }
                }
            }
        }

        true
    }

    /// Check whether the graph execution has converged.
    ///
    /// Convergence means all nodes are in a terminal state (Completed, Failed,
    /// TimedOut, or Skipped) and the ready queue is empty.
    #[must_use]
    pub fn is_converged(&self) -> bool {
        if !self.state.ready_queue.is_empty() {
            return false;
        }

        self.state.states.values().all(|s| match s.status {
            NodeStatus::Completed
            | NodeStatus::Failed
            | NodeStatus::TimedOut
            | NodeStatus::Skipped => true,
            NodeStatus::Pending | NodeStatus::Running => false,
        })
    }

    /// Get a reference to the graph definition.
    #[must_use]
    pub fn graph(&self) -> &GraphDef {
        &self.graph
    }

    /// Get a reference to the runtime state.
    #[must_use]
    pub fn state(&self) -> &RuntimeState {
        &self.state
    }

    /// Get a mutable reference to the runtime state.
    pub fn state_mut(&mut self) -> &mut RuntimeState {
        &mut self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::edge::Strategy;
    use crate::graph::graph_def::{NodeData, NodeParams, NodeTransfer};
    use petgraph::stable_graph::StableDiGraph;

    /// Build a simple 2-node A → B chain graph for testing.
    fn build_chain_graph() -> GraphDef {
        let mut graph = StableDiGraph::new();
        let a = graph.add_node(NodeData {
            id: "A".into(),
            providers: vec![],
            process_timeout_secs: 10,
        });
        let b = graph.add_node(NodeData {
            id: "B".into(),
            providers: vec![],
            process_timeout_secs: 10,
        });

        let mut index = HashMap::new();
        index.insert("A".into(), a);
        index.insert("B".into(), b);

        let edges = vec![EdgeDef {
            from: a,
            to: b,
            event_type: EventType::Complete,
            exit_reason: None,
            threshold: 1,
            strategy: Strategy::All,
        }];
        let mut transfers = HashMap::new();
        transfers.insert(
            a,
            NodeTransfer {
                from: a,
                out_edge_indices: vec![0],
            },
        );
        transfers.insert(
            b,
            NodeTransfer {
                from: b,
                out_edge_indices: vec![],
            },
        );

        let params = HashMap::from([
            (a, NodeParams {
                process_timeout_secs: 10,
            }),
            (b, NodeParams {
                process_timeout_secs: 10,
            }),
        ]);
        let entries = vec![a];

        GraphDef::from_components(graph, index, edges, transfers, params, entries)
            .expect("test chain graph should be valid")
    }

    /// Build a fan-in graph: A → C, B → C.
    fn build_fan_in_graph() -> GraphDef {
        let mut graph = StableDiGraph::new();
        let a = graph.add_node(NodeData {
            id: "A".into(),
            providers: vec![],
            process_timeout_secs: 10,
        });
        let b = graph.add_node(NodeData {
            id: "B".into(),
            providers: vec![],
            process_timeout_secs: 10,
        });
        let c = graph.add_node(NodeData {
            id: "C".into(),
            providers: vec![],
            process_timeout_secs: 10,
        });

        let mut index = HashMap::new();
        index.insert("A".into(), a);
        index.insert("B".into(), b);
        index.insert("C".into(), c);

        let edges = vec![
            EdgeDef {
                from: a, to: c,
                event_type: EventType::Complete, exit_reason: None, threshold: 1,
                strategy: Strategy::All,
            },
            EdgeDef {
                from: b, to: c,
                event_type: EventType::Complete, exit_reason: None, threshold: 1,
                strategy: Strategy::All,
            },
        ];

        let mut transfers = HashMap::new();
        transfers.insert(
            a,
            NodeTransfer {
                from: a,
                out_edge_indices: vec![0],
            },
        );
        transfers.insert(
            b,
            NodeTransfer {
                from: b,
                out_edge_indices: vec![1],
            },
        );
        transfers.insert(
            c,
            NodeTransfer {
                from: c,
                out_edge_indices: vec![],
            },
        );

        let params = HashMap::from([
            (a, NodeParams {
                process_timeout_secs: 10,
            }),
            (b, NodeParams {
                process_timeout_secs: 10,
            }),
            (c, NodeParams {
                process_timeout_secs: 10,
            }),
        ]);
        let entries = vec![a, b];

        GraphDef::from_components(graph, index, edges, transfers, params, entries)
            .expect("test fan-in graph should be valid")
    }

    /// Build a fan-out graph: A → B, A → C.
    fn build_fan_out_graph() -> GraphDef {
        let mut graph = StableDiGraph::new();
        let a = graph.add_node(NodeData {
            id: "A".into(),
            providers: vec![],
            process_timeout_secs: 10,
        });
        let b = graph.add_node(NodeData {
            id: "B".into(),
            providers: vec![],
            process_timeout_secs: 10,
        });
        let c = graph.add_node(NodeData {
            id: "C".into(),
            providers: vec![],
            process_timeout_secs: 10,
        });

        let mut index = HashMap::new();
        index.insert("A".into(), a);
        index.insert("B".into(), b);
        index.insert("C".into(), c);

        let edges = vec![
            EdgeDef {
                from: a,
                to: b,
                event_type: EventType::Complete,
                exit_reason: None,
                threshold: 1,
                strategy: Strategy::All,
            },
            EdgeDef {
                from: a,
                to: c,
                event_type: EventType::Complete,
                exit_reason: None,
                threshold: 1,
                strategy: Strategy::All,
            },
        ];

        let mut transfers = HashMap::new();
        transfers.insert(
            a,
            NodeTransfer {
                from: a,
                out_edge_indices: vec![0, 1],
            },
        );
        transfers.insert(
            b,
            NodeTransfer {
                from: b,
                out_edge_indices: vec![],
            },
        );
        transfers.insert(
            c,
            NodeTransfer {
                from: c,
                out_edge_indices: vec![],
            },
        );

        let params = HashMap::from([
            (a, NodeParams {
                process_timeout_secs: 10,
            }),
            (b, NodeParams {
                process_timeout_secs: 10,
            }),
            (c, NodeParams {
                process_timeout_secs: 10,
            }),
        ]);
        let entries = vec![a];

        GraphDef::from_components(graph, index, edges, transfers, params, entries)
            .expect("test fan-out graph should be valid")
    }

    // ── 1. Simple chain ───────────────────────────────────

    #[test]
    fn test_handle_event_simple_chain() {
        let mut scheduler = Scheduler::new(build_chain_graph());
        let a_idx = scheduler.graph().node_index("A").expect("A should exist");

        let ready = scheduler.handle_event(a_idx, EventType::Complete, None);
        assert_eq!(ready.len(), 1, "A completing should trigger edge to B");

        let b_idx = scheduler.graph().node_index("B").expect("B should exist");
        assert_eq!(
            ready[0], b_idx,
            "edge should target B"
        );

        // After triggering, edge should be marked triggered.
        assert!(scheduler.state.edge_states[0].triggered);
    }

    // ── 2. Event type mismatch ────────────────────────────

    #[test]
    fn test_event_type_mismatch() {
        let mut scheduler = Scheduler::new(build_chain_graph());
        let a_idx = scheduler.graph().node_index("A").expect("A should exist");

        // Edge expects Complete, but we send Failed.
        let ready = scheduler.handle_event(a_idx, EventType::Failed, None);
        assert!(
            ready.is_empty(),
            "Failed event should not trigger a Complete edge"
        );
    }

    // ── 3. Strategy: Any (threshold = 1) ──────────────────

    #[test]
    fn test_strategy_any() {
        // Build graph: A → C (Any), B → C (Any), threshold=1
        let mut graph = StableDiGraph::new();
        let a = graph.add_node(NodeData {
            id: "A".into(),
            providers: vec![],
            process_timeout_secs: 10,
        });
        let b = graph.add_node(NodeData {
            id: "B".into(),
            providers: vec![],
            process_timeout_secs: 10,
        });
        let c = graph.add_node(NodeData {
            id: "C".into(),
            providers: vec![],
            process_timeout_secs: 10,
        });

        let mut index = HashMap::new();
        index.insert("A".into(), a);
        index.insert("B".into(), b);
        index.insert("C".into(), c);

        let edges = vec![
            EdgeDef {
                from: a, to: c,
                event_type: EventType::Complete, exit_reason: None, threshold: 1,
                strategy: Strategy::Any,
            },
            EdgeDef {
                from: b, to: c,
                event_type: EventType::Complete, exit_reason: None, threshold: 1,
                strategy: Strategy::Any,
            },
        ];

        let mut transfers = HashMap::new();
        transfers.insert(
            a,
            NodeTransfer {
                from: a,
                out_edge_indices: vec![0],
            },
        );
        transfers.insert(
            b,
            NodeTransfer {
                from: b,
                out_edge_indices: vec![1],
            },
        );
        transfers.insert(
            c,
            NodeTransfer {
                from: c,
                out_edge_indices: vec![],
            },
        );

        let params = HashMap::from([
            (a, NodeParams {
                process_timeout_secs: 10,
            }),
            (b, NodeParams {
                process_timeout_secs: 10,
            }),
            (c, NodeParams {
                process_timeout_secs: 10,
            }),
        ]);
        let entries = vec![a, b];

        let graph_def = GraphDef::from_components(
            graph, index, edges, transfers, params, entries,
        )
        .expect("graph should be valid");

        let mut scheduler = Scheduler::new(graph_def);

        // A completes → edge 0 (A→C) fires (Any, threshold=1). C is ready.
        let ready = scheduler.handle_event(a, EventType::Complete, None);
        assert_eq!(ready.len(), 1, "A completing should trigger its edge to C");
        assert_eq!(ready[0], c, "edge should target C");

        // B completes → edge 1 (B→C) fires separately (different edge idx).
        let ready2 = scheduler.handle_event(b, EventType::Complete, None);
        assert_eq!(ready2.len(), 1, "B's separate edge should also fire");
        assert_eq!(ready2[0], c, "B's edge should also target C");
    }

    // ── 4. Fan-in All: both must complete ──────────────────

    #[test]
    fn test_fan_in_triggers_separate_edges() {
        // build_fan_in_graph uses Strategy::All for both edges, so C should
        // only be ready after BOTH A and B complete.
        let mut scheduler = Scheduler::new(build_fan_in_graph());
        let a_idx = scheduler.graph().node_index("A").expect("A should exist");
        let b_idx = scheduler.graph().node_index("B").expect("B should exist");
        let c_idx = scheduler.graph().node_index("C").expect("C should exist");

        // A completes → edge 0 fires but C is NOT ready (B hasn't completed).
        let ready = scheduler.handle_event(a_idx, EventType::Complete, None);
        assert!(ready.is_empty(), "A alone should not trigger C with All strategy");

        // B completes → now both edges have fired, C should be ready.
        let ready = scheduler.handle_event(b_idx, EventType::Complete, None);
        assert_eq!(ready.len(), 1, "Both A and B complete → C ready");
        assert_eq!(ready[0], c_idx, "C should be the ready node");
    }

    // ── 5. Threshold > 1 ──────────────────────────────────

    #[test]
    fn test_threshold_greater_than_one() {
        // A → B (Any, threshold=3)
        let mut graph = StableDiGraph::new();
        let a = graph.add_node(NodeData {
            id: "A".into(),
            providers: vec![],
            process_timeout_secs: 10,
        });
        let b = graph.add_node(NodeData {
            id: "B".into(),
            providers: vec![],
            process_timeout_secs: 10,
        });

        let mut index = HashMap::new();
        index.insert("A".into(), a);
        index.insert("B".into(), b);

        let edges = vec![EdgeDef {
            from: a,
            to: b,
            event_type: EventType::Complete,
            exit_reason: None,
            threshold: 3,
            strategy: Strategy::Any,
        }];

        let mut transfers = HashMap::new();
        transfers.insert(
            a,
            NodeTransfer {
                from: a,
                out_edge_indices: vec![0],
            },
        );
        transfers.insert(
            b,
            NodeTransfer {
                from: b,
                out_edge_indices: vec![],
            },
        );

        let params = HashMap::from([
            (a, NodeParams {
                process_timeout_secs: 10,
            }),
            (b, NodeParams {
                process_timeout_secs: 10,
            }),
        ]);
        let entries = vec![a];

        let graph_def = GraphDef::from_components(
            graph, index, edges, transfers, params, entries,
        )
        .expect("graph should be valid");

        let mut scheduler = Scheduler::new(graph_def);

        // First two events should not trigger.
        for i in 0..2 {
            let ready = scheduler.handle_event(a, EventType::Complete, None);
            assert!(
                ready.is_empty(),
                "event {} should not trigger threshold=3 edge",
                i + 1
            );
        }

        // Third event should trigger.
        let ready = scheduler.handle_event(a, EventType::Complete, None);
        assert_eq!(ready.len(), 1, "third event should trigger edge");
        assert_eq!(ready[0], b, "edge should target B");
    }

    // ── 6. Exit reason filter ─────────────────────────────

    #[test]
    fn test_exit_reason_filter() {
        // A → B (Complete, exit_reason="ok")
        let mut graph = StableDiGraph::new();
        let a = graph.add_node(NodeData {
            id: "A".into(),
            providers: vec![],
            process_timeout_secs: 10,
        });
        let b = graph.add_node(NodeData {
            id: "B".into(),
            providers: vec![],
            process_timeout_secs: 10,
        });

        let mut index = HashMap::new();
        index.insert("A".into(), a);
        index.insert("B".into(), b);

        let edges = vec![EdgeDef {
            from: a,
            to: b,
            event_type: EventType::Complete,
            exit_reason: Some("ok".into()),
            threshold: 1,
            strategy: Strategy::All,
        }];

        let mut transfers = HashMap::new();
        transfers.insert(
            a,
            NodeTransfer {
                from: a,
                out_edge_indices: vec![0],
            },
        );
        transfers.insert(
            b,
            NodeTransfer {
                from: b,
                out_edge_indices: vec![],
            },
        );

        let params = HashMap::from([
            (a, NodeParams {
                process_timeout_secs: 10,
            }),
            (b, NodeParams {
                process_timeout_secs: 10,
            }),
        ]);
        let entries = vec![a];

        let graph_def = GraphDef::from_components(
            graph, index, edges, transfers, params, entries,
        )
        .expect("graph should be valid");

        let mut scheduler = Scheduler::new(graph_def);

        // Wrong exit reason should not trigger.
        let ready = scheduler.handle_event(a, EventType::Complete, Some("wrong"));
        assert!(
            ready.is_empty(),
            "wrong exit reason should not trigger edge"
        );

        // Correct exit reason should trigger.
        let ready = scheduler.handle_event(a, EventType::Complete, Some("ok"));
        assert_eq!(ready.len(), 1, "correct exit reason should trigger edge");
        assert_eq!(ready[0], b);
    }

    // ── 6b. Exit reason branch routing ─────────────────────

    fn build_exit_reason_branch_graph() -> GraphDef {
        // A → B (Complete, exit_reason="ok"), A → C (Complete, exit_reason="review")
        let mut graph = StableDiGraph::new();
        let a = graph.add_node(NodeData {
            id: "A".into(),
            providers: vec![],
            process_timeout_secs: 10,
        });
        let b = graph.add_node(NodeData {
            id: "B".into(),
            providers: vec![],
            process_timeout_secs: 10,
        });
        let c = graph.add_node(NodeData {
            id: "C".into(),
            providers: vec![],
            process_timeout_secs: 10,
        });

        let mut index = HashMap::new();
        index.insert("A".into(), a);
        index.insert("B".into(), b);
        index.insert("C".into(), c);

        let edges = vec![
            EdgeDef {
                from: a,
                to: b,
                event_type: EventType::Complete,
                exit_reason: Some("ok".into()),
                threshold: 1,
                strategy: Strategy::Any,
            },
            EdgeDef {
                from: a,
                to: c,
                event_type: EventType::Complete,
                exit_reason: Some("review".into()),
                threshold: 1,
                strategy: Strategy::Any,
            },
        ];

        let mut transfers = HashMap::new();
        transfers.insert(
            a,
            NodeTransfer {
                from: a,
                out_edge_indices: vec![0, 1],
            },
        );
        transfers.insert(
            b,
            NodeTransfer {
                from: b,
                out_edge_indices: vec![],
            },
        );
        transfers.insert(
            c,
            NodeTransfer {
                from: c,
                out_edge_indices: vec![],
            },
        );

        let params = HashMap::from([
            (a, NodeParams {
                process_timeout_secs: 10,
            }),
            (b, NodeParams {
                process_timeout_secs: 10,
            }),
            (c, NodeParams {
                process_timeout_secs: 10,
            }),
        ]);
        let entries = vec![a];

        GraphDef::from_components(graph, index, edges, transfers, params, entries)
            .expect("graph should be valid")
    }

    #[test]
    fn test_exit_reason_branch_to_b() {
        // exit_reason "ok" should route to B, not C
        let mut scheduler = Scheduler::new(build_exit_reason_branch_graph());
        let a = scheduler.graph().node_index("A").expect("A exists");
        let b = scheduler.graph().node_index("B").expect("B exists");

        let ready = scheduler.handle_event(a, EventType::Complete, Some("ok"));
        assert_eq!(ready.len(), 1, "exit_reason 'ok' should trigger exactly one downstream");
        assert_eq!(ready[0], b, "'ok' should route to B");
    }

    #[test]
    fn test_exit_reason_branch_to_c() {
        // exit_reason "review" should route to C, not B
        let mut scheduler = Scheduler::new(build_exit_reason_branch_graph());
        let a = scheduler.graph().node_index("A").expect("A exists");
        let c = scheduler.graph().node_index("C").expect("C exists");

        let ready = scheduler.handle_event(a, EventType::Complete, Some("review"));
        assert_eq!(ready.len(), 1, "exit_reason 'review' should trigger exactly one downstream");
        assert_eq!(ready[0], c, "'review' should route to C");
    }

    #[test]
    fn test_exit_reason_branch_no_match() {
        // exit_reason "unknown" should match neither edge → no downstream triggered
        let mut scheduler = Scheduler::new(build_exit_reason_branch_graph());
        let a = scheduler.graph().node_index("A").expect("A exists");

        let ready = scheduler.handle_event(a, EventType::Complete, Some("unknown"));
        assert!(ready.is_empty(), "no edge matches exit_reason 'unknown'");
    }

    // ── 7. Retry node ─────────────────────────────────────

    #[test]
    fn test_retry_node() {
        let mut scheduler = Scheduler::new(build_chain_graph());
        let a_idx = scheduler.graph().node_index("A").expect("A should exist");

        // Initially, retry count is 0.
        assert_eq!(scheduler.state.retry_counts[&a_idx], 0);

        // Retry should succeed (count=0 < 3).
        let could_retry = scheduler.retry_node(a_idx, 3);
        assert!(could_retry, "first retry should succeed");
        assert_eq!(scheduler.state.retry_counts[&a_idx], 1);
        assert_eq!(
            scheduler.state.states[&a_idx].status,
            NodeStatus::Pending,
            "retry should reset status to Pending"
        );

        // Exhaust retries: max_retries=3 means 3 retries total allowed (counts 0,1,2).
        // After 3 successful retries (count becomes 3), next one should fail.
        scheduler.retry_node(a_idx, 3); // count=2
        scheduler.retry_node(a_idx, 3); // count=3 → >= 3 → fail
        let could_retry = scheduler.retry_node(a_idx, 3);
        assert!(
            !could_retry,
            "retry should fail after max_retries=3 retries exhausted"
        );
    }

    // ── 7b. Retry resets edge state ────────────────────────

    #[test]
    fn test_retry_node_resets_edge_state() {
        // A → B. A completes → edge fires. retry_node → edge reset.
        // After reset, handle_event again → B is re-triggered.
        let mut scheduler = Scheduler::new(build_chain_graph());
        let a = scheduler.graph().node_index("A").expect("A exists");
        let b = scheduler.graph().node_index("B").expect("B exists");

        // First completion: edge fires, B is made ready.
        let ready = scheduler.handle_event(a, EventType::Complete, None);
        assert_eq!(ready.len(), 1, "A should trigger B");
        assert_eq!(ready[0], b, "should target B");

        // Verify edge is triggered.
        assert!(
            scheduler.state.edge_states[0].triggered,
            "edge should be triggered after first fire"
        );

        // Retry resets edge state.
        assert!(scheduler.retry_node(a, 3), "retry should succeed");
        assert!(
            !scheduler.state.edge_states[0].triggered,
            "edge triggered should be reset after retry"
        );
        assert_eq!(
            scheduler.state.edge_states[0].event_count,
            0,
            "edge event_count should be cleared after retry"
        );

        // Verify node state was reset.
        assert_eq!(
            scheduler.state.states[&a].status,
            NodeStatus::Pending,
            "retried node should be Pending"
        );

        // Second completion: B is triggered again (edge was reset).
        let ready2 = scheduler.handle_event(a, EventType::Complete, None);
        assert_eq!(
            ready2.len(),
            1,
            "retried A should re-trigger B"
        );
        assert_eq!(
            ready2[0], b,
            "retried A should re-trigger B"
        );
    }

    // ── 8. Triggered edge prevents re-fire ────────────────

    #[test]
    fn test_already_triggered_edge_does_not_fire_again() {
        let mut scheduler = Scheduler::new(build_fan_in_graph());
        let a = scheduler.graph().node_index("A").expect("A should exist");
        let b = scheduler.graph().node_index("B").expect("B should exist");

        // A completes → edge 0 fires but C not ready (All, pending still 1).
        let r1 = scheduler.handle_event(a, EventType::Complete, None);
        assert!(r1.is_empty(), "A alone should not trigger C with All strategy");

        // B completes → edge 1 fires, C is now ready (pending 0).
        let r2 = scheduler.handle_event(b, EventType::Complete, None);
        assert_eq!(r2.len(), 1, "B completes → C ready");

        // B completes again → edge 1 already triggered.
        let r3 = scheduler.handle_event(b, EventType::Complete, None);
        assert!(
            r3.is_empty(),
            "already-triggered edge should not fire again"
        );
    }

    // ── 9. Enqueue / dequeue ──────────────────────────────

    #[test]
    fn test_enqueue_dequeue() {
        let mut scheduler = Scheduler::new(build_chain_graph());
        let a = scheduler.graph().node_index("A").expect("A should exist");
        let b = scheduler.graph().node_index("B").expect("B should exist");

        assert_eq!(scheduler.dequeue(), None, "queue should start empty");

        scheduler.enqueue(a);
        scheduler.enqueue(b);

        assert_eq!(scheduler.dequeue(), Some(a));
        assert_eq!(scheduler.dequeue(), Some(b));
        assert_eq!(scheduler.dequeue(), None, "queue should be empty after dequeue");
    }

    // ── 10. Enqueue entries ───────────────────────────────

    #[test]
    fn test_enqueue_entries() {
        let mut scheduler = Scheduler::new(build_chain_graph());
        scheduler.enqueue_entries();

        let a = scheduler.graph().node_index("A").expect("A should exist");
        assert_eq!(scheduler.dequeue(), Some(a));
        assert_eq!(scheduler.dequeue(), None);
    }

    // ── 11. Convergence ───────────────────────────────────

    #[test]
    fn test_is_converged() {
        let mut scheduler = Scheduler::new(build_chain_graph());
        assert!(
            !scheduler.is_converged(),
            "graph with pending nodes should not be converged"
        );

        let a = scheduler.graph().node_index("A").expect("A should exist");
        let b = scheduler.graph().node_index("B").expect("B should exist");

        // Mark both nodes as completed.
        scheduler.state.states.get_mut(&a).unwrap().status = NodeStatus::Completed;
        scheduler.state.states.get_mut(&b).unwrap().status = NodeStatus::Completed;

        assert!(
            scheduler.is_converged(),
            "graph with all nodes completed should be converged"
        );
    }

    #[test]
    fn test_not_converged_with_ready_queue() {
        let mut scheduler = Scheduler::new(build_chain_graph());
        let a = scheduler.graph().node_index("A").expect("A should exist");
        scheduler.enqueue(a);

        // All nodes are Pending, but queue has items.
        assert!(!scheduler.is_converged());
    }

    // ── 12. NodeResult terminal checks ────────────────────

    #[test]
    fn test_node_result_is_terminal() {
        assert!(!NodeResult::None.is_terminal());
        assert!(NodeResult::Completed.is_terminal());
        assert!(NodeResult::Failed("err".into()).is_terminal());
        assert!(NodeResult::TimedOut.is_terminal());
    }

    // ── 13. Handle event sets node state ──────────────────

    #[test]
    fn test_handle_event_sets_node_state() {
        let mut scheduler = Scheduler::new(build_chain_graph());
        let a = scheduler.graph().node_index("A").expect("A should exist");
        let b = scheduler.graph().node_index("B").expect("B should exist");

        // A fails.
        let _ready = scheduler.handle_event(a, EventType::Failed, Some("crash"));
        assert_eq!(
            scheduler.state.states[&a].status,
            NodeStatus::Failed,
            "A should be marked as Failed"
        );

        // B completes.
        let _ready = scheduler.handle_event(b, EventType::Complete, None);
        assert_eq!(
            scheduler.state.states[&b].status,
            NodeStatus::Completed,
            "B should be marked as Completed"
        );

        // A times out.
        // Reset A first by re-creating... Actually, just check that timeout works.
        // We need a new scheduler because A's state was already set.
    }

    #[test]
    fn test_handle_event_timeout() {
        // A → B, timeout A.
        let mut scheduler = Scheduler::new(build_chain_graph());
        let a = scheduler.graph().node_index("A").expect("A should exist");

        let _ready = scheduler.handle_event(a, EventType::Timeout, None);
        assert_eq!(
            scheduler.state.states[&a].status,
            NodeStatus::TimedOut,
            "A should be marked as TimedOut"
        );
    }

    // ── 14. Empty graph scheduler ─────────────────────────

    #[test]
    fn test_empty_graph_scheduler() {
        let graph = StableDiGraph::new();
        let index = HashMap::new();
        let edges: Vec<EdgeDef> = vec![];
        let transfers: HashMap<NodeIndex, NodeTransfer> = HashMap::new();
        let params: HashMap<NodeIndex, NodeParams> = HashMap::new();
        let entries: Vec<NodeIndex> = vec![];

        let graph_def = GraphDef::from_components(
            graph, index, edges, transfers, params, entries,
        )
        .expect("empty graph should be valid");

        let mut scheduler = Scheduler::new(graph_def);

        assert_eq!(scheduler.dequeue(), None, "empty graph should have no ready nodes");
        assert!(
            scheduler.is_converged(),
            "empty graph should be converged"
        );
        scheduler.enqueue_entries(); // no-op, no crash
    }

    // ── 15. Fan-in All: waits for all upstreams ───────────

    #[test]
    fn test_fan_in_all_waits_for_all_upstreams() {
        // A → C (All, Complete), B → C (All, Complete)
        // True All: C should NOT be ready until both A and B have completed.
        let mut scheduler = Scheduler::new(build_fan_in_graph());
        let a = scheduler.graph().node_index("A").expect("A exists");
        let b = scheduler.graph().node_index("B").expect("B exists");
        let c = scheduler.graph().node_index("C").expect("C exists");

        // A completes — C should NOT be ready yet (B hasn't completed).
        let ready = scheduler.handle_event(a, EventType::Complete, None);
        assert!(ready.is_empty(), "All: A alone should NOT trigger C");
        assert!(!scheduler.has_ready(), "no nodes should be ready");

        // B also completes — now C should be ready.
        let ready = scheduler.handle_event(b, EventType::Complete, None);
        assert_eq!(ready.len(), 1, "All: both A and B complete → C ready");
        assert_eq!(ready[0], c, "C should be the ready node");

        // Verify fan_in_pending is now 0 for C.
        let pending = scheduler.state.fan_in_pending.get(&c).copied().unwrap_or(0);
        assert_eq!(pending, 0, "fan_in_pending for C should be 0 after both fire");
    }

    #[test]
    fn test_fan_in_all_only_triggers_once() {
        // Same graph: A → C (All), B → C (All). After both fire, C is ready.
        // Further events on A or B should not re-enqueue C.
        let mut scheduler = Scheduler::new(build_fan_in_graph());
        let a = scheduler.graph().node_index("A").expect("A exists");
        let b = scheduler.graph().node_index("B").expect("B exists");

        // Both complete → C ready.
        scheduler.handle_event(a, EventType::Complete, None);
        scheduler.handle_event(b, EventType::Complete, None);
        assert_eq!(scheduler.dequeue_all().len(), 1, "C should be ready once");

        // More events on A/B — edges already triggered, no-op.
        scheduler.handle_event(a, EventType::Complete, None);
        scheduler.handle_event(b, EventType::Complete, None);
        assert!(!scheduler.has_ready(), "no more ready nodes after all edges spent");
    }

    // ── 16. Fan-out: one event triggers multiple edges ────

    #[test]
    fn test_fan_out_triggers_multiple_edges() {
        let mut scheduler = Scheduler::new(build_fan_out_graph());
        let a = scheduler.graph().node_index("A").expect("A should exist");

        let ready = scheduler.handle_event(a, EventType::Complete, None);
        assert_eq!(ready.len(), 2, "fan-out A should trigger both B and C");
    }
}
