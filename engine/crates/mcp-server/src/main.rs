//! Nexus MCP Server — JSON-RPC stdio-based MCP server for the Nexus engine.
//!
//! This binary exposes the nexus-engine as an MCP tool via stdin/stdout.
//! It supports four methods:
//! - `validate_workflow` — validate a workflow JSON string
//! - `parse_workflow` — parse a workflow JSON into structured output
//! - `describe_schema` — return the WorkflowDef JSON schema
//! - `run_workflow` — parse and execute a workflow.
//!   If `dashboard_url` is provided, delegates to the Dashboard HTTP API
//!   so the run appears in the real-time monitoring UI.

use std::io::{self, BufRead, Write};

use serde::Serialize;
use serde_json::Value;

use nexus_engine::graph::validate;
use nexus_engine::model::{EngineConfig, WorkflowDef};
use nexus_engine::runtime::Engine;

/// JSON-RPC success response envelope.
#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    pub jsonrpc: String,
    pub result: Value,
    pub id: Value,
}

/// JSON-RPC error response envelope.
#[derive(Debug, Serialize)]
struct JsonRpcErrorResponse {
    pub jsonrpc: String,
    pub error: JsonRpcError,
    pub id: Value,
}

/// JSON-RPC error object.
#[derive(Debug, Serialize)]
struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

fn make_success(result: Value, id: Value) -> Value {
    serde_json::to_value(JsonRpcResponse {
        jsonrpc: "2.0".into(),
        result,
        id,
    })
    .unwrap_or_default()
}

fn make_error(code: i64, message: String, id: Value) -> Value {
    serde_json::to_value(JsonRpcErrorResponse {
        jsonrpc: "2.0".into(),
        error: JsonRpcError { code, message },
        id,
    })
    .unwrap_or_default()
}

/// Extract JSON string parameter from the `params` object.
fn extract_json_str(params: &Value) -> Result<String, String> {
    params
        .get("workflow_json")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .ok_or_else(|| "Missing required parameter: workflow_json".into())
}

async fn handle_validate(params: &Value, id: &Value) -> Value {
    let json_str = match extract_json_str(params) {
        Ok(s) => s,
        Err(e) => {
            return make_error(-32602, e, id.clone());
        }
    };

    let def: Result<WorkflowDef, _> = serde_json::from_str(&json_str);
    match def {
        Ok(wf) => {
            let warnings: Vec<String> = nexus_engine::graph::validate_warnings(&wf)
                .iter()
                .map(std::string::ToString::to_string)
                .collect();
            match validate(&wf) {
                Ok(()) => make_success(
                    serde_json::json!({"valid": true, "errors": [], "warnings": warnings}),
                    id.clone(),
                ),
                Err(errors) => {
                    let err_strs: Vec<String> = errors.iter().map(std::string::ToString::to_string).collect();
                    make_success(
                        serde_json::json!({"valid": false, "errors": err_strs, "warnings": warnings}),
                        id.clone(),
                    )
                }
            }
        },
        Err(e) => make_success(
            serde_json::json!({"valid": false, "errors": [format!("JSON parse error: {e}")]}),
            id.clone(),
        ),
    }
}

async fn handle_parse(params: &Value, id: &Value) -> Value {
    let json_str = match extract_json_str(params) {
        Ok(s) => s,
        Err(e) => {
            return make_error(-32602, e, id.clone());
        }
    };

    let def: Result<WorkflowDef, _> = serde_json::from_str(&json_str);
    match def {
        Ok(wf) => {
            let pretty = serde_json::to_string_pretty(&wf).unwrap_or_default();
            make_success(
                serde_json::json!({
                    "parsed": true,
                    "workflow": wf,
                    "pretty_json": pretty
                }),
                id.clone(),
            )
        }
        Err(e) => make_success(
            serde_json::json!({"parsed": false, "error": format!("{e}")}),
            id.clone(),
        ),
    }
}

