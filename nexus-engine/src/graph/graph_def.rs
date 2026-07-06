//! Core graph definition type [`GraphDef`] and its component types.
//!
//! `GraphDef` is the verified, aggregated graph definition that all downstream
//! work (scheduling, routing, execution) derives from. It is constructed via
//! [`GraphDef::from_components`] which verifies five structural invariants
//! before returning an instance.

use std::collections::HashMap;

use petgraph::graph::NodeIndex;
use petgraph::stable_graph::StableDiGraph;

use crate::graph::edge::EdgeDef;
use crate::model::error::BuildError;
use crate::model::provider::ProviderDef;

/// Data associated with each node in the graph.
#[derive(Debug, Clone)]
pub struct NodeData {
    /// Node identifier from the workflow definition.
    pub id: String,
    /// How this node is executed.
    pub providers: Vec<ProviderDef>,
    /// Maximum execution time in seconds.
    pub process_timeout_secs: u64,
    /// Maximum concurrent executions for this node.
    pub max_concurrency: usize,
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
#[derive(Debug, Clone)]
pub struct NodeTransfer {
    /// The node this transfer belongs to.
    pub from: NodeIndex,
    /// Indices into [`GraphDef::edges`] for this node's outgoing edges.
    pub out_edge_indices: Vec<usize>,
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
    /// before returning. Returns [`BuildError`] if invariants fail.
    ///
    /// # Errors
    ///
    /// Returns `BuildError::InvalidNodeIndex` when any invariant check fails.
    pub fn from_components(
        graph: StableDiGraph<NodeData, ()>,
        index: HashMap<String, NodeIndex>,
        edges: Vec<EdgeDef>,
        transfers: HashMap<NodeIndex, NodeTransfer>,
        params: HashMap<NodeIndex, NodeParams>,
        entries: Vec<NodeIndex>,
    ) -> Result<Self, BuildError> {
        let def = Self {
            graph,
            index,
            edges,
            transfers,
            params,
            entries,
        };
        if let Err(detail) = def.check_invariants() {
            return Err(BuildError::InvalidNodeIndex {
                description: detail.into(),
            });
        }
        Ok(def)
    }

    /// Verify all five invariants hold.
    ///
    /// 1. All entries have valid [`NodeIndex`] values in the graph
    /// 2. All edges' `from_nodes` / `to` reference valid [`NodeIndex`] values
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
            for &from in &edge.from_nodes {
                if from.index() >= n {
                    return Err("invariant 2: edge from_nodes index out of bounds");
                }
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
    use std::collections::HashMap;

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

        let edges = vec![EdgeDef {
            from_nodes: vec![a],
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
            max_concurrency: 1,
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
            from_nodes: vec![NodeIndex::new(0)],
            to: NodeIndex::new(1),
            event_type: EventType::Failed,
            exit_reason: None,
            threshold: 1,
            strategy: Strategy::All,
        };
        let edges = vec![
            EdgeDef {
                from_nodes: vec![NodeIndex::new(0)],
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
}
