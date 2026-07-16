//! Fuzz / boundary tests for `WorkflowDef` JSON deserialization,
//! validation, and graph building — full pipeline, no panic.
//!
//! Ensures that arbitrary JSON input never panics at any stage:
//!   serde parse → validate → `Builder::build`

#![allow(unused_crate_dependencies)]

use nexus_engine::graph::validate;
use nexus_engine::graph::Builder;
use nexus_engine::model::provider::ProviderDef;
use nexus_engine::model::workflow::{RoutePolicyDef, WorkflowDef};

/// Run the full pipeline on a JSON string: parse → validate → build.
/// Must never panic — even on adversarial inputs.
fn full_pipeline_no_panic(json: &str) {
    let wf: Result<WorkflowDef, _> = serde_json::from_str(json);
    let Ok(wf) = wf else { return };
    // Validation may fail with errors — that's fine, just no panic.
    if validate(&wf).is_ok() {
        // If validation passes, building must succeed (or return errors, not panic).
        let _ = Builder::build(&wf);
    }
}

/// Verify that a JSON string does not panic during deserialization.
fn no_panic(json: &str) {
    let _: Result<WorkflowDef, _> = serde_json::from_str(json);
}

/// Verify `ProviderDef` deserialization doesn't panic.
fn provider_no_panic(json: &str) {
    let _: Result<ProviderDef, _> = serde_json::from_str(json);
}

/// Verify `RoutePolicyDef` deserialization doesn't panic.
fn rp_no_panic(json: &str) {
    let _: Result<RoutePolicyDef, _> = serde_json::from_str(json);
}

// ── Malformed / adversarial inputs ─────────────────────────

#[test]
fn fuzz_empty_string() { no_panic(""); }

#[test]
fn fuzz_not_json() { no_panic("not valid json at all!!!"); }

