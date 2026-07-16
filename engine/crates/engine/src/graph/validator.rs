//! Workflow definition validation.
//!
//! The [`validate`] function applies all nine structural checks to a
//! [`WorkflowDef`] before it is passed to the builder.
//!
//! # Checks
//!
//! 1. **EmptyGraph** — zero nodes
//! 2. **DuplicateNodeId** — duplicate IDs in `nodes`
//! 3. **NoEntryNode** — all nodes have at least one predecessor
//! 4. **UnreachableNode** — BFS from entry nodes cannot reach every node
//! 5. **ExitNotReachable** — reverse BFS from exit nodes cannot reach every node
//! 6. **CycleWithoutEntry** — SCC with no entry node (deadlock detection)
//! 7. **NoValidProvider** — a node has an empty `providers` array
//! 8. **InputSourceNotFound** — `inputs` reference non-existent node IDs
//! 9. **InputSourceUnreachable** — input source node not reachable from any entry

use std::collections::{HashMap, HashSet, VecDeque};

use petgraph::algo::tarjan_scc;
use petgraph::graph::DiGraph;

use crate::model::predecessor::SchedulingEdgeDef;
use crate::model::workflow::{NodeDef, WorkflowDef};
use crate::model::{ValidationError, ValidationWarning};

/// Validate a [`WorkflowDef`] against all nine structural checks.
///
/// Returns `Ok(())` if all checks pass, or `Err(Vec<ValidationError>)` with
/// every error found (not just the first one).
///
/// # Errors
///
/// Returns a vector of [`ValidationError`] describing every detected problem.
#[allow(clippy::too_many_lines)]
pub fn validate(def: &WorkflowDef) -> Result<(), Vec<ValidationError>> {
    let mut errors: Vec<ValidationError> = Vec::new();

    // 1. EmptyGraph
    if def.nodes.is_empty() {
        errors.push(ValidationError::EmptyGraph);
        // No point checking further if there are no nodes.
        return Err(errors);
    }

    // 2. DuplicateNodeId
    let mut seen_ids: HashSet<&str> = HashSet::new();
    for node in &def.nodes {
        if !seen_ids.insert(node.id.as_str()) {
            errors.push(ValidationError::DuplicateNodeId {
                node_id: node.id.clone(),
            });
        }
    }

    // Build a node-id → NodeDef index map for later lookups.
    let id_to_node: HashMap<&str, &NodeDef> =
        def.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    // 7. NoValidProvider
    for node in &def.nodes {
        if node.providers.is_empty() {
            errors.push(ValidationError::NoValidProvider {
                node_id: node.id.clone(),
            });
        }
    }

    // 7b. ZeroTimeout
    for node in &def.nodes {
        if node.process_timeout_secs == 0 {
            errors.push(ValidationError::ZeroTimeout {
                node_id: node.id.clone(),
            });
        }
    }

    // 7c. LlmNodeMissingWrapper — type=llm nodes must have llm_node.py
    //     reachable via their resolved scripts_dir.  Mirrors the engine's
    //     resolution chain: CWD → exe-relative → common fallbacks.
    for node in &def.nodes {
        let has_llm = node.providers.iter().any(|p| {
            matches!(p, crate::model::provider::ProviderDef::Llm { .. })
        });
        if !has_llm {
            continue;
        }
        let sd = node.scripts_dir.as_deref()
            .or(def.scripts_dir.as_deref())
            .unwrap_or("./scripts");

        let found = {
            let cwd = std::path::PathBuf::from(sd).join("llm_node.py");
            let exe_rel = std::env::current_exe().ok().and_then(|exe| {
                let base = exe.parent()?.parent()?; // bin/ → release/
                Some(base.join(sd).join("llm_node.py"))
            }).unwrap_or_else(|| cwd.clone());
            let alt = std::path::PathBuf::from("release").join(sd).join("llm_node.py");
            cwd.exists() || exe_rel.exists() || alt.exists()
        };

        if !found {
            errors.push(ValidationError::LlmNodeMissingWrapper {
                node_id: node.id.clone(),
                scripts_dir: sd.to_string(),
            });
        }
    }

    // Build a set of all node IDs for quick lookup.
    let all_node_ids: HashSet<&str> =
        def.nodes.iter().map(|n| n.id.as_str()).collect();

    // 3. NoEntryNode
    // Entry nodes are those with no incoming scheduling edges.
    let has_incoming_edge: HashSet<&str> =
        def.edges.iter().map(|e| e.to.as_str()).collect();
    let entry_ids: Vec<&str> = def
        .nodes
        .iter()
        .filter(|n| !has_incoming_edge.contains(n.id.as_str()))
        .map(|n| n.id.as_str())
        .collect();

    if entry_ids.is_empty() {
        errors.push(ValidationError::NoEntryNode);
    }

    // Build shared child_map and reachable set for all reachability checks.
    // This avoids building the same adjacency map 3 times across 4/6/9.
    let child_map: HashMap<&str, Vec<&str>> = build_child_map(&def.edges);
    let reachable: HashSet<&str> = if entry_ids.is_empty() {
        HashSet::new()
    } else {
        bfs_from(entry_ids.as_slice(), &child_map)
    };

    // 4. UnreachableNode
    if !entry_ids.is_empty() {
        for node_id in &all_node_ids {
            if !reachable.contains(node_id) {
                errors.push(ValidationError::UnreachableNode {
                    node_id: (*node_id).to_string(),
                });
            }
        }

        // 5. ExitNotReachable — reverse BFS from exit nodes.
        // An exit node is one that doesn't appear as a 'from' in any edge.
        let has_outgoing_edge: HashSet<&str> =
            def.edges.iter().map(|e| e.from.as_str()).collect();
        let exit_ids: Vec<&str> = def
            .nodes
            .iter()
            .filter(|n| !has_outgoing_edge.contains(n.id.as_str()))
            .map(|n| n.id.as_str())
            .collect();

        // Build parent map: node → parents (reverse of child_map) — from edges.
        let mut parent_map: HashMap<&str, Vec<&str>> = HashMap::new();
        for edge in &def.edges {
            parent_map
                .entry(edge.to.as_str())
                .or_default()
                .push(edge.from.as_str());
        }

        let can_reach_exit: HashSet<&str> = bfs_from(&exit_ids, &parent_map);

        for node_id in &all_node_ids {
            if !can_reach_exit.contains(node_id) {
                errors.push(ValidationError::ExitNotReachable {
                    node_id: (*node_id).to_string(),
                });
            }
        }
    }

    // 6. CycleWithoutEntry (reuses `reachable` from above)
    if find_cycle_without_entry(&def.nodes, def.edges.as_slice(), &reachable).is_some() {
        errors.push(ValidationError::CycleWithoutEntry);
    }

    // 8. InputSourceNotFound
    for df in &def.dataflows {
        if !id_to_node.contains_key(df.from.as_str()) {
            errors.push(ValidationError::InputSourceNotFound {
                node_id: df.to.clone(),
                source_id: df.from.clone(),
            });
        }
        if !id_to_node.contains_key(df.to.as_str()) {
            errors.push(ValidationError::InputSourceNotFound {
                node_id: df.to.clone(),
                source_id: df.from.clone(),
            });
        }
    }

    // 9. InputSourceUnreachable (reuses `reachable` from above)
    if !entry_ids.is_empty() {
        for df in &def.dataflows {
            if id_to_node.contains_key(df.from.as_str())
                && !reachable.contains(df.from.as_str())
            {
                errors.push(ValidationError::InputSourceUnreachable {
                    node_id: df.to.clone(),
                    source_id: df.from.clone(),
                });
            }
        }
    }

    let valid_metadata_fields: &[&str] = &["run_count", "timed_out"];
    let valid_dr_fields: &[&str] = &["route", "content"];

    // 10. Validate template placeholders: {{metadata.<field>}},
    //     {{datarouter.<src>.<field>}}, and unrecognized {{prefix.key}}.
    {
        for node in &def.nodes {
            // Collect text to scan: prompt + command from all providers
            let mut text = String::new();
            for provider in &node.providers {
                match provider {
                    crate::model::provider::ProviderDef::Llm { prompt, command, .. } => {
                        if let Some(p) = prompt { text.push_str(p); text.push(' '); }
                        text.push_str(command);
                    }
                    crate::model::provider::ProviderDef::Subprocess { command }
                    | crate::model::provider::ProviderDef::Shell { command } => {
                        text.push_str(command);
                    }
                    _ => {}
                }
            }

            let incoming: HashSet<&str> = def
                .dataflows
                .iter()
                .filter(|df| df.to == node.id)
                .map(|df| df.from.as_str())
                .collect();

            // 10a. {{metadata.<field>}} — field must be run_count or timed_out.
            let mut rest = text.as_str();
            while let Some(start) = rest.find("{{metadata.") {
                let after = &rest[start + 11..];
                if let Some(end) = after.find("}}") {
                    let field = &after[..end];
                    if !valid_metadata_fields.contains(&field) {
                        errors.push(ValidationError::UnknownMetadataField {
                            node_id: node.id.clone(),
                            field: field.to_string(),
                        });
                    }
                    rest = &after[end + 2..];
                } else {
                    rest = after;
                }
            }

            // 10b. {{datarouter.<src>.<field>}} — source must have a
            //     dataflow, field must be "route" or "content".
            rest = text.as_str();
            while let Some(start) = rest.find("{{datarouter.") {
                let after = &rest[start + 13..];
                if let Some(end) = after.find("}}") {
                    let path = &after[..end];
                    if let Some(dot) = path.find('.') {
                        let src = &path[..dot];
                        let field = &path[dot + 1..];
                        if !valid_dr_fields.contains(&field) {
                            errors.push(ValidationError::UnknownDatarouterField {
                                node_id: node.id.clone(),
                                source_id: src.to_string(),
                                field: field.to_string(),
                            });
                        }
                        if !incoming.contains(src) && all_node_ids.contains(src) {
                            errors.push(ValidationError::DatarouterRefWithoutDataflow {
                                node_id: node.id.clone(),
                                source_id: src.to_string(),
                            });
                        }
                    }
                    rest = &after[end + 2..];
                } else {
                    rest = after;
                }
            }

            // 10c. Unrecognized {{prefix.key...}} — structured templates
            //     (contain '.') that don't match metadata/datarouter.
            rest = text.as_str();
            while let Some(start) = rest.find("{{") {
                let after = &rest[start + 2..];
                if let Some(end) = after.find("}}") {
                    let content = &after[..end];
                    if content.contains('.') {
                        let known = content.starts_with("metadata.")
                            || content.starts_with("datarouter.");
                        if !known {
                            errors.push(ValidationError::UnrecognizedTemplate {
                                node_id: node.id.clone(),
                                template: content.to_string(),
                            });
                        }
                    }
                    rest = &after[end + 2..];
                } else {
                    rest = after;
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Build a child-adjacency map from scheduling edges: node → [children].
fn build_child_map(edges: &[SchedulingEdgeDef]) -> HashMap<&str, Vec<&str>> {
    let mut child_map: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in edges {
        child_map
            .entry(edge.from.as_str())
            .or_default()
            .push(edge.to.as_str());
    }
    child_map
}

/// BFS traversal from `roots` following the edges in `adjacency`.
fn bfs_from<'a>(
    roots: &[&'a str],
    adjacency: &HashMap<&'a str, Vec<&'a str>>,
) -> HashSet<&'a str> {
    let mut visited: HashSet<&str> = HashSet::new();
    let mut queue: VecDeque<&str> = VecDeque::new();

    for &root in roots {
        if visited.insert(root) {
            queue.push_back(root);
        }
    }

    while let Some(node) = queue.pop_front() {
        if let Some(children) = adjacency.get(node) {
            for &child in children {
                if visited.insert(child) {
                    queue.push_back(child);
                }
            }
        }
    }

    visited
}

/// Detect cycles that are unreachable from any entry node using Tarjan's SCC.
///
/// Returns `Some` with the node ID of the first non-trivial SCC (size > 1)
/// where **all** nodes in the SCC are unreachable from entries, or a self-loop
/// at an unreachable node.
fn find_cycle_without_entry<'a>(
    nodes: &'a [NodeDef],
    edges: &[SchedulingEdgeDef],
    reachable_from_entries: &HashSet<&str>,
) -> Option<&'a str> {

    // Build a petgraph directed graph from the workflow definitions.
    // Edge direction: from → to.
    let mut graph = DiGraph::<&str, ()>::new();
    let mut indices: HashMap<&str, petgraph::graph::NodeIndex> = HashMap::new();

    for node_def in nodes {
        let idx = graph.add_node(node_def.id.as_str());
        indices.insert(node_def.id.as_str(), idx);
    }

    for edge in edges {
        if let (Some(&from_idx), Some(&to_idx)) =
            (indices.get(edge.from.as_str()), indices.get(edge.to.as_str()))
        {
            graph.add_edge(from_idx, to_idx, ());
        }
    }

    let sccs = tarjan_scc(&graph);

    for scc in &sccs {
        if scc.len() > 1 {
            // Non-trivial SCC — only flag it if ALL nodes are
            // unreachable from entries.
            let all_unreachable = scc.iter().all(|&idx| {
                let name = graph.node_weight(idx).unwrap_or(&"");
                !reachable_from_entries.contains(name)
            });
            if all_unreachable {
                let first_name: &str = scc
                    .iter()
                    .find_map(|&idx| graph.node_weight(idx))
                    .unwrap_or(&"");
                return nodes.iter().find(|n| n.id.as_str() == first_name).map(|n| n.id.as_str());
            }
        } else if scc.len() == 1 {
            // Self-loop: only flag if the node is unreachable from entries.
            let idx = scc[0];
            let name = graph.node_weight(idx).unwrap_or(&"");
            if !reachable_from_entries.contains(name) {
                let has_self_loop = edges.iter().any(|e| {
                    e.from == *name && e.to == *name
                });
                if has_self_loop {
                    return nodes.iter().find(|n| n.id.as_str() == *name).map(|n| n.id.as_str());
                }
            }
        }
    }

    None
}

/// Validate a [`WorkflowDef`] for advisory warnings.
///
/// Unlike [`validate`], warnings do NOT block execution. They flag common
/// configuration pitfalls that may cause runtime issues (e.g. missing
/// streaming output, multi-line prompts on Windows).
///
/// Returns a vector of [`ValidationWarning`] — empty means no warnings.
#[must_use]
pub fn validate_warnings(def: &WorkflowDef) -> Vec<ValidationWarning> {
    let mut warnings: Vec<ValidationWarning> = Vec::new();

    // Collect node-id → NodeDef for lookups
    let _id_to_node: HashMap<&str, &NodeDef> =
        def.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    for node in &def.nodes {
        for provider in &node.providers {
            if let crate::model::provider::ProviderDef::Llm { command, prompt, .. } = provider {
                // Warn: -p "{{prompt}}" + multi-line prompt → breaks on Windows
                if (command.contains("-p \"{{prompt}}\"") || command.contains("-p '{{prompt}}'"))
                    && prompt.as_ref().is_some_and(|p| p.contains('\n'))
                {
                    warnings.push(ValidationWarning::LlmPromptInCommandLine {
                        node_id: node.id.clone(),
                    });
                }

                // Warn: --output-format json without stream-json → no chunks
                if command.contains("--output-format json")
                    && !command.contains("stream-json")
                {
                    warnings.push(ValidationWarning::LlmNoStreamingOutput {
                        node_id: node.id.clone(),
                    });
                }
            }
        }

        // Warn: suspicious scripts_dir (redundant path segments when resolved)
        let sd = node.scripts_dir.as_deref()
            .or(def.scripts_dir.as_deref())
            .unwrap_or("");
        if !sd.is_empty() {
            let resolved = std::path::PathBuf::from(sd);
            let resolved_str = if resolved.is_absolute() {
                resolved.to_string_lossy().to_string()
            } else {
                std::env::current_dir()
                    .unwrap_or_default()
                    .join(&resolved)
                    .to_string_lossy()
                    .to_string()
            };
            let segments: Vec<&str> = resolved_str
                .split(['/', '\\'])
                .filter(|s| !s.is_empty() && *s != ".")
                .collect();
            let mut counts = std::collections::HashMap::new();
            for s in &segments {
                *counts.entry(*s).or_insert(0) += 1;
            }
            if counts.values().any(|&c| c > 1) {
                warnings.push(ValidationWarning::SuspiciousScriptsDir {
                    node_id: node.id.clone(),
                    scripts_dir: sd.to_string(),
                    resolved: resolved_str,
                });
            }
        }
    }

    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::predecessor::{DataFlowDef, EventType, SchedulingEdgeDef, TriggerExpr};
    use crate::model::provider::ProviderDef;
    use crate::model::workflow::NodeDef;

    fn make_node(id: &str) -> NodeDef {
        NodeDef { id: id.into(),
        providers: vec![ProviderDef::Subprocess {
            command: "echo".into(),
        }],
        process_timeout_secs: 10,
        returns: vec![],
        max_retries: None, route_policy: None, scripts_dir: None }
    }

    fn make_node_with_providers(id: &str, providers: Vec<ProviderDef>) -> NodeDef {
        NodeDef { providers, route_policy: None,
        ..make_node(id) }
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

    // ── 1. EmptyGraph ─────────────────────────────────────

    #[test]
    fn test_empty_graph() {
        let def = WorkflowDef {
            nodes: vec![],
            edges: vec![],
            dataflows: vec![],
            scripts_dir: None,
        };
        let result = validate(&def);
        assert!(result.is_err(), "empty graph should fail validation");
        let errors = result.unwrap_err();
        assert!(errors.contains(&ValidationError::EmptyGraph));
        // Should only report one error for empty graph.
        assert_eq!(errors.len(), 1);
    }

    // ── 2. DuplicateNodeId ────────────────────────────────

    #[test]
    fn test_duplicate_node_id() {
        let def = WorkflowDef {
            nodes: vec![make_node("a"), make_node("a")],
            edges: vec![],
            dataflows: vec![],
            scripts_dir: None,
        };
        let result = validate(&def);
        assert!(
            result.is_err(),
            "duplicate node ID should fail validation"
        );
        let errors = result.unwrap_err();
        assert!(errors.contains(&ValidationError::DuplicateNodeId {
            node_id: "a".into()
        }));
    }

    // ── 3. NoEntryNode ────────────────────────────────────

    #[test]
    fn test_no_entry_node() {
        // All nodes have incoming edges → no entry.
        let def = WorkflowDef {
            nodes: vec![make_node("a"), make_node("b")],
            edges: vec![sched_edge("a", "b"), sched_edge("b", "a")],
            dataflows: vec![],
            scripts_dir: None,
        };
        let result = validate(&def);
        assert!(
            result.is_err(),
            "no entry node should fail validation"
        );
        let errors = result.unwrap_err();
        assert!(errors.contains(&ValidationError::NoEntryNode));
    }

    // ── 4. UnreachableNode ────────────────────────────────

    #[test]
    fn test_unreachable_node() {
        // A → B; C → D → C (cycle, no entry, unreachable from A)
        let def = WorkflowDef {
            nodes: vec![make_node("a"), make_node("b"), make_node("c"), make_node("d")],
            edges: vec![
                sched_edge("a", "b"),
                sched_edge("d", "c"),
                sched_edge("c", "d"),
            ],
            dataflows: vec![],
            scripts_dir: None,
        };
        let result = validate(&def);
        assert!(
            result.is_err(),
            "unreachable node should fail validation"
        );
        let errors = result.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::UnreachableNode { node_id } if node_id == "c")),
            "node c should be reported as unreachable"
        );
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::UnreachableNode { node_id } if node_id == "d")),
            "node d should be reported as unreachable"
        );
    }

    // ── 5. ExitNotReachable ───────────────────────────────

    #[test]
    fn test_exit_not_reachable() {
        // A → B, C → C self-loop, no path from entry to C or from C to exit
        let def = WorkflowDef {
            nodes: vec![make_node("a"), make_node("b"), make_node("c")],
            edges: vec![
                sched_edge("a", "b"),
                SchedulingEdgeDef {
                    from: "c".into(),
                    to: "c".into(),
                    trigger: TriggerExpr::All,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 1,
                },
            ],
            dataflows: vec![],
            scripts_dir: None,
        };
        let result = validate(&def);
        assert!(
            result.is_err(),
            "exit not reachable should fail validation"
        );
        let errors = result.unwrap_err();
        let has_exit_err = errors.iter().any(|e| {
            matches!(e, ValidationError::ExitNotReachable { .. })
        });
        assert!(has_exit_err || errors.iter().any(|e| matches!(e, ValidationError::UnreachableNode { .. })));
    }

    // ── 6. CycleWithoutEntry ──────────────────────────────

    #[test]
    fn test_cycle_without_entry() {
        // A → B, B → A (no entry node)
        let def = WorkflowDef {
            nodes: vec![make_node("a"), make_node("b")],
            edges: vec![sched_edge("a", "b"), sched_edge("b", "a")],
            dataflows: vec![],
            scripts_dir: None,
        };
        let result = validate(&def);
        assert!(
            result.is_err(),
            "cycle without entry should fail validation"
        );
        let errors = result.unwrap_err();
        assert!(errors.contains(&ValidationError::CycleWithoutEntry));
    }

    // ── 7. NoValidProvider ────────────────────────────────

    #[test]
    fn test_no_valid_provider() {
        let def = WorkflowDef {
            nodes: vec![make_node_with_providers("a", vec![])],
            edges: vec![],
            dataflows: vec![],
            scripts_dir: None,
        };
        let result = validate(&def);
        assert!(
            result.is_err(),
            "no provider should fail validation"
        );
        let errors = result.unwrap_err();
        assert!(errors.contains(&ValidationError::NoValidProvider {
            node_id: "a".into()
        }));
    }

    // ── 8. InputSourceNotFound ────────────────────────────

    #[test]
    fn test_input_source_not_found() {
        let def = WorkflowDef {
            nodes: vec![make_node("a"), make_node("b")],
            edges: vec![],
            dataflows: vec![DataFlowDef {
                from: "nonexistent".into(),
                to: "b".into(),
                alias: None,
            }],
            scripts_dir: None,
        };
        let result = validate(&def);
        assert!(
            result.is_err(),
            "input source not found should fail validation"
        );
        let errors = result.unwrap_err();
        assert!(errors.contains(&ValidationError::InputSourceNotFound {
            node_id: "b".into(),
            source_id: "nonexistent".into(),
        }));
    }

    // ── 9. InputSourceUnreachable ─────────────────────────

    #[test]
    fn test_input_source_unreachable() {
        // A (entry) → B, C self-loop (not entry), D requests dataflow from C.
        let def = WorkflowDef {
            nodes: vec![make_node("a"), make_node("b"), make_node("c"), make_node("d")],
            edges: vec![
                sched_edge("a", "b"),
                SchedulingEdgeDef {
                    from: "c".into(),
                    to: "c".into(),
                    trigger: TriggerExpr::All,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 1,
                },
            ],
            dataflows: vec![DataFlowDef {
                from: "c".into(),
                to: "d".into(),
                alias: None,
            }],
            scripts_dir: None,
        };
        let result = validate(&def);
        assert!(
            result.is_err(),
            "input source unreachable should fail validation"
        );
        let errors = result.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::InputSourceUnreachable { node_id, source_id }
                    if node_id == "d" && source_id == "c")),
            "d's input source c is unreachable from entry"
        );
    }

    // ── Happy path ────────────────────────────────────────

    #[test]
    fn test_valid_chain_passes() {
        let def = WorkflowDef {
            nodes: vec![make_node("a"), make_node("b")],
            edges: vec![sched_edge("a", "b")],
            dataflows: vec![],
            scripts_dir: None,
        };
        let result = validate(&def);
        assert!(
            result.is_ok(),
            "valid chain should pass validation: {result:?}"
        );
    }

    #[test]
    fn test_valid_fan_in_passes() {
        let def = WorkflowDef {
            nodes: vec![make_node("a"), make_node("b"), make_node("c")],
            edges: vec![sched_edge("a", "c"), sched_edge("b", "c")],
            dataflows: vec![],
            scripts_dir: None,
        };
        let result = validate(&def);
        assert!(
            result.is_ok(),
            "valid fan-in should pass validation: {result:?}"
        );
    }

    #[test]
    fn test_valid_single_node_passes() {
        let def = WorkflowDef {
            nodes: vec![make_node("a")],
            edges: vec![],
            dataflows: vec![],
            scripts_dir: None,
        };
        let result = validate(&def);
        assert!(
            result.is_ok(),
            "single node should pass validation: {result:?}"
        );
    }

    /// Edge case: two isolated nodes should both be entries and pass.
    #[test]
    fn test_two_isolated_nodes_passes() {
        let def = WorkflowDef {
            nodes: vec![make_node("a"), make_node("b")],
            edges: vec![],
            dataflows: vec![],
            scripts_dir: None,
        };
        let result = validate(&def);
        assert!(
            result.is_ok(),
            "two isolated nodes should pass: {result:?}"
        );
    }
}
