//! Integration tests for the MCP Server binary.
//!
//! Each test spawns `nexus-mcp-server` as a subprocess, sends a JSON-RPC
//! request via stdin, and validates the response on stdout.

use std::io::Write;
use std::process::{Command, Stdio};

/// Helper: send a JSON-RPC request to the MCP server and return the response.
fn mcp_call(method: &str, params: &serde_json::Value) -> serde_json::Value {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1
    });

    let mut child = Command::new(env!("CARGO_BIN_EXE_nexus-mcp-server"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn mcp-server");

    {
        let stdin = child.stdin.as_mut().expect("stdin not available");
        stdin
            .write_all(serde_json::to_string(&request).unwrap().as_bytes())
            .expect("write to stdin");
        stdin.write_all(b"\n").expect("write newline");
    }

    let output = child.wait_with_output().expect("mcp-server failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim()).expect("invalid JSON response")
}

// ── describe_schema ──────────────────────────────────────────

#[test]
fn test_describe_schema_returns_valid_schema() {
    let resp = mcp_call("describe_schema", &serde_json::json!({}));
    let schema = &resp["result"]["schema"];
    assert!(schema.is_object(), "schema should be an object");
    assert_eq!(schema["type"], "object", "schema type should be object");
    assert!(schema["properties"]["nodes"].is_object(), "should have nodes property");
    assert!(schema["required"].is_array(), "should have required array");
}

// ── validate_workflow ────────────────────────────────────────

#[test]
fn test_validate_empty_graph_fails() {
    let wf = r#"{"nodes":[]}"#;
    let resp = mcp_call(
        "validate_workflow",
        &serde_json::json!({"workflow_json": wf}),
    );
    assert_eq!(resp["result"]["valid"], false);
    assert!(!resp["result"]["errors"].as_array().unwrap().is_empty());
}

#[test]
fn test_validate_single_node_passes() {
    let wf = r#"{"nodes":[{"id":"a","providers":[{"type":"subprocess","command":"echo hi"}],"process_timeout_secs":10}]}"#;
    let resp = mcp_call(
        "validate_workflow",
        &serde_json::json!({"workflow_json": wf}),
    );
    assert_eq!(resp["result"]["valid"], true);
}

#[test]
fn test_validate_duplicate_ids_fails() {
    let wf = r#"{"nodes":[{"id":"a","providers":[{"type":"subprocess","command":"echo hi"}],"process_timeout_secs":10},{"id":"a","providers":[{"type":"subprocess","command":"echo hi"}],"process_timeout_secs":10}]}"#;
    let resp = mcp_call(
        "validate_workflow",
        &serde_json::json!({"workflow_json": wf}),
    );
    assert_eq!(resp["result"]["valid"], false);
}

#[test]
fn test_validate_cycle_without_entry_fails() {
    let wf = r#"{"nodes":[{"id":"a","providers":[{"type":"subprocess","command":"echo hi"}],"process_timeout_secs":10},{"id":"b","providers":[{"type":"subprocess","command":"echo hi"}],"process_timeout_secs":10}],"edges":[{"from":"a","to":"b","trigger":"any","event":"complete"},{"from":"b","to":"a","trigger":"any","event":"complete"}]}"#;
    let resp = mcp_call(
        "validate_workflow",
        &serde_json::json!({"workflow_json": wf}),
    );
    assert_eq!(resp["result"]["valid"], false);
}

// ── parse_workflow ───────────────────────────────────────────

#[test]
fn test_parse_workflow_single_node() {
    let wf = r#"{"nodes":[{"id":"hi","providers":[{"type":"subprocess","command":"echo hi"}],"process_timeout_secs":10}]}"#;
    let resp = mcp_call(
        "parse_workflow",
        &serde_json::json!({"workflow_json": wf}),
    );
    assert_eq!(resp["result"]["parsed"], true);
    assert!(resp["result"]["workflow"]["nodes"].is_array());
}

#[test]
fn test_parse_invalid_json_returns_error() {
    let resp = mcp_call(
        "parse_workflow",
        &serde_json::json!({"workflow_json": "not json"}),
    );
    // parse_workflow returns success with parsed=false for invalid JSON
    assert_eq!(resp["result"]["parsed"], false);
}

// ── run_workflow (minimal) ───────────────────────────────────

#[test]
fn test_run_workflow_single_node_completes() {
    let wf = if cfg!(windows) {
        r#"{"nodes":[{"id":"n","providers":[{"type":"shell","command":"echo {\"route\":\"ok\",\"content\":\"done\"}"}],"process_timeout_secs":10}]}"#
    } else {
        r#"{"nodes":[{"id":"n","providers":[{"type":"shell","command":"echo '{\"route\":\"ok\",\"content\":\"done\"}'"}],"process_timeout_secs":10}]}"#
    };
    let resp = mcp_call(
        "run_workflow",
        &serde_json::json!({"workflow_json": wf}),
    );
    assert_eq!(resp["result"]["status"], "completed");
    assert!(resp["result"]["nodes"]["n"].is_object());
}

#[test]
fn test_run_workflow_empty_graph_completes_silently() {
    // Empty graphs are valid at the engine level (validate() rejects them,
    // but Engine::new bypasses validation). Run completes instantly.
    let wf = r#"{"nodes":[]}"#;
    let resp = mcp_call(
        "run_workflow",
        &serde_json::json!({"workflow_json": wf}),
    );
    assert_eq!(resp["result"]["status"], "completed");
    assert_eq!(resp["result"]["nodes"].as_object().unwrap().len(), 0);
}

// ── Missing params ───────────────────────────────────────────

#[test]
fn test_missing_workflow_json_returns_error() {
    let resp = mcp_call("validate_workflow", &serde_json::json!({}));
    assert!(resp["error"]["code"].is_i64());
}

// ── Unknown method ───────────────────────────────────────────

#[test]
fn test_unknown_method_returns_error() {
    let resp = mcp_call("nonexistent_method", &serde_json::json!({}));
    assert_eq!(resp["error"]["code"], -32601);
}