async fn handle_describe_schema(id: &Value) -> Value {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "nodes": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["id", "providers", "process_timeout_secs"],
                    "properties": {
                        "id": {"type": "string", "description": "Unique node identifier"},
                        "providers": {
                            "type": "array",
                            "items": {
                                "oneOf": [
                                    {
                                        "type": "object",
                                        "properties": {
                                            "type": {"const": "subprocess"},
                                            "command": {"type": "string"}
                                        },
                                        "required": ["type", "command"]
                                    },
                                    {
                                        "type": "object",
                                        "properties": {
                                            "type": {"const": "shell"},
                                            "command": {"type": "string"}
                                        },
                                        "required": ["type", "command"]
                                    },
                                    {
                                        "type": "object",
                                        "properties": {
                                            "type": {"const": "http"},
                                            "url": {"type": "string"},
                                            "method": {"type": "string"}
                                        },
                                        "required": ["type", "url"]
                                    },
                                    {
                                        "type": "object",
                                        "properties": {
                                            "type": {"const": "llm_sdk"},
                                            "model": {"type": "string"},
                                            "api_key_env": {"type": "string"},
                                            "system_prompt": {"type": "string"},
                                            "prompt": {"type": "string"},
                                            "routes": {"type": "array", "items": {"type": "string"}},
                                            "max_tokens": {"type": "integer"}
                                        },
                                        "required": ["type", "model"]
                                    },
                                    {
                                        "type": "object",
                                        "properties": {
                                            "type": {"const": "llm"},
                                            "command": {"type": "string"},
                                            "prompt": {"type": "string"},
                                            "routes": {"type": "array", "items": {"type": "string"}},
                                            "max_tokens": {"type": "integer"}
                                        },
                                        "required": ["type", "command"]
                                    }
                                ]
                            }
                        },
                        "process_timeout_secs": {"type": "integer", "minimum": 1},
                        "returns": {
                            "type": "array",
                            "items": {"type": "string"}
                        },
                        "max_retries": {
                            "type": "integer",
                            "minimum": 0
                        },
                        "route_policy": {
                            "type": "object",
                            "properties": {
                                "type": {"type": "string", "enum": ["max_runs"]},
                                "max": {"type": "integer", "minimum": 1},
                                "then_route": {"type": "string"}
                            },
                            "required": ["type", "max", "then_route"]
                        }
                    }
                }
            },
            "edges": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["from", "to", "trigger", "event"],
                    "properties": {
                        "from": {"type": "string", "description": "Source node ID"},
                        "to": {"type": "string", "description": "Target node ID"},
                        "trigger": {"type": "string", "enum": ["all", "any"]},
                        "event": {"type": "string", "enum": ["complete", "failed", "timeout"]},
                        "exit_reason": {"type": "string"},
                        "threshold": {"type": "integer", "minimum": 1}
                    }
                }
            },
            "dataflows": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["from", "to"],
                    "properties": {
                        "from": {"type": "string", "description": "Source node ID"},
                        "to": {"type": "string", "description": "Target node ID"},
                        "alias": {"type": "string", "description": "Key in target's inputs; defaults to source node ID"}
                    }
                }
            }
        },
        "required": ["nodes"]
    });

    make_success(serde_json::json!({"schema": schema}), id.clone())
}

async fn handle_run(params: &Value, id: &Value) -> Value {
    let json_str = match extract_json_str(params) {
        Ok(s) => s,
        Err(e) => return make_error(-32602, e, id.clone()),
    };

    // If dashboard_url is provided, delegate to the Dashboard HTTP API.
    // This way the run appears in the real-time monitoring UI.
    if let Some(dashboard_url) = params.get("dashboard_url").and_then(Value::as_str) {
        return handle_run_via_dashboard(&json_str, dashboard_url, id).await;
    }

    // Local execution path (no dashboard).
    let def: WorkflowDef = match serde_json::from_str(&json_str) {
        Ok(d) => d,
        Err(e) => return make_error(-32602, format!("Parse error: {e}"), id.clone()),
    };

    let config = EngineConfig::default();
    let mut engine = match Engine::new(def, config, None) {
        Ok(e) => e,
        Err(errors) => {
            let err_strs: Vec<String> = errors.iter().map(std::string::ToString::to_string).collect();
            return make_success(
                serde_json::json!({"valid": false, "errors": err_strs}),
                id.clone(),
            );
        }
    };

    let run_id = uuid::Uuid::new_v4().to_string();
    match engine.run().await {
        Ok(result) => {
            let nodes: Value = result
                .snapshot
                .nodes
                .iter()
                .map(|(nid, ns)| {
                    (
                        nid.clone(),
                        serde_json::json!({
                            "status": format!("{:?}", ns.status),
                            "result": format!("{:?}", ns.result),
                            "retry_count": ns.retry_count,
                        }),
                    )
                })
                .collect::<serde_json::Map<_, _>>()
                .into();
            make_success(
                serde_json::json!({
                    "run_id": run_id,
                    "status": "completed",
                    "duration_secs": result.snapshot.elapsed.as_secs(),
                    "running_count": result.snapshot.running_count(),
                    "nodes": nodes,
                }),
                id.clone(),
            )
        }
        Err(e) => make_error(-32603, format!("Runtime error: {e}"), id.clone()),
    }
}

