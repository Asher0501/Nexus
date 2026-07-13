//! Core graph definition type [`GraphDef`] and its component types.
//!
//! `GraphDef` is the verified, aggregated graph definition that all downstream
//! work (scheduling, routing, execution) derives from. It is constructed via
//! [`GraphDef::from_components`] which verifies five structural invariants
//! before returning an instance.
//!
//! [`NodeTransfer`] implements the local closure theorem's f_v: State_v → 2^V
//! through its [`NodeTransfer::evaluate`] method — making the transfer function
//! a named, callable entity instead of inline logic in [`Scheduler`].

use std::collections::{HashMap, VecDeque};

use petgraph::graph::NodeIndex;
use petgraph::stable_graph::StableDiGraph;

use crate::graph::edge::{EdgeDef, EdgeState, Strategy};
use crate::model::error::ValidationError;
use crate::model::predecessor::EventType;
use crate::model::provider::ProviderDef;
use crate::model::workflow::RoutePolicyDef;

/// Data associated with each node in the graph.
#[derive(Debug, Clone)]
pub struct NodeData {
    /// Node identifier from the workflow definition.
    pub id: String,
    /// How this node is executed.
    pub providers: Vec<ProviderDef>,
    /// Maximum execution time in seconds.
    pub process_timeout_secs: u64,
    /// Route policy for this node (None = no override).
    pub route_policy: Option<RoutePolicyDef>,
    /// Per-node max retries on failure (None = inherit global default).
    pub max_retries: Option<u64>,
}

/// Execution parameters for a node.
#[derive(Debug, Clone)]
pub struct NodeParams {
    /// Maximum execution time in seconds.
    pub process_timeout_secs: u64,
}

/// Local transfer function f_v for a single node.
///
/// Corresponds to the local closure theorem's f_v: State_v → 2^V.
/// Each node has exactly one `NodeTransfer`, aggregating all outgoing edges.
///
/// The [`evaluate`](NodeTransfer::evaluate) method makes the transfer function
/// a named, callable entity — instead of inline logic in [`Scheduler`].
#[derive(Debug, Clone)]
pub struct NodeTransfer {
    /// The node this transfer belongs to.
    pub from: NodeIndex,
    /// Indices into [`GraphDef::edges`] for this node's outgoing edges.
    pub out_edge_indices: Vec<usize>,
}

impl NodeTransfer {
    /// Evaluate f_v: State_v → 2^V.
    ///
    /// Implements the local closure theorem's transfer function for this node.
    /// For each outgoing edge e = (v, w, h_e, g_e) where v is `self.from`:
    ///
    /// 1. **h_e** — branch matching: event type match, exit_reason filter, threshold counter
    /// 2. **g_e** — strategy aggregation: `Any` enqueues immediately; `All` waits for
    ///    all upstream nodes via `fan_in_pending`, then resets for the next round
    ///
    /// This is a **pure function** with respect to node state — it reads `edges` and
    /// `edge_states`, mutates `edge_states` (threshold counters), `fan_in_pending`,
    /// and `ready_queue`, and returns the set of downstream nodes to enqueue.
    /// It does NOT modify the calling node's status, result, or event counters.
    #[must_use]
    pub fn evaluate(
        &self,
        edges: &[EdgeDef],
        edge_states: &mut [EdgeState],
        event: EventType,
        exit_reason: Option<&str>,
        fan_in_pending: &mut HashMap<NodeIndex, usize>,
        ready_queue: &mut VecDeque<NodeIndex>,
    ) -> Vec<NodeIndex> {
        let mut ready: Vec<NodeIndex> = Vec::new();

        for &edge_idx in &self.out_edge_indices {
            let edge: &EdgeDef = &edges[edge_idx];
            let es: &mut EdgeState = &mut edge_states[edge_idx];

            // ════════════════════════════════════════════
            // h_e: 分支匹配函数
            // 纯函数判定——不记忆、无 triggered
            // ════════════════════════════════════════════

            // (a) 事件类型匹配
            if edge.event_type != event {
                continue;
            }

            // (b) exit_reason 匹配
            if let Some(ref expected_reason) = edge.exit_reason {
                match exit_reason {
                    Some(actual) if actual == expected_reason.as_str() => {}
                    _ => continue,
                }
            }

            // (c) 阈值计数器递增 + 判定
            es.event_count += 1;
            if es.event_count < edge.threshold {
                continue;
            }

            // ════════════════════════════════════════════
            // g_e: 策略聚合函数
            // Any → 直接入队; All → fan_in_pending 归零后入队
            // ════════════════════════════════════════════

            match edge.strategy {
                Strategy::Any => {
                    ready_queue.push_back(edge.to);
                    ready.push(edge.to);
                }
                Strategy::All => {
                    if let Some(pending) = fan_in_pending.get_mut(&edge.to) {
                        if *pending > 0 {
                            *pending -= 1;
                        }
                    }
                    let all_ready = fan_in_pending
                        .get(&edge.to)
                        .map(|&p| p == 0)
                        .unwrap_or(true);
                    if all_ready {
                        ready_queue.push_back(edge.to);
                        ready.push(edge.to);
                        // 重置 fan_in_pending—下一轮的 g_e 重新计数
                        if let Some(pending) = fan_in_pending.get_mut(&edge.to) {
                            let all_count = edges
                                .iter()
                                .filter(|e| e.to == edge.to && e.strategy == Strategy::All)
                                .count();
                            *pending = all_count;
                        }
                    }
                }
            }
        }

        ready
    }
}

