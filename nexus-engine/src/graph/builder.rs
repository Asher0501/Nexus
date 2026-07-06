//! Builder for constructing a [`GraphDef`] from a [`WorkflowDef`].
//!
//! The [`Builder`] type converts a validated workflow definition into the
//! internal graph representation by:
//!
//! 1. Creating nodes in a [`StableDiGraph`]
//! 2. Building edges from predecessor declarations
//! 3. Constructing local transfer functions (`f_v`)
//! 4. Extracting per-node execution parameters

use std::collections::{HashMap, HashSet};

use petgraph::graph::NodeIndex;
use petgraph::stable_graph::StableDiGraph;

use crate::graph::edge::{EdgeDef, Strategy};
use crate::graph::graph_def::{GraphDef, NodeData, NodeParams, NodeTransfer};
use crate::model::error::ValidationError;
use crate::model::predecessor::{EventType, TriggerExpr};
use crate::model::workflow::WorkflowDef;

/// Converts a validated [`WorkflowDef`] into a [`GraphDef`].
///
/// # Errors
///
/// Returns a vector of [`ValidationError`] if the definition contains issues
/// that are detected during the build phase (duplicate IDs, invalid
/// predecessors).
pub struct Builder;

impl Builder {
    /// Build a complete [`GraphDef`] from a [`WorkflowDef`].
    ///
    /// This is the primary entry point. It runs all build phases in order:
    ///
    /// 1. Create nodes in the underlying stable digraph
    /// 2. Build edges from `PredecessorDef` declarations
    /// 3. Build transfer functions (`f_v`) that aggregate outgoing edges per node
    /// 4. Extract per-node execution parameters
    ///
    /// # Errors
    ///
    /// Returns `Err(Vec<ValidationError>)` if:
    /// - A duplicate node ID is encountered
    /// - A predecessor references a non-existent node ID
    /// - The resulting components fail [`GraphDef::from_components`] invariants
    pub fn build(def: &WorkflowDef) -> Result<GraphDef, Vec<ValidationError>> {
        let (graph, index) = Self::create_nodes(def)?;
        let (edges, entries) = Self::build_edges(def, &index)?;
        let transfers = Self::build_transfers(&edges, &graph)?;
        let params = Self::build_params(def, &index)?;

        GraphDef::from_components(graph, index, edges, transfers, params, entries)
            .map_err(|e| vec![ValidationError::InvalidPredecessor {
                node_id: String::new(),
                predecessor_id: e.to_string(),
            }])
    }

    /// Create all nodes in the stable digraph and build the ID-to-index map.
    ///
    /// # Errors
    ///
    /// Returns `DuplicateNodeId` if a node ID appears more than once.
    fn create_nodes(
        def: &WorkflowDef,
    ) -> Result<(StableDiGraph<NodeData, ()>, HashMap<String, NodeIndex>), Vec<ValidationError>>
    {
        let mut graph: StableDiGraph<NodeData, ()> = StableDiGraph::new();
        let mut index: HashMap<String, NodeIndex> = HashMap::new();

        // Pre-process dataflows: group inputs by target node ID.
        let mut input_map: HashMap<&str, Vec<String>> = HashMap::new();
        for df in &def.dataflows {
            input_map
                .entry(df.to.as_str())
                .or_default()
                .push(df.alias.clone().unwrap_or_else(|| df.from.clone()));
        }

        for node_def in &def.nodes {
            if index.contains_key(&node_def.id) {
                return Err(vec![ValidationError::DuplicateNodeId {
                    node_id: node_def.id.clone(),
                }]);
            }
            let node_data = NodeData {
                id: node_def.id.clone(),
                providers: node_def.providers.clone(),
                process_timeout_secs: node_def.process_timeout_secs,
                max_concurrency: node_def.max_concurrency.unwrap_or(1),
            };
            let idx = graph.add_node(node_data);
            index.insert(node_def.id.clone(), idx);
        }

        Ok((graph, index))
    }

