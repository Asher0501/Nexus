//! Runtime data routing between graph nodes.
//!
//! The [`DataRouter`] manages the flow of output data from upstream nodes to
//! downstream consumers, driven by the workflow's `dataflows[]` declarations.
//! It maintains a pre-built index that maps each target node to its list of
//! input sources.

use std::collections::HashMap;

use petgraph::graph::NodeIndex;

/// Routes output data from upstream nodes to downstream consumers.
///
/// Driven by the workflow's `dataflows[]` declarations — not by per-node
/// `inputs` fields. Each target node automatically receives data from all
/// sources declared in the data flow topology.
#[derive(Debug, Clone)]
pub struct DataRouter {
    /// Maps each node index to its stored output string.
    outputs: HashMap<NodeIndex, String>,
    /// Pre-built index: target node → [(alias, source_node_index)]
    flow_index: HashMap<NodeIndex, Vec<(String, NodeIndex)>>,
}

impl DataRouter {
    /// Create a new `DataRouter` with the given node ID-to-index map and
    /// data flow definitions.
    ///
    /// Builds a pre-computed `flow_index` from the `dataflows[]` declarations
    /// so that `build_input` runs in O(inputs_of_target) time.
    #[must_use]
    pub fn new(
        node_id_to_index: HashMap<String, NodeIndex>,
        dataflows: &[crate::model::predecessor::DataFlowDef],
    ) -> Self {
        let mut flow_index: HashMap<NodeIndex, Vec<(String, NodeIndex)>> = HashMap::new();
        for df in dataflows {
            if let (Some(&from), Some(&to)) =
                (node_id_to_index.get(&df.from), node_id_to_index.get(&df.to))
            {
                let alias = df.alias.clone().unwrap_or_else(|| df.from.clone());
                flow_index.entry(to).or_default().push((alias, from));
            }
        }
        Self {
            outputs: HashMap::new(),
            flow_index,
        }
    }

    /// Store the output string produced by the given node.
    ///
    /// If the node already has stored output, it is overwritten.
    pub fn store_output(&mut self, node_index: NodeIndex, output: &str) {
        self.outputs.insert(node_index, output.to_string());
    }

    /// Remove the stored output for a node, typically called before retrying it
    /// so the downstream does not read stale data from the previous execution.
    pub fn clear_output(&mut self, node_index: NodeIndex) {
        self.outputs.remove(&node_index);
    }

    /// Build an input map for a target node by looking up its registered
    /// data flows.
    ///
    /// Returns a map of alias → output string for each source declared in
    /// the workflow's `dataflows[]`. Sources with no stored output yet
    /// yield an empty string.
    #[must_use]
    pub fn build_input(&self, target: NodeIndex) -> HashMap<String, String> {
        let mut result = HashMap::new();
        if let Some(flows) = self.flow_index.get(&target) {
            for (alias, source) in flows {
                let output = self.outputs.get(source).cloned().unwrap_or_default();
                result.insert(alias.clone(), output);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::predecessor::DataFlowDef;

    fn make_router(dataflows: Vec<DataFlowDef>) -> DataRouter {
        let mut id_to_idx = HashMap::new();
        id_to_idx.insert("A".into(), NodeIndex::new(0));
        id_to_idx.insert("B".into(), NodeIndex::new(1));
        id_to_idx.insert("C".into(), NodeIndex::new(2));
        DataRouter::new(id_to_idx, &dataflows)
    }

    fn df(from: &str, to: &str) -> DataFlowDef {
        DataFlowDef {
            from: from.to_string(),
            to: to.to_string(),
            alias: None,
        }
    }

    fn df_with_alias(from: &str, to: &str, alias: &str) -> DataFlowDef {
        DataFlowDef {
            from: from.to_string(),
            to: to.to_string(),
            alias: Some(alias.to_string()),
        }
    }

    #[test]
    fn test_store_and_retrieve() {
        let mut router = make_router(vec![df("A", "C"), df("B", "C")]);
        router.store_output(NodeIndex::new(0), "output_from_A");
        router.store_output(NodeIndex::new(1), "output_from_B");

        let inputs = router.build_input(NodeIndex::new(2)); // C's inputs
        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs.get("A"), Some(&"output_from_A".into()));
        assert_eq!(inputs.get("B"), Some(&"output_from_B".into()));
    }

    #[test]
    fn test_overwrite_output() {
        let mut router = make_router(vec![df("A", "B")]);
        router.store_output(NodeIndex::new(0), "first");
        router.store_output(NodeIndex::new(0), "second");

        let inputs = router.build_input(NodeIndex::new(1));
        assert_eq!(inputs.get("A"), Some(&"second".into()));
    }

    #[test]
    fn test_missing_output_returns_empty() {
        let mut router = make_router(vec![df("A", "B")]);
        router.store_output(NodeIndex::new(1), "B_data");
        // A has no stored output → should return empty string
        let inputs = router.build_input(NodeIndex::new(1));
        assert_eq!(inputs.get("A"), Some(&String::new()));
    }

    #[test]
    fn test_empty_flows_returns_empty() {
        let router = make_router(vec![]);
        let inputs = router.build_input(NodeIndex::new(0));
        assert!(inputs.is_empty());
    }

    #[test]
    fn test_clear_output_removes_stored_data() {
        let mut router = make_router(vec![df("A", "B")]);
        router.store_output(NodeIndex::new(0), "stale");
        router.clear_output(NodeIndex::new(0));

        // After clear, build_input should yield empty string (as if never stored).
        let inputs = router.build_input(NodeIndex::new(1));
        assert_eq!(inputs.get("A"), Some(&String::new()));
    }

    #[test]
    fn test_clear_output_noop_on_absent_node() {
        let mut router = make_router(vec![]);
        // Should not panic.
        router.clear_output(NodeIndex::new(99));
    }

    #[test]
    fn test_alias_maps_to_correct_key() {
        let mut router = make_router(vec![df_with_alias("A", "C", "input_a")]);
        router.store_output(NodeIndex::new(0), "val");

        let inputs = router.build_input(NodeIndex::new(2));
        assert_eq!(inputs.get("input_a"), Some(&"val".into()));
        assert!(inputs.get("A").is_none(), "alias should replace raw node ID");
    }
}