/// A verified, aggregated graph definition.
///
/// Constructed by [`GraphDef::from_components`] and verified by
/// [`GraphDef::invariants_hold`]. All fields are private; access is through
/// safe methods only.
#[derive(Debug, Clone)]
pub struct GraphDef {
    /// The underlying petgraph directed graph (node indices are stable).
    graph: StableDiGraph<NodeData, ()>,
    /// Maps node IDs to their graph indices.
    index: HashMap<String, NodeIndex>,
    /// All edge definitions in the graph.
    edges: Vec<EdgeDef>,
    /// Local transfer functions for each node (f_v).
    transfers: HashMap<NodeIndex, NodeTransfer>,
    /// Execution parameters for each node.
    params: HashMap<NodeIndex, NodeParams>,
    /// Entry nodes (nodes with no predecessors).
    entries: Vec<NodeIndex>,
}

impl GraphDef {
    /// Construct a `GraphDef` from pre-built components.
    ///
    /// This is the ONLY way to create a `GraphDef`. All invariants are checked
    /// before returning. Returns [`ValidationError`] if invariants fail.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError::BuildInvariant` when any invariant check fails.
    pub fn from_components(
        graph: StableDiGraph<NodeData, ()>,
        index: HashMap<String, NodeIndex>,
        edges: Vec<EdgeDef>,
        transfers: HashMap<NodeIndex, NodeTransfer>,
        params: HashMap<NodeIndex, NodeParams>,
        entries: Vec<NodeIndex>,
    ) -> Result<Self, ValidationError> {
        let def = Self {
            graph,
            index,
            edges,
            transfers,
            params,
            entries,
        };
        if let Err(detail) = def.check_invariants() {
            return Err(ValidationError::BuildInvariant {
                description: detail.into(),
            });
        }
        Ok(def)
    }

    /// Verify all five invariants hold.
    ///
    /// 1. All entries have valid [`NodeIndex`] values in the graph
    /// 2. All edges' `from` / `to` reference valid [`NodeIndex`] values
    /// 3. `params` covers every node (same count as node count)
    /// 4. `transfers` covers every node
    /// 5. Every edge is referenced by at least one transfer
    ///
    /// # Errors
    ///
    /// Returns the name of the first violated invariant for diagnostics.
    #[must_use]
    pub fn invariants_hold(&self) -> bool {
        self.check_invariants().is_ok()
    }

    /// Run all five invariant checks and return a descriptive error message
    /// on the first violation, or `Ok(())` if all pass.
    fn check_invariants(&self) -> Result<(), &'static str> {
        let n = self.graph.node_count();

        // 1. All entries have valid indices
        for &entry in &self.entries {
            if entry.index() >= n {
                return Err("invariant 1: entry node index out of bounds");
            }
        }