    /// Build edges from the workflow definition's scheduling edges.
    ///
    /// Groups [`SchedulingEdgeDef`] entries by their `(to, trigger, event,
    /// exit_reason, threshold)` tuple, merging `from_nodes` for entries that
    /// share the same grouping key. This produces one [`EdgeDef`] per unique
    /// combination.
    ///
    /// Also detects entry nodes — nodes that have no incoming edges.
    ///
    /// # Errors
    ///
    /// Returns `InvalidPredecessor` if an edge references a node ID that
    /// does not exist in `index`.
    fn build_edges(
        def: &WorkflowDef,
        index: &HashMap<String, NodeIndex>,
    ) -> Result<(Vec<EdgeDef>, Vec<NodeIndex>), Vec<ValidationError>> {
        // Key: (to_node_id, trigger, event, exit_reason, threshold)
        // Value: set of from_node NodeIndices
        type EdgeKey = (String, TriggerExpr, EventType, Option<String>, u64);

        let mut edge_groups: HashMap<EdgeKey, HashSet<NodeIndex>> = HashMap::new();
        let mut has_incoming_edge: HashSet<String> = HashSet::new();

        for edge_def in &def.edges {
            let from_idx = match index.get(&edge_def.from) {
                Some(idx) => *idx,
                None => {
                    return Err(vec![ValidationError::InvalidPredecessor {
                        node_id: edge_def.to.clone(),
                        predecessor_id: edge_def.from.clone(),
                    }]);
                }
            };
            if !index.contains_key(&edge_def.to) {
                return Err(vec![ValidationError::InvalidPredecessor {
                    node_id: edge_def.to.clone(),
                    predecessor_id: edge_def.from.clone(),
                }]);
            }

            has_incoming_edge.insert(edge_def.to.clone());

            let key: EdgeKey = (
                edge_def.to.clone(),
                edge_def.trigger.clone(),
                edge_def.event.clone(),
                edge_def.exit_reason.clone(),
                edge_def.threshold,
            );
            edge_groups.entry(key).or_default().insert(from_idx);

            // Track the 'to' node as well for unreferenced check — we also consider
            // that the 'from' node appears in the scheduling graph.
            has_incoming_edge.insert(edge_def.to.clone());
        }

        // Build edges from groups.
        let edges: Vec<EdgeDef> = edge_groups
            .into_iter()
            .map(|((to_id, trigger, event, exit_reason, threshold), from_set)| {
                let mut from_nodes: Vec<NodeIndex> = from_set.into_iter().collect();
                from_nodes.sort_by_key(|ni| ni.index());
                EdgeDef {
                    from_nodes,
                    to: index[&to_id],
                    event_type: event,
                    exit_reason,
                    threshold,
                    strategy: match trigger {
                        TriggerExpr::All => Strategy::All,
                        TriggerExpr::Any => Strategy::Any,
                    },
                }
            })
            .collect();

        // Detect entry nodes: nodes that have no incoming edges.
        let entries: Vec<NodeIndex> = def
            .nodes
            .iter()
            .filter(|n| !has_incoming_edge.contains(n.id.as_str()))
            .map(|n| index[&n.id])
            .collect();

        Ok((edges, entries))
    }

    /// Build transfer functions (`f_v`) that aggregate outgoing edges per node.
    ///
    /// Each node gets a [`NodeTransfer`] listing the edge indices where it
    /// appears as a `from_node`.
    fn build_transfers(
        edges: &[EdgeDef],
        graph: &StableDiGraph<NodeData, ()>,
    ) -> Result<HashMap<NodeIndex, NodeTransfer>, Vec<ValidationError>> {
        let mut transfers: HashMap<NodeIndex, NodeTransfer> = HashMap::new();

        for node_idx in graph.node_indices() {
            transfers.insert(
                node_idx,
                NodeTransfer {
                    from: node_idx,
                    out_edge_indices: vec![],
                },
            );
        }

        for (edge_idx, edge) in edges.iter().enumerate() {
            for &from_node in &edge.from_nodes {
                if let Some(transfer) = transfers.get_mut(&from_node) {
                    transfer.out_edge_indices.push(edge_idx);
                }
            }
        }

        Ok(transfers)
    }