/// Proxy `run_workflow` to a running Dashboard instance via its HTTP API.
///
/// 1. POST `/api/workflows` → create the workflow (temp name).
/// 2. POST `/api/workflows/{id}/run` → trigger execution.
/// 3. Return `run_id` + `dashboard_url` so the caller can poll or
///    open the monitoring UI.
async fn handle_run_via_dashboard(json_str: &str, dashboard_url: &str, id: &Value) -> Value {
    let base = dashboard_url.trim_end_matches('/');
    let client = reqwest::Client::new();

    // 1. Create a temporary workflow on the Dashboard.
    let short_id = uuid::Uuid::new_v4().to_string();
    let wf_name = format!("mcp-{}", &short_id[..8]);
    let create_body = serde_json::json!({"name": wf_name, "definition": json_str});
    let resp = match client
        .post(format!("{base}/api/workflows"))
        .json(&create_body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return make_error(-32603, format!("Dashboard unreachable: {e}"), id.clone());
        }
    };
    let created: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return make_error(-32603, format!("Dashboard response parse error: {e}"), id.clone());
        }
    };
    let wf_id = match created.get("id").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => {
            return make_error(
                -32603,
                format!("Dashboard did not return workflow id: {created}"),
                id.clone(),
            );
        }
    };

    // 2. Trigger the run.
    let resp = match client
        .post(format!("{base}/api/workflows/{wf_id}/run"))
        .json(&serde_json::json!({}))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return make_error(-32603, format!("Dashboard trigger failed: {e}"), id.clone());
        }
    };
    let triggered: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return make_error(-32603, format!("Dashboard trigger response parse error: {e}"), id.clone());
        }
    };
    let run_id = match triggered.get("run_id").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => {
            return make_error(
                -32603,
                format!("Dashboard did not return run_id: {triggered}"),
                id.clone(),
            );
        }
    };

    make_success(
        serde_json::json!({
            "run_id": run_id,
            "workflow_id": wf_id,
            "dashboard_url": dashboard_url,
            "monitor_url": format!("{base}/#/runs/{run_id}"),
            "status": "accepted",
            "note": "Run is executing in the background. Open monitor_url in a browser or poll GET /api/runs/{run_id} for status."
        }),
        id.clone(),
    )
}

#[tokio::main]
async fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }

        // Parse the JSON-RPC request.
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let err = make_error(-32700, format!("Parse error: {e}"), Value::Null);
                let mut out = stdout.lock();
                // Best-effort: MCP transport failure means the client disconnected — discard.
                let _ = writeln!(out, "{}", serde_json::to_string(&err).unwrap());
                let _ = out.flush();
                continue;
            }
        };

        let method = match request.get("method").and_then(Value::as_str) {
            Some(m) => m.to_string(),
            None => {
                let err = make_error(-32600, "Missing method field".into(), request.get("id").cloned().unwrap_or(Value::Null));
                let mut out = stdout.lock();
                // Best-effort: MCP transport failure means the client disconnected — discard.
                let _ = writeln!(out, "{}", serde_json::to_string(&err).unwrap());
                let _ = out.flush();
                continue;
            }
        };

        let params = request.get("params").cloned().unwrap_or(Value::Null);
        let id = request.get("id").cloned().unwrap_or(Value::Null);

        let response = match method.as_str() {
            "validate_workflow" => handle_validate(&params, &id).await,
            "parse_workflow" => handle_parse(&params, &id).await,
            "describe_schema" => handle_describe_schema(&id).await,
            "run_workflow" => handle_run(&params, &id).await,
            _ => make_error(-32601, format!("Method not found: {method}"), id),
        };

        let mut out = stdout.lock();
        // Best-effort: MCP transport failure means the client disconnected — discard.
        let _ = writeln!(out, "{}", serde_json::to_string(&response).unwrap());
        let _ = out.flush();
    }
}
