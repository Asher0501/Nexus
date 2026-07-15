//! Runtime data routing between graph nodes.
//!
//! The [`DataRouter`] manages the flow of output data from upstream nodes to
//! downstream consumers, driven by the workflow's `dataflows[]` declarations.
//! It maintains a pre-built index that maps each target node to its list of
//! input sources.

use std::collections::HashMap;

use petgraph::graph::NodeIndex;
use tracing;

use crate::nodeshell::NodeOutput;

/// Routes output data from upstream nodes to downstream consumers.
///
/// Driven by the workflow's `dataflows[]` declarations — not by per-node
/// `inputs` fields. Each target node automatically receives data from all
/// sources declared in the data flow topology.
#[derive(Debug, Clone)]
pub struct DataRouter {
    /// Maps each node index to its stored output (route + content).
    outputs: HashMap<NodeIndex, NodeOutput>,
    /// Pre-built index: target node → [(alias, source_node_index)]
    flow_index: HashMap<NodeIndex, Vec<(String, NodeIndex)>>,
    /// Reverse mapping: NodeIndex → string node ID (for diagnostics).
    index_to_id: HashMap<NodeIndex, String>,
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
        let index_to_id: HashMap<NodeIndex, String> = node_id_to_index
            .iter()
            .map(|(id, idx)| (*idx, id.clone()))
            .collect();

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
            index_to_id,
        }
    }

    /// Store the output produced by the given node.
    ///
    /// If the node already has stored output, it is overwritten.
    pub fn store_output(&mut self, node_index: NodeIndex, output: &NodeOutput) {
        self.outputs.insert(node_index, output.clone());
    }

    /// Remove the stored output for a node, typically called before retrying it
    /// so the downstream does not read stale data from the previous execution.
    pub fn clear_output(&mut self, node_index: NodeIndex) {
        self.outputs.remove(&node_index);
    }

    /// Build an input map for a target node by looking up its registered
    /// data flows.
    ///
    /// Returns a map of alias → content string for each source declared in
    /// the workflow's `dataflows[]`. Sources with no stored output yet
    /// yield an empty string.
    #[must_use]
    pub fn build_input(&self, target: NodeIndex) -> HashMap<String, String> {
        let mut result = HashMap::new();
        if let Some(flows) = self.flow_index.get(&target) {
            for (alias, source) in flows {
                match self.outputs.get(source) {
                    Some(output) if !output.content.is_empty() => {
                        result.insert(alias.clone(), output.content.clone());
                    }
                    _ => {
                        let source_id = self
                            .index_to_id
                            .get(source)
                            .map(|s| s.as_str())
                            .unwrap_or("unknown");
                        tracing::info!(
                            target: "nexus::diagnostic",
                            "[Engine.DataRouter] Task node:{} no msg.",
                            source_id,
                        );
                        result.insert(alias.clone(), String::new());
                    }
                }
            }
        }
        result
    }

    /// Build an upstream output map for the target node.
    ///
    /// Returns a map of alias → full NodeOutput (route + content).
    /// Used by the template engine to resolve `{{datarouter.<alias>.route}}`
    /// and `{{datarouter.<alias>.content}}` references.
    /// Sources with no stored output yield `{route: "", content: ""}`.
    #[must_use]
    pub fn build_upstream(&self, target: NodeIndex) -> HashMap<String, NodeOutput> {
        let mut result = HashMap::new();
        if let Some(flows) = self.flow_index.get(&target) {
            for (alias, source) in flows {
                let output = self
                    .outputs
                    .get(source)
                    .cloned()
                    .unwrap_or_else(|| NodeOutput {
                        route: String::new(),
                        content: String::new(),
                    });
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

    fn no(content: &str) -> NodeOutput {
        NodeOutput { route: "ok".into(), content: content.into() }
    }

    fn no_route(route: &str, content: &str) -> NodeOutput {
        NodeOutput { route: route.into(), content: content.into() }
    }

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
        router.store_output(NodeIndex::new(0), &no("output_from_A"));
        router.store_output(NodeIndex::new(1), &no("output_from_B"));

        let inputs = router.build_input(NodeIndex::new(2)); // C's inputs
        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs.get("A"), Some(&"output_from_A".into()));
        assert_eq!(inputs.get("B"), Some(&"output_from_B".into()));
    }

    #[test]
    fn test_overwrite_output() {
        let mut router = make_router(vec![df("A", "B")]);
        router.store_output(NodeIndex::new(0), &no("first"));
        router.store_output(NodeIndex::new(0), &no("second"));

        let inputs = router.build_input(NodeIndex::new(1));
        assert_eq!(inputs.get("A"), Some(&"second".into()));
    }

    #[test]
    fn test_missing_output_returns_empty() {
        let mut router = make_router(vec![df("A", "B")]);
        router.store_output(NodeIndex::new(1), &no("B_data"));
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
        router.store_output(NodeIndex::new(0), &no("stale"));
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
        router.store_output(NodeIndex::new(0), &no("val"));

        let inputs = router.build_input(NodeIndex::new(2));
        assert_eq!(inputs.get("input_a"), Some(&"val".into()));
        assert!(inputs.get("A").is_none(), "alias should replace raw node ID");
    }

    #[test]
    fn test_build_upstream() {
        let mut router = make_router(vec![df("A", "C"), df("B", "C")]);
        router.store_output(NodeIndex::new(0), &no_route("complete", "data_a"));
        router.store_output(NodeIndex::new(1), &no_route("fixed", "data_b"));

        let upstream = router.build_upstream(NodeIndex::new(2));
        assert_eq!(upstream.get("A").map(|o| o.route.as_str()), Some("complete"));
        assert_eq!(upstream.get("A").map(|o| o.content.as_str()), Some("data_a"));
        assert_eq!(upstream.get("B").map(|o| o.route.as_str()), Some("fixed"));
    }

    #[test]
    fn test_build_upstream_missing_returns_empty() {
        let router = make_router(vec![df("A", "C")]);
        let upstream = router.build_upstream(NodeIndex::new(2));
        assert_eq!(upstream.get("A").map(|o| o.route.as_str()), Some(""));
        assert_eq!(upstream.get("A").map(|o| o.content.as_str()), Some(""));
    }
}