        // 2. All edge indices are valid
        for edge in &self.edges {
            if edge.from.index() >= n {
                return Err("invariant 2: edge from index out of bounds");
            }
            if edge.to.index() >= n {
                return Err("invariant 2: edge to index out of bounds");
            }
        }

        // 3. params covers every node
        if self.params.len() != n {
            return Err("invariant 3: params count does not match node count");
        }
        for i in 0..n {
            let idx = NodeIndex::new(i);
            if !self.params.contains_key(&idx) {
                return Err("invariant 3: missing params for node");
            }
        }

        // 4. transfers covers every node
        if self.transfers.len() != n {
            return Err("invariant 4: transfers count does not match node count");
        }
        for i in 0..n {
            let idx = NodeIndex::new(i);
            if !self.transfers.contains_key(&idx) {
                return Err("invariant 4: missing transfer for node");
            }
        }

        // 5. Every edge appears in at least one transfer
        let edge_count = self.edges.len();
        for edge_idx in 0..edge_count {
            let found = self
                .transfers
                .values()
                .any(|t| t.out_edge_indices.contains(&edge_idx));
            if !found {
                return Err("invariant 5: edge not referenced by any transfer");
            }
        }

        Ok(())
    }

    // ── Accessors ──────────────────────────────────────────

    /// Get the number of nodes in the graph.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Get the number of edges in the graph.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Get the graph's node weight by index.
    #[must_use]
    pub fn node_weight(&self, idx: NodeIndex) -> Option<&NodeData> {
        self.graph.node_weight(idx)
    }

    /// Look up a node index by its string ID.
    #[must_use]
    pub fn node_index(&self, id: &str) -> Option<NodeIndex> {
        self.index.get(id).copied()
    }

    /// Get all edge definitions.
    #[must_use]
    pub fn edges(&self) -> &[EdgeDef] {
        &self.edges
    }

    /// Get all node transfers (f_v).
    #[must_use]
    pub fn transfers(&self) -> &HashMap<NodeIndex, NodeTransfer> {
        &self.transfers
    }

    /// Get parameters for a node.
    #[must_use]
    pub fn node_params(&self, idx: NodeIndex) -> Option<&NodeParams> {
        self.params.get(&idx)
    }

    /// Get all entry node indices.
    #[must_use]
    pub fn entry_nodes(&self) -> &[NodeIndex] {
        &self.entries
    }

    /// Iterate over all node indices in the graph.
    #[must_use]
    pub fn node_indices(&self) -> impl Iterator<Item = NodeIndex> + '_ {
        (0..self.graph.node_count()).map(NodeIndex::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};

    use crate::graph::edge::Strategy;
    use crate::model::predecessor::EventType;
    use petgraph::graph::NodeIndex;
    use petgraph::stable_graph::StableDiGraph;

    fn make_valid_graph_data() -> (
        StableDiGraph<NodeData, ()>,
        HashMap<String, NodeIndex>,
        Vec<EdgeDef>,
        HashMap<NodeIndex, NodeTransfer>,
        HashMap<NodeIndex, NodeParams>,
        Vec<NodeIndex>,
    ) {
        let mut graph = StableDiGraph::new();
        let a = graph.add_node(NodeData {
            id: "A".into(),
            providers: vec![],
            process_timeout_secs: 10,
            route_policy: None, max_retries: None,
        });
        let b = graph.add_node(NodeData {
            id: "B".into(),
            providers: vec![],
            process_timeout_secs: 10,
            route_policy: None, max_retries: None,
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

        (graph, index, edges, transfers, params, entries)
    }

    #[test]
    fn test_valid_graph_passes_invariants() {
        let (g, i, e, t, p, en) = make_valid_graph_data();
        let def = GraphDef::from_components(g, i, e, t, p, en)
            .expect("valid graph should pass invariants");
        assert!(def.invariants_hold());
        assert_eq!(def.node_count(), 2);
    }

    #[test]
    fn test_invariant_1_invalid_entry_index() {
        let (mut graph, i, e, t, p, _) = make_valid_graph_data();
        // Add a third node so we have 3 nodes total (indices 0, 1, 2)
        let node_c = graph.add_node(NodeData {
            id: "C".into(),
            providers: vec![],
            process_timeout_secs: 10,
            route_policy: None, max_retries: None,
        });
        let mut p2 = p.clone();
        p2.insert(node_c, NodeParams {
            process_timeout_secs: 10,
        });
        let mut t2 = t.clone();
        t2.insert(
            node_c,
            NodeTransfer {
                from: node_c,
                out_edge_indices: vec![],
            },
        );

        // Valid entries = [node_c] (index 2, which exists)
        let valid_entries = vec![node_c];
        let def = GraphDef::from_components(
            graph.clone(),
            i.clone(),
            e.clone(),
            t2,
            p2,
            valid_entries,
        )
        .expect("valid graph with 3 nodes should pass");

        assert!(def.invariants_hold());

        // Now test with invalid entry (index 999 which does not exist)
        let invalid_entries = vec![NodeIndex::new(999)];
        let result = GraphDef::from_components(
            graph,
            i,
            e,
            t,
            p,
            invalid_entries,
        );
        assert!(
            result.is_err(),
            "invalid entry index should fail invariants"
        );
    }

    #[test]
    fn test_invariant_3_params_missing_node() {
        let (g, i, e, t, p, en) = make_valid_graph_data();
        // Remove params for the entry node (index `en[0]`)
        let mut missing_params = p;
        missing_params.remove(&en[0]);
        let result = GraphDef::from_components(g, i, e, t, missing_params, en);
        assert!(
            result.is_err(),
            "missing params should fail invariant #3"
        );
    }

    #[test]
    fn test_invariant_4_transfers_missing_node() {
        let (g, i, e, t, p, en) = make_valid_graph_data();
        // Remove transfer for the entry node (index `en[0]`)
        let mut missing_transfers = t;
        missing_transfers.remove(&en[0]);
        let result = GraphDef::from_components(g, i, e, missing_transfers, p, en);
        assert!(
            result.is_err(),
            "missing transfers should fail invariant #4"
        );
    }

    #[test]
    fn test_empty_graph_passes_invariants() {
        let graph = StableDiGraph::new();
        let index = HashMap::new();
        let edges = vec![];
        let transfers = HashMap::new();
        let params = HashMap::new();
        let entries = vec![];

        let def = GraphDef::from_components(
            graph, index, edges, transfers, params, entries,
        )
        .expect("empty graph should pass invariants");
        assert!(def.invariants_hold());
        assert_eq!(def.node_count(), 0);
    }

    #[test]
    fn test_node_lookup() {
        let (g, i, e, t, p, en) = make_valid_graph_data();
        let def = GraphDef::from_components(g, i, e, t, p, en)
            .expect("valid graph should pass invariants");
        let idx = def.node_index("A");
        assert!(idx.is_some(), "A should exist in index");
        let weight = def.node_weight(idx.expect("A should be found"));
        assert!(weight.is_some());
        assert_eq!(weight.expect("weight should exist").id, "A");
    }

    /// Verify that invariant #5 catches an edge not referenced by any transfer.
    #[test]
    fn test_invariant_5_edge_not_in_transfer() {
        let (g, i, _e, t, p, en) = make_valid_graph_data();
        // Create an edge that is NOT referenced by any transfer
        // (the existing valid transfer references edge index 0; add edge index 1)
        let orphan_edge = EdgeDef {
            from: NodeIndex::new(0),
            to: NodeIndex::new(1),
            event_type: EventType::Failed,
            exit_reason: None,
            threshold: 1,
            strategy: Strategy::All,
        };
        let edges = vec![
            EdgeDef {
                from: NodeIndex::new(0),
                to: NodeIndex::new(1),
                event_type: EventType::Complete,
                exit_reason: None,
                threshold: 1,
                strategy: Strategy::All,
            },
            orphan_edge,
        ];
        let result = GraphDef::from_components(g, i, edges, t, p, en);
        assert!(
            result.is_err(),
            "edge not in any transfer should fail invariant #5"
        );
    }

    /// Verify that accessor methods return correct values on a valid graph.
    #[test]
    fn test_accessors() {
        let (g, i, e, t, p, en) = make_valid_graph_data();
        let def = GraphDef::from_components(g, i, e, t, p, en)
            .expect("valid graph should pass invariants");

        assert_eq!(def.node_count(), 2);
        assert_eq!(def.edge_count(), 1);
        assert_eq!(def.entry_nodes().len(), 1);
        assert_eq!(
            def.entry_nodes()[0],
            def.node_index("A").expect("A should exist")
        );

        let all_indices: Vec<NodeIndex> = def.node_indices().collect();
        assert_eq!(all_indices.len(), 2);

        let params_a =
            def.node_params(def.node_index("A").expect("A should exist"));
        assert!(params_a.is_some());
        assert_eq!(
            params_a.expect("params for A should exist").process_timeout_secs,
            10
        );

        let params_b =
            def.node_params(def.node_index("B").expect("B should exist"));
        assert!(params_b.is_some());
        assert_eq!(
            params_b.expect("params for B should exist").process_timeout_secs,
            10
        );

        let transfers = def.transfers();
        assert_eq!(transfers.len(), 2);

        let edges = def.edges();
        assert_eq!(edges.len(), 1);
    }

    // ── NodeTransfer::evaluate tests ──────────────────────

    /// Build a simple chain: A → B (All, Complete, threshold=1).
    fn chain_transfer() -> (NodeTransfer, Vec<EdgeDef>, Vec<EdgeState>) {
        let a = NodeIndex::new(0);
        let b = NodeIndex::new(1);
        let transfer = NodeTransfer {
            from: a,
            out_edge_indices: vec![0],
        };
        let edges = vec![EdgeDef {
            from: a,
            to: b,
            event_type: EventType::Complete,
            exit_reason: None,
            threshold: 1,
            strategy: Strategy::All,
        }];
        let edge_states = vec![EdgeState::default()];
        (transfer, edges, edge_states)
    }

    #[test]
    fn test_transfer_evaluate_chain() {
        let (transfer, edges, mut edge_states) = chain_transfer();
        let b = NodeIndex::new(1);
        let mut fan_in_pending: HashMap<NodeIndex, usize> = HashMap::new();
        let mut ready_queue: VecDeque<NodeIndex> = VecDeque::new();

        let ready = transfer.evaluate(
            &edges,
            &mut edge_states,
            EventType::Complete,
            None,
            &mut fan_in_pending,
            &mut ready_queue,
        );

        assert_eq!(ready.len(), 1, "chain: A Complete should trigger B");
        assert_eq!(ready[0], b, "chain: should target B");
        assert_eq!(edge_states[0].event_count, 1, "chain: event_count should be 1");
    }

    #[test]
    fn test_transfer_evaluate_event_mismatch() {
        let (transfer, edges, mut edge_states) = chain_transfer();
        let mut fan_in_pending: HashMap<NodeIndex, usize> = HashMap::new();
        let mut ready_queue: VecDeque<NodeIndex> = VecDeque::new();

        // Edge expects Complete, send Failed — no trigger.
        let ready = transfer.evaluate(
            &edges,
            &mut edge_states,
            EventType::Failed,
            None,
            &mut fan_in_pending,
            &mut ready_queue,
        );

        assert!(ready.is_empty(), "Failed event should not trigger Complete edge");
        assert_eq!(edge_states[0].event_count, 0, "no match → no event_count increment");
    }

    #[test]
    fn test_transfer_evaluate_exit_reason_filter() {
        let a = NodeIndex::new(0);
        let b = NodeIndex::new(1);
        let transfer = NodeTransfer {
            from: a,
            out_edge_indices: vec![0],
        };
        let edges = vec![EdgeDef {
            from: a,
            to: b,
            event_type: EventType::Complete,
            exit_reason: Some("ok".into()),
            threshold: 1,
            strategy: Strategy::All,
        }];
        let mut edge_states = vec![EdgeState::default()];
        let mut fan_in_pending: HashMap<NodeIndex, usize> = HashMap::new();
        let mut ready_queue: VecDeque<NodeIndex> = VecDeque::new();

        // Wrong exit_reason → no trigger.
        let r1 = transfer.evaluate(
            &edges, &mut edge_states,
            EventType::Complete, Some("wrong"),
            &mut fan_in_pending, &mut ready_queue,
        );
        assert!(r1.is_empty(), "wrong exit_reason should not trigger");

        // Correct exit_reason → trigger.
        let mut edge_states2 = vec![EdgeState::default()];
        let mut ready_queue2: VecDeque<NodeIndex> = VecDeque::new();
        let r2 = transfer.evaluate(
            &edges, &mut edge_states2,
            EventType::Complete, Some("ok"),
            &mut fan_in_pending, &mut ready_queue2,
        );
        assert_eq!(r2.len(), 1, "correct exit_reason should trigger");
        assert_eq!(r2[0], b);
    }

    #[test]
    fn test_transfer_evaluate_threshold() {
        let a = NodeIndex::new(0);
        let b = NodeIndex::new(1);
        // threshold=3: need 3 events before triggering.
        let transfer = NodeTransfer {
            from: a,
            out_edge_indices: vec![0],
        };
        let edges = vec![EdgeDef {
            from: a, to: b, event_type: EventType::Complete,
            exit_reason: None, threshold: 3, strategy: Strategy::Any,
        }];
        let mut fan_in_pending: HashMap<NodeIndex, usize> = HashMap::new();

        // First two events → no trigger.
        let mut es1 = vec![EdgeState::default()];
        let mut rq1: VecDeque<NodeIndex> = VecDeque::new();
        let r1 = transfer.evaluate(
            &edges, &mut es1, EventType::Complete, None,
            &mut fan_in_pending, &mut rq1,
        );
        assert!(r1.is_empty(), "2 of 3 should not trigger");
        assert_eq!(es1[0].event_count, 1, "first event counted");

        let mut es2 = vec![EdgeState { event_count: 1 }];
        let mut rq2: VecDeque<NodeIndex> = VecDeque::new();
        let r2 = transfer.evaluate(
            &edges, &mut es2, EventType::Complete, None,
            &mut fan_in_pending, &mut rq2,
        );
        assert!(r2.is_empty(), "2 of 3 should not trigger");
        assert_eq!(es2[0].event_count, 2);

        // Third event → trigger.
        let mut es3 = vec![EdgeState { event_count: 2 }];
        let mut rq3: VecDeque<NodeIndex> = VecDeque::new();
        let r3 = transfer.evaluate(
            &edges, &mut es3, EventType::Complete, None,
            &mut fan_in_pending, &mut rq3,
        );
        assert_eq!(r3.len(), 1, "3rd event should trigger threshold=3");
        assert_eq!(r3[0], b);
    }

    #[test]
    fn test_transfer_evaluate_fan_in_all() {
        let a = NodeIndex::new(0);
        let b = NodeIndex::new(1);
        let c = NodeIndex::new(2);

        // A → C (All), B → C (All)
        let transfer_a = NodeTransfer { from: a, out_edge_indices: vec![0] };
        let transfer_b = NodeTransfer { from: b, out_edge_indices: vec![1] };

        let edges = vec![
            EdgeDef { from: a, to: c, event_type: EventType::Complete, exit_reason: None, threshold: 1, strategy: Strategy::All },
            EdgeDef { from: b, to: c, event_type: EventType::Complete, exit_reason: None, threshold: 1, strategy: Strategy::All },
        ];

        let mut fan_in_pending: HashMap<NodeIndex, usize> = HashMap::new();
        fan_in_pending.insert(c, 2); // 2 All edges → C

        // A completes → still need B.
        let mut edge_states = vec![EdgeState::default(), EdgeState::default()];
        let mut ready_queue: VecDeque<NodeIndex> = VecDeque::new();
        let r1 = transfer_a.evaluate(
            &edges, &mut edge_states, EventType::Complete, None,
            &mut fan_in_pending, &mut ready_queue,
        );
        assert!(r1.is_empty(), "A alone should not trigger C with All");
        assert_eq!(fan_in_pending[&c], 1, "fan_in_pending[C] should drop to 1");

        // B completes → C is now ready.
        let r2 = transfer_b.evaluate(
            &edges, &mut edge_states, EventType::Complete, None,
            &mut fan_in_pending, &mut ready_queue,
        );
        assert_eq!(r2.len(), 1, "both complete → C ready");
        assert_eq!(r2[0], c, "C should be ready");
        // fan_in_pending resets to 2 for next round.
        assert_eq!(fan_in_pending[&c], 2, "fan_in_pending[C] reset to 2");
        // Each transfer pushes to ready_queue when its All condition triggers.
        // A's evaluate returned empty (pending still 1).
        // B's evaluate triggered (pending 0) and pushed C.
        assert_eq!(ready_queue.len(), 1, "C pushed to ready_queue once (B's evaluate)");
    }

    #[test]
    fn test_transfer_evaluate_fan_out() {
        let a = NodeIndex::new(0);
        let b = NodeIndex::new(1);
        let c = NodeIndex::new(2);

        // A → B (Any), A → C (Any) — fan-out.
        let transfer = NodeTransfer { from: a, out_edge_indices: vec![0, 1] };
        let edges = vec![
            EdgeDef { from: a, to: b, event_type: EventType::Complete, exit_reason: None, threshold: 1, strategy: Strategy::Any },
            EdgeDef { from: a, to: c, event_type: EventType::Complete, exit_reason: None, threshold: 1, strategy: Strategy::Any },
        ];

        let mut edge_states = vec![EdgeState::default(), EdgeState::default()];
        let mut fan_in_pending: HashMap<NodeIndex, usize> = HashMap::new();
        let mut ready_queue: VecDeque<NodeIndex> = VecDeque::new();

        let ready = transfer.evaluate(
            &edges, &mut edge_states, EventType::Complete, None,
            &mut fan_in_pending, &mut ready_queue,
        );
        assert_eq!(ready.len(), 2, "fan-out should trigger both B and C");
        assert!(ready.contains(&b), "B should be in ready set");
        assert!(ready.contains(&c), "C should be in ready set");
        assert_eq!(ready_queue.len(), 2, "ready_queue should have both B and C");
    }

    #[test]
    fn test_transfer_evaluate_cycle_fires_repeatedly() {
        let a = NodeIndex::new(0);
        let b = NodeIndex::new(1);

        // A → B (Any), B → A (Any) — directed cycle.
        let transfer_a = NodeTransfer { from: a, out_edge_indices: vec![0] };
        let transfer_b = NodeTransfer { from: b, out_edge_indices: vec![1] };
        let edges = vec![
            EdgeDef { from: a, to: b, event_type: EventType::Complete, exit_reason: None, threshold: 1, strategy: Strategy::Any },
            EdgeDef { from: b, to: a, event_type: EventType::Complete, exit_reason: None, threshold: 1, strategy: Strategy::Any },
        ];

        let mut edge_states = vec![EdgeState::default(), EdgeState::default()];
        let mut fan_in_pending: HashMap<NodeIndex, usize> = HashMap::new();
        let mut ready_queue: VecDeque<NodeIndex> = VecDeque::new();

        // Round 1: A → B
        let r1 = transfer_a.evaluate(
            &edges, &mut edge_states, EventType::Complete, None,
            &mut fan_in_pending, &mut ready_queue,
        );
        assert_eq!(r1.len(), 1, "A→B round 1");
        assert_eq!(r1[0], b);

        // Round 2: B → A (no triggered gate)
        let r2 = transfer_b.evaluate(
            &edges, &mut edge_states, EventType::Complete, None,
            &mut fan_in_pending, &mut ready_queue,
        );
        assert_eq!(r2.len(), 1, "B→A round 2");
        assert_eq!(r2[0], a);

        // Round 3: A → B again (no triggered gate)
        let r3 = transfer_a.evaluate(
            &edges, &mut edge_states, EventType::Complete, None,
            &mut fan_in_pending, &mut ready_queue,
        );
        assert_eq!(r3.len(), 1, "A→B round 3 (no triggered gate — design philosophy)");
        assert_eq!(r3[0], b);
    }
}