    /// Extract per-node execution parameters from the workflow definition.
    fn build_params(
        def: &WorkflowDef,
        index: &HashMap<String, NodeIndex>,
    ) -> Result<HashMap<NodeIndex, NodeParams>, Vec<ValidationError>> {
        let params: HashMap<NodeIndex, NodeParams> = def
            .nodes
            .iter()
            .map(|n| {
                (
                    index[&n.id],
                    NodeParams {
                        process_timeout_secs: n.process_timeout_secs,
                    },
                )
            })
            .collect();
        Ok(params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::predecessor::{DataFlowDef, EventType, SchedulingEdgeDef, TriggerExpr};
    use crate::model::provider::ProviderDef;
    use crate::model::workflow::{NodeDef, WorkflowDef};

    fn make_node(id: &str) -> NodeDef {
        NodeDef {
            id: id.into(),
            providers: vec![ProviderDef::Subprocess {
                command: "echo".into(),
            }],
            process_timeout_secs: 10,
            max_concurrency: None,
            returns: vec![],
            max_retries: None,
        }
    }

    fn sched_edge(from: &str, to: &str) -> SchedulingEdgeDef {
        SchedulingEdgeDef {
            from: from.into(),
            to: to.into(),
            trigger: TriggerExpr::All,
            event: EventType::Complete,
            exit_reason: None,
            threshold: 1,
        }
    }

    // ── 3-node chain ──────────────────────────────────────

    #[test]
    fn test_build_three_node_chain() {
        // A → B → C
        let def = WorkflowDef {
            nodes: vec![make_node("A"), make_node("B"), make_node("C")],
            edges: vec![sched_edge("A", "B"), sched_edge("B", "C")],
            dataflows: vec![],
        };
        let graph = Builder::build(&def).expect("3-node chain should build");
        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edge_count(), 2);
        assert_eq!(graph.entry_nodes().len(), 1);
        assert_eq!(
            graph.entry_nodes()[0],
            graph.node_index("A").expect("A should exist")
        );
    }

    // ── Fan-out / Fan-in ──────────────────────────────────

    #[test]
    fn test_build_fan_out_fan_in() {
        // A → B, A → C, B → D, C → D
        let def = WorkflowDef {
            nodes: vec![make_node("A"), make_node("B"), make_node("C"), make_node("D")],
            edges: vec![
                sched_edge("A", "B"),
                sched_edge("A", "C"),
                sched_edge("B", "D"),
                sched_edge("C", "D"),
            ],
            dataflows: vec![],
        };
        let graph = Builder::build(&def).expect("fan-out/fan-in should build");
        assert_eq!(graph.node_count(), 4);
        // B→D and C→D share the same EdgeKey (D, All, Complete, None, 1),
        // so they merge into one edge with two from_nodes. Total: 3 edges.
        assert_eq!(graph.edge_count(), 3);
        assert_eq!(graph.entry_nodes().len(), 1);
        assert_eq!(
            graph.entry_nodes()[0],
            graph.node_index("A").expect("A should exist")
        );
    }

    // ── Duplicate node ID ─────────────────────────────────

    #[test]
    fn test_build_duplicate_id() {
        let def = WorkflowDef {
            nodes: vec![make_node("X"), make_node("X")],
            edges: vec![],
            dataflows: vec![],
        };
        let result = Builder::build(&def);
        assert!(result.is_err(), "duplicate ID should fail");
        let errors = result.expect_err("expected errors");
        assert!(errors.contains(&ValidationError::DuplicateNodeId {
            node_id: "X".into()
        }));
    }

    // ── Invalid predecessor ───────────────────────────────

    #[test]
    fn test_build_invalid_predecessor() {
        let def = WorkflowDef {
            nodes: vec![make_node("A"), make_node("B")],
            edges: vec![SchedulingEdgeDef {
                from: "NONEXISTENT".into(),
                to: "B".into(),
                trigger: TriggerExpr::All,
                event: EventType::Complete,
                exit_reason: None,
                threshold: 1,
            }],
            dataflows: vec![],
        };
        let result = Builder::build(&def);
        assert!(result.is_err(), "invalid predecessor should fail");
    }

    // ── Empty graph ───────────────────────────────────────

    #[test]
    fn test_build_empty_graph() {
        let def = WorkflowDef {
            nodes: vec![],
            edges: vec![],
            dataflows: vec![],
        };
        let graph = Builder::build(&def).expect("empty graph should build");
        assert_eq!(graph.node_count(), 0);
        assert_eq!(graph.edge_count(), 0);
        assert!(graph.entry_nodes().is_empty());
    }

    // ── Single node ───────────────────────────────────────

    #[test]
    fn test_build_single_node() {
        let def = WorkflowDef {
            nodes: vec![make_node("A")],
            edges: vec![],
            dataflows: vec![],
        };
        let graph = Builder::build(&def).expect("single node should build");
        assert_eq!(graph.node_count(), 1);
        assert_eq!(graph.edge_count(), 0);
        assert_eq!(graph.entry_nodes().len(), 1);
        assert_eq!(
            graph.entry_nodes()[0],
            graph.node_index("A").expect("A should exist")
        );
    }

    // ── Verify NodeData content ───────────────────────────

    #[test]
    fn test_build_node_data_content() {
        let def = WorkflowDef {
            nodes: vec![
                NodeDef {
                    id: "worker".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: "python".into(),
                    }],
                    process_timeout_secs: 60,
                    max_concurrency: Some(4),
                    returns: vec!["ok".into()],
                    max_retries: Some(5),
                },
                make_node("collector"),
            ],
            edges: vec![sched_edge("worker", "collector")],
            dataflows: vec![DataFlowDef {
                from: "input_a".into(),
                to: "worker".into(),
                alias: None,
            }],
        };
        let graph = Builder::build(&def).expect("should build");
        let worker_idx = graph
            .node_index("worker")
            .expect("worker should exist");
        let data = graph
            .node_weight(worker_idx)
            .expect("worker weight should exist");