#[test]
fn fuzz_half_json() { no_panic(r#"{"nodes":[{"id":"#); }

#[test]
fn fuzz_wrong_type_nodes() { no_panic(r#"{"nodes":42,"edges":[]}"#); }

#[test]
fn fuzz_wrong_type_edges() { no_panic(r#"{"nodes":[],"edges":"not_array"}"#); }

#[test]
fn fuzz_deep_nesting() {
    let deep = format!(r#"{{"nodes":{}"edges":[],"dataflows":[]}}"#, "[".repeat(500));
    no_panic(&deep);
}

#[test]
fn fuzz_huge_field() {
    let huge = format!(
        r#"{{"nodes":[{{"id":"x","providers":[{{"type":"subprocess","command":"{}"}}],"process_timeout_secs":10}}],"edges":[]}}"#,
        "A".repeat(1_000_000)
    );
    no_panic(&huge);
}

#[test]
fn fuzz_unterminated_string() {
    no_panic(r#"{"nodes":[{"id":"unclosed)"#);
}

#[test]
fn fuzz_null_bytes() {
    no_panic("{\"nodes\":[{\"id\":\"a\\u0000b\"}]}");
}

#[test]
fn fuzz_invalid_unicode() {
    no_panic("{\"nodes\":[{\"id\":\"a\\uFFFD\\u0000b\"}]}");
}

#[test]
fn fuzz_only_braces() { no_panic("{}"); no_panic("[]"); no_panic(r#"{"a":{}}"#); }

#[test]
fn fuzz_missing_required_fields() {
    no_panic(r#"{"nodes":[{}]}"#);          // no id, no providers, no timeout
    no_panic(r#"{"nodes":[{"id":"x"}]}"#);   // no providers, no timeout
}

#[test]
fn fuzz_negative_timeout() {
    // Negative u64 → serde should fail, not panic
    no_panic(r#"{"nodes":[{"id":"x","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":-1}]}"#);
}

#[test]
fn fuzz_duplicate_node_ids() {
    let json = r#"{
        "nodes":[
            {"id":"dup","providers":[{"type":"subprocess","command":"e1"}],"process_timeout_secs":10},
            {"id":"dup","providers":[{"type":"subprocess","command":"e2"}],"process_timeout_secs":10}
        ],
        "edges":[]
    }"#;
    full_pipeline_no_panic(json);
}

#[test]
fn fuzz_self_loop_edge() {
    let json = r#"{
        "nodes":[
            {"id":"A","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10}
        ],
        "edges":[{"from":"A","to":"A","trigger":"any","event":"complete","threshold":5}],
        "dataflows":[]
    }"#;
    full_pipeline_no_panic(json);
}

// ── Full pipeline: validate + build edge cases ────────────

#[test]
fn fuzz_pipeline_empty_nodes() {
    full_pipeline_no_panic(r#"{"nodes":[],"edges":[]}"#);
}

#[test]
fn fuzz_pipeline_all_isolated_nodes_valid() {
    full_pipeline_no_panic(r#"{"nodes":[{"id":"A","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10},{"id":"B","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10}],"edges":[],"dataflows":[]}"#);
}

#[test]
fn fuzz_pipeline_fan_in_all() {
    full_pipeline_no_panic(r#"{"nodes":[{"id":"A","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10},{"id":"B","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10},{"id":"C","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10}],"edges":[{"from":"A","to":"C","trigger":"all","event":"complete"},{"from":"B","to":"C","trigger":"all","event":"complete"}],"dataflows":[]}"#);
}

#[test]
fn fuzz_pipeline_cycle_with_route_policy() {
    full_pipeline_no_panic(r#"{"nodes":[{"id":"A","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10,"route_policy":{"type":"max_runs","max":3,"then_route":"exit"}},{"id":"B","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10}],"edges":[{"from":"A","to":"B","trigger":"any","event":"complete","exit_reason":"again"},{"from":"B","to":"A","trigger":"any","event":"complete"},{"from":"A","to":"B","trigger":"any","event":"complete","exit_reason":"exit"}],"dataflows":[{"from":"B","to":"A"}]}"#);
}

#[test]
fn fuzz_pipeline_dataflow_skip_level() {
    full_pipeline_no_panic(r#"{"nodes":[{"id":"A","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10},{"id":"B","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10},{"id":"C","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10}],"edges":[{"from":"A","to":"B","trigger":"any","event":"complete"},{"from":"B","to":"C","trigger":"any","event":"complete"}],"dataflows":[{"from":"A","to":"C"}]}"#);
}

#[test]
fn fuzz_pipeline_node_with_max_duration() {
    full_pipeline_no_panic(r#"{"nodes":[{"id":"A","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10,"route_policy":{"type":"max_duration","max_secs":60,"then_route":"timeout"}}],"edges":[]}"#);
}

#[test]
fn fuzz_pipeline_http_with_all_fields() {
    full_pipeline_no_panic(r#"{"nodes":[{"id":"A","providers":[{"type":"http","url":"https://example.com","method":"POST","headers":{"Auth":"x"},"body":"{}"}],"process_timeout_secs":15}],"edges":[]}"#);
}

#[test]
fn fuzz_pipeline_edge_to_nonexistent_node() {
    // Edge references non-existent node — validator should catch, not panic
    full_pipeline_no_panic(r#"{"nodes":[{"id":"A","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10}],"edges":[{"from":"A","to":"GHOST","trigger":"any","event":"complete"}],"dataflows":[]}"#);
}

#[test]
fn fuzz_pipeline_dataflow_to_nonexistent_node() {
    full_pipeline_no_panic(r#"{"nodes":[{"id":"A","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10}],"edges":[],"dataflows":[{"from":"A","to":"GHOST"}]}"#);
}

#[test]
fn fuzz_pipeline_unreachable_component() {
    // C→D is isolated and unreachable from entry A
    full_pipeline_no_panic(r#"{"nodes":[{"id":"A","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10},{"id":"B","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10},{"id":"C","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10},{"id":"D","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10}],"edges":[{"from":"A","to":"B","trigger":"any","event":"complete"},{"from":"C","to":"D","trigger":"any","event":"complete"}],"dataflows":[]}"#);
}

#[test]
fn fuzz_pipeline_self_loop_without_entry() {
    // Self-loop on a node with no entry edge — validator catches
    full_pipeline_no_panic(r#"{"nodes":[{"id":"A","providers":[{"type":"subprocess","command":"e"}],"process_timeout_secs":10}],"edges":[{"from":"A","to":"A","trigger":"any","event":"complete"}],"dataflows":[]}"#);
}

// ── All ProviderDef variants ────────────────────────────────

#[test]
fn fuzz_all_provider_types() {
    provider_no_panic(r#"{"type":"subprocess","command":"echo"}"#);
    provider_no_panic(r#"{"type":"shell","command":"echo hello"}"#);
    provider_no_panic(r#"{"type":"http","url":"https://example.com","method":"GET"}"#);
    provider_no_panic(r#"{"type":"http","url":"https://example.com","method":"POST","headers":{"A":"b"},"body":"{}"}"#);
    provider_no_panic(r#"{"type":"llm","command":"claude","prompt":"test","routes":["ok"]}"#);
    provider_no_panic(r#"{"type":"llm_sdk","model":"claude-sonnet-5-20251001","prompt":"hi","routes":["ok","err"]}"#);
    // Unknown type
    provider_no_panic(r#"{"type":"nonexistent_provider_xyz"}"#);
}

// ── All RoutePolicyDef variants ─────────────────────────────

#[test]
fn fuzz_all_route_policy_types() {
    rp_no_panic(r#"{"type":"max_runs","max":3,"then_route":"approved"}"#);
    rp_no_panic(r#"{"type":"max_duration","max_secs":300,"then_route":"timeout_exit"}"#);
    // Unknown type
    rp_no_panic(r#"{"type":"unknown_policy"}"#);
    // Missing fields
    let r: Result<RoutePolicyDef, _> = serde_json::from_str(r#"{"type":"max_runs"}"#);
    assert!(r.is_err());
}

// ── Roundtrip: valid workflow survives serialize → deserialize ──

#[test]
fn fuzz_valid_roundtrip_all_features() {
    let original = serde_json::json!({
        "nodes": [
            {"id":"A","providers":[{"type":"subprocess","command":"echo"}],"process_timeout_secs":10,"max_retries":2},
            {"id":"B","providers":[{"type":"http","url":"https://example.com","method":"POST","headers":{"Auth":"x"},"body":"{}"}],"process_timeout_secs":30,"route_policy":{"type":"max_runs","max":3,"then_route":"stop"},"returns":["ok","err"]},
            {"id":"C","providers":[{"type":"llm_sdk","model":"m","prompt":"p","routes":["ok"],"max_tokens":512}],"process_timeout_secs":60,"route_policy":{"type":"max_duration","max_secs":600,"then_route":"timeout"}}
        ],
        "edges":[
            {"from":"A","to":"B","trigger":"all","event":"complete","exit_reason":"ok","threshold":2},
            {"from":"B","to":"C","trigger":"any","event":"failed"}
        ],
        "dataflows":[
            {"from":"A","to":"B"},
            {"from":"A","to":"C","alias":"skip"}
        ],
        "scripts_dir": "./scripts"
    });
    let s = original.to_string();
    let wf: WorkflowDef = serde_json::from_str(&s).expect("valid workflow must parse");
    let roundtrip = serde_json::to_string(&wf).expect("serialize");
    let recovered: WorkflowDef = serde_json::from_str(&roundtrip).expect("roundtrip parse");
    assert_eq!(wf, recovered);
    assert_eq!(wf.nodes.len(), 3);
    assert_eq!(wf.edges.len(), 2);
    assert_eq!(wf.dataflows.len(), 2);
}