        assert_eq!(data.id, "worker");
        assert_eq!(data.process_timeout_secs, 60);
        assert_eq!(data.max_concurrency, 4);
        assert_eq!(data.providers.len(), 1);

        let params = graph
            .node_params(worker_idx)
            .expect("worker params should exist");
        assert_eq!(params.process_timeout_secs, 60);
    }

    // ── Strategy mapping ──────────────────────────────────

    #[test]
    fn test_build_strategy_mapping() {
        // A → B (All), A → C (Any)
        let def = WorkflowDef {
            nodes: vec![make_node("A"), make_node("B"), make_node("C")],
            edges: vec![
                sched_edge("A", "B"),
                SchedulingEdgeDef {
                    from: "A".into(),
                    to: "C".into(),
                    trigger: TriggerExpr::Any,
                    event: EventType::Failed,
                    exit_reason: None,
                    threshold: 1,
                },
            ],
            dataflows: vec![],
        };
        let graph = Builder::build(&def).expect("should build");
        assert_eq!(graph.edge_count(), 2);

        // Verify that strategies are set correctly (edges are ordered by (to, ...) grouping).
        let edges = graph.edges();
        for edge in edges {
            if edge.event_type == EventType::Complete {
                assert_eq!(edge.strategy, Strategy::All);
            } else if edge.event_type == EventType::Failed {
                assert_eq!(edge.strategy, Strategy::Any);
            }
        }
    }

    // ── Merge same-key predecessors ───────────────────────

    #[test]
    fn test_build_merges_same_key_predecessors() {
        // C has predecessors from A and B with identical trigger/event/exit_reason/threshold
        let def = WorkflowDef {
            nodes: vec![make_node("A"), make_node("B"), make_node("C")],
            edges: vec![
                SchedulingEdgeDef {
                    from: "A".into(),
                    to: "C".into(),
                    trigger: TriggerExpr::All,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 2,
                },
                SchedulingEdgeDef {
                    from: "B".into(),
                    to: "C".into(),
                    trigger: TriggerExpr::All,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 2,
                },
            ],
            dataflows: vec![],
        };
        let graph = Builder::build(&def).expect("should build");
        // Two predecessors with same key should merge into one edge with 2 from_nodes.
        assert_eq!(graph.edge_count(), 1);
        let edge = &graph.edges()[0];
        assert_eq!(edge.from_nodes.len(), 2);
        assert_eq!(edge.threshold, 2);
    }
}
