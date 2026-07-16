//! API integration tests for nexus-dashboard.
//!
//! Each test builds a fresh server on a random port with its own in-memory
//! (temp-file) database, ensuring complete isolation. Tests exercise the full
//! HTTP request/response cycle through reqwest.
//!
//! API endpoints covered:
//!   API-1  GET    /api/workflows
//!   API-2  POST   /api/workflows
//!   API-3  GET    /api/workflows/{id}
//!   API-4  PUT    /api/workflows/{id}
//!   API-5  DELETE /api/workflows/{id}
//!   API-6  GET    /api/workflows/{id}/graph
//!   API-7  GET    /api/workflows/{id}/run
//!   API-8  POST   /api/workflows/{id}/run
//!   API-9  GET    /api/runs
//!   API-10 GET    /`api/runs/{run_id`}
//!   API-11 POST   /`api/runs/{run_id}/stop`
//!   API-12 GET    /`api/runs/{run_id}/graph`
//!   API-13 GET    /api/workflows (empty list)
//!   API-14 POST   /api/workflows (duplicate id via same body — should succeed with new id)
//!   API-15 GET    /api/workflows/{id} (not found)
//!   API-16 PUT    /api/workflows/{id} (not found — `SQLite` UPDATE succeeds with 0 rows)
//!   API-17 DELETE /api/workflows/{id} (not found)

#![allow(missing_docs)]
// Silence unused-crate-dependencies — used via nexus_dashboard crate.
#[expect(unused_imports)]
use futures as _futures;
#[expect(unused_imports)]
use futures_util as _futures_util;
#[expect(unused_imports)]
use nexus_engine as _nexus_engine;
#[expect(unused_imports)]
use rusqlite as _rusqlite;
#[expect(unused_imports)]
use serde as _serde;
#[expect(unused_imports)]
use tokio_tungstenite as _tokio_tungstenite;
#[expect(unused_imports)]
use tower as _tower;
#[expect(unused_imports)]
use tracing as _tracing;
#[expect(unused_imports)]
use tracing_subscriber as _tracing_subscriber;

use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use tower_http::cors::CorsLayer;

use nexus_dashboard::{api, db::Store, state::AppState, ws::WsRoom};

/// Helper: start a test server, return the base URL.
async fn spawn_server() -> (String, Store, Arc<WsRoom>) {
    let dir = std::env::temp_dir().join(format!("nexus-int-api-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).ok();
    let db_path = dir.join("test.db");

    let store = Store::new(Some(db_path)).expect("test db creation");
    let room = WsRoom::new();
    let state = AppState {
        store: store.clone(),
        room: room.clone(),
        cancel_flags: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
    };

    let app: Router = Router::new()
        .route("/ws/runs/{run_id}", get(nexus_dashboard::ws::ws_handler))
        .nest("/api", api::routes())
        .with_state(state)
        .layer(CorsLayer::permissive());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind random port");
    let addr = listener.local_addr().expect("get local addr");
    let base_url = format!("http://{addr}");

    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("server runs");
    });

    (base_url, store, room)
}

/// Create a workflow via API and return its id.
async fn create_workflow(client: &reqwest::Client, base_url: &str, name: &str) -> String {
    let body = serde_json::json!({
        "name": name,
        "definition": {
            "nodes": [{"id": "step1"}],
            "edges": []
        }
    });
    let resp = client
        .post(format!("{base_url}/api/workflows"))
        .json(&body)
        .send()
        .await
        .expect("create workflow");
    assert_eq!(resp.status(), 200, "create workflow should return 200");
    let val: serde_json::Value = resp.json().await.expect("parse create response");
    val["id"].as_str().expect("id present").to_string()
}

// ---------------------------------------------------------------------------
// API-1: GET /api/workflows — list workflows
// ---------------------------------------------------------------------------
#[tokio::test]
async fn api_01_list_workflows() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base_url}/api/workflows"))
        .send()
        .await
        .expect("list workflows");
    assert_eq!(resp.status(), 200);
    let val: serde_json::Value = resp.json().await.expect("parse list response");
    assert!(val.is_array(), "response should be an array");
}

// ---------------------------------------------------------------------------
// API-2: POST /api/workflows — create a workflow
// ---------------------------------------------------------------------------
#[tokio::test]
async fn api_02_create_workflow() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "name": "my-workflow",
        "definition": {"nodes": [], "edges": []}
    });
    let resp = client
        .post(format!("{base_url}/api/workflows"))
        .json(&body)
        .send()
        .await
        .expect("create workflow");
    assert_eq!(resp.status(), 200);
    let val: serde_json::Value = resp.json().await.expect("parse create response");
    assert_eq!(val["status"], "created");
    assert!(val["id"].as_str().is_some(), "should return an id");
}

// ---------------------------------------------------------------------------
// API-3: GET /api/workflows/{id} — get workflow by id
// ---------------------------------------------------------------------------
#[tokio::test]
async fn api_03_get_workflow_by_id() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    let wf_id = create_workflow(&client, &base_url, "get-test").await;

    let resp = client
        .get(format!("{base_url}/api/workflows/{wf_id}"))
        .send()
        .await
        .expect("get workflow");
    assert_eq!(resp.status(), 200);
    let val: serde_json::Value = resp.json().await.expect("parse get response");
    assert_eq!(val["id"], wf_id);
    assert_eq!(val["name"], "get-test");
    assert!(val["definition"].is_string(), "definition should be a JSON string");
}

// ---------------------------------------------------------------------------
// API-4: PUT /api/workflows/{id} — update a workflow
// ---------------------------------------------------------------------------
#[tokio::test]
async fn api_04_update_workflow() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    let wf_id = create_workflow(&client, &base_url, "update-test").await;

    let body = serde_json::json!({
        "name": "updated-name",
        "definition": {"nodes": [{"id": "new-node"}], "edges": []}
    });
    let resp = client
        .put(format!("{base_url}/api/workflows/{wf_id}"))
        .json(&body)
        .send()
        .await
        .expect("update workflow");
    assert_eq!(resp.status(), 200);
    let val: serde_json::Value = resp.json().await.expect("parse update response");
    assert_eq!(val["status"], "updated");

    // Verify the update persisted
    let get_resp = client
        .get(format!("{base_url}/api/workflows/{wf_id}"))
        .send()
        .await
        .expect("get workflow after update");
    let get_val: serde_json::Value = get_resp.json().await.expect("parse get response");
    assert_eq!(get_val["name"], "updated-name");
}

// ---------------------------------------------------------------------------
// API-5: DELETE /api/workflows/{id} — delete a workflow
// ---------------------------------------------------------------------------
#[tokio::test]
async fn api_05_delete_workflow() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    let wf_id = create_workflow(&client, &base_url, "delete-test").await;

    let resp = client
        .delete(format!("{base_url}/api/workflows/{wf_id}"))
        .send()
        .await
        .expect("delete workflow");
    assert_eq!(resp.status(), 200);
    let val: serde_json::Value = resp.json().await.expect("parse delete response");
    assert_eq!(val["status"], "deleted");

    // Verify it's gone
    let get_resp = client
        .get(format!("{base_url}/api/workflows/{wf_id}"))
        .send()
        .await
        .expect("get deleted workflow");
    let get_val: serde_json::Value = get_resp.json().await.expect("parse get response");
    assert_eq!(get_val["error"], "workflow not found");
}

// ---------------------------------------------------------------------------
// API-6: GET /api/workflows/{id}/graph — workflow graph topology
// ---------------------------------------------------------------------------
#[tokio::test]
async fn api_06_get_graph() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "name": "graph-test",
        "definition": {
            "nodes": [{"id": "fetch"}, {"id": "validate"}],
            "edges": [{"from": "fetch", "to": "validate"}],
            "dataflows": [{"from": "fetch", "to": "validate", "alias": "data"}]
        }
    });
    let create_resp = client
        .post(format!("{base_url}/api/workflows"))
        .json(&body)
        .send()
        .await
        .expect("create workflow for graph");
    let create_val: serde_json::Value = create_resp.json().await.expect("parse create response");
    let wf_id = create_val["id"].as_str().expect("id present");

    let resp = client
        .get(format!("{base_url}/api/workflows/{wf_id}/graph"))
        .send()
        .await
        .expect("get graph");
    assert_eq!(resp.status(), 200);
    let val: serde_json::Value = resp.json().await.expect("parse graph response");
    assert!(val["nodes"].is_array(), "nodes should be an array");
    assert!(val["edges"].is_array(), "edges should be an array");
    assert!(val["dataflows"].is_array(), "dataflows should be an array");
    assert_eq!(val["nodes"][0]["id"], "fetch");
    assert_eq!(val["edges"][0]["from"], "fetch");
    assert_eq!(val["edges"][0]["to"], "validate");
}

// ---------------------------------------------------------------------------
// API-7: GET /api/workflows/{id}/run — list runs for a workflow
// ---------------------------------------------------------------------------
#[tokio::test]
async fn api_07_list_runs_for_workflow() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    let wf_id = create_workflow(&client, &base_url, "list-runs-test").await;

    let resp = client
        .get(format!("{base_url}/api/workflows/{wf_id}/run"))
        .send()
        .await
        .expect("list runs for workflow");
    assert_eq!(resp.status(), 200);
    let val: serde_json::Value = resp.json().await.expect("parse list runs response");
    assert!(val.is_array(), "response should be an array");
    // Fresh workflow should have no runs
    assert!(val.as_array().unwrap_or(&vec![]).is_empty(), "new workflow has no runs");
}

// ---------------------------------------------------------------------------
// API-8: POST /api/workflows/{id}/run — trigger a workflow run
// ---------------------------------------------------------------------------
#[tokio::test]
async fn api_08_trigger_run() {
    let (base_url, store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    // Insert a simple workflow directly so trigger has something to find
    let wf_id = uuid::Uuid::new_v4().to_string();
    store
        .create_workflow(&wf_id, "trigger-test", r#"{"nodes":[],"edges":[]}"#)
        .expect("create workflow in store");

    let resp = client
        .post(format!("{base_url}/api/workflows/{wf_id}/run"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("trigger run");
    assert_eq!(resp.status(), 202, "trigger should return 202 Accepted");
    let val: serde_json::Value = resp.json().await.expect("parse trigger response");
    assert_eq!(val["status"], "accepted");
    assert!(val["run_id"].as_str().is_some(), "should return a run_id");
}

// ---------------------------------------------------------------------------
// API-9: GET /api/runs — list all runs
// ---------------------------------------------------------------------------
#[tokio::test]
async fn api_09_list_all_runs() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base_url}/api/runs"))
        .send()
        .await
        .expect("list all runs");
    assert_eq!(resp.status(), 200);
    let val: serde_json::Value = resp.json().await.expect("parse list runs response");
    assert!(val.is_array());
}

// ---------------------------------------------------------------------------
// API-10: GET /api/runs/{run_id} — get run by id
// ---------------------------------------------------------------------------
#[tokio::test]
async fn api_10_get_run_by_id() {
    let (base_url, store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    // Insert a workflow and a run directly
    let wf_id = uuid::Uuid::new_v4().to_string();
    store
        .create_workflow(&wf_id, "run-get-test", r#"{"nodes":[],"edges":[]}"#)
        .expect("create workflow");

    let run_id = uuid::Uuid::new_v4().to_string();
    store.create_run(&run_id, &wf_id).expect("create run");

    let resp = client
        .get(format!("{base_url}/api/runs/{run_id}"))
        .send()
        .await
        .expect("get run");
    assert_eq!(resp.status(), 200);
    let val: serde_json::Value = resp.json().await.expect("parse get run response");
    assert_eq!(val["run_id"], run_id);
    assert_eq!(val["workflow_id"], wf_id);
}

// ---------------------------------------------------------------------------
// API-11: POST /api/runs/{run_id}/stop — stop a running workflow
// ---------------------------------------------------------------------------
#[tokio::test]
async fn api_11_stop_run() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    let run_id = uuid::Uuid::new_v4().to_string();
    let resp = client
        .post(format!("{base_url}/api/runs/{run_id}/stop"))
        .send()
        .await
        .expect("stop run");
    assert_eq!(resp.status(), 404);
    let val: serde_json::Value = resp.json().await.expect("parse stop response");
    assert_eq!(val["run_id"], run_id);
    assert!(val["error"].as_str().is_some());
}

// ---------------------------------------------------------------------------
// API-12: GET /api/runs/{run_id}/graph — live graph status for a run
// ---------------------------------------------------------------------------
#[tokio::test]
async fn api_12_run_graph_status() {
    let (base_url, store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    // Create a workflow and run so graph has source data
    let wf_id = uuid::Uuid::new_v4().to_string();
    store
        .create_workflow(
            &wf_id,
            "graph-status-test",
            r#"{"nodes":[{"id":"A"},{"id":"B"}],"edges":[{"from":"A","to":"B"}]}"#,
        )
        .expect("create workflow");

    let run_id = uuid::Uuid::new_v4().to_string();
    store.create_run(&run_id, &wf_id).expect("create run");

    let resp = client
        .get(format!("{base_url}/api/runs/{run_id}/graph"))
        .send()
        .await
        .expect("get run graph");
    assert_eq!(resp.status(), 501);
    let val: serde_json::Value = resp.json().await.expect("parse run graph response");
    assert_eq!(val["run_id"], run_id);
    assert!(val["error"].as_str().is_some());
}

// ---------------------------------------------------------------------------
// API-13: GET /api/workflows (empty list when no workflows exist)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn api_13_list_empty() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base_url}/api/workflows"))
        .send()
        .await
        .expect("list workflows");
    assert_eq!(resp.status(), 200);
    let val: serde_json::Value = resp.json().await.expect("parse list response");
    assert!(val.is_array());
    assert!(val.as_array().unwrap_or(&vec![]).is_empty(), "should be empty");
}

// ---------------------------------------------------------------------------
// API-14: POST /api/workflows — multiple creates produce distinct ids
// ---------------------------------------------------------------------------
#[tokio::test]
async fn api_14_create_multiple() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "name": "dup-name",
        "definition": {"nodes": [], "edges": []}
    });

    let res1 = client
        .post(format!("{base_url}/api/workflows"))
        .json(&body)
        .send()
        .await
        .expect("first create");
    let val1: serde_json::Value = res1.json().await.expect("parse");
    let id1 = val1["id"].as_str().expect("id").to_string();

    let res2 = client
        .post(format!("{base_url}/api/workflows"))
        .json(&body)
        .send()
        .await
        .expect("second create");
    let val2: serde_json::Value = res2.json().await.expect("parse");
    let id2 = val2["id"].as_str().expect("id").to_string();

    // Each create should produce a unique id
    assert_ne!(id1, id2, "two creates should yield different ids");

    // Both listed
    let list_resp = client
        .get(format!("{base_url}/api/workflows"))
        .send()
        .await
        .expect("list");
    let list: serde_json::Value = list_resp.json().await.expect("parse list");
    assert_eq!(list.as_array().map_or(0, std::vec::Vec::len), 2, "should have 2 workflows");
}

// ---------------------------------------------------------------------------
// API-15: GET /api/workflows/{id} (not found)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn api_15_get_not_found() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base_url}/api/workflows/nonexistent-id"))
        .send()
        .await
        .expect("get nonexistent workflow");
    assert_eq!(resp.status(), 200);
    let val: serde_json::Value = resp.json().await.expect("parse response");
    assert_eq!(val["error"], "workflow not found");
}

// ---------------------------------------------------------------------------
// API-16: PUT /api/workflows/{id} (not found)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn api_16_update_not_found() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "name": "ghost",
        "definition": {"nodes": [], "edges": []}
    });
    let resp = client
        .put(format!("{base_url}/api/workflows/nonexistent-id"))
        .json(&body)
        .send()
        .await
        .expect("update nonexistent workflow");
    assert_eq!(resp.status(), 200);
    let val: serde_json::Value = resp.json().await.expect("parse response");
    // SQLite UPDATE on missing id succeeds with 0 affected rows, handler returns "updated"
    assert_eq!(val["status"], "updated");
}

// ---------------------------------------------------------------------------
// API-17: DELETE /api/workflows/{id} (not found)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn api_17_delete_not_found() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .delete(format!("{base_url}/api/workflows/nonexistent-id"))
        .send()
        .await
        .expect("delete nonexistent workflow");
    assert_eq!(resp.status(), 200);
    let val: serde_json::Value = resp.json().await.expect("parse response");
    assert_eq!(val["status"], "deleted");
}

// ── API-18: Graph endpoint includes trigger + threshold ──────

#[tokio::test]
async fn api_18_graph_includes_trigger_and_threshold() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "name": "graph-edge-fields",
        "definition": {
            "nodes": [
                {"id":"A","providers":[{"type":"subprocess","command":"echo a"}],"process_timeout_secs":10},
                {"id":"B","providers":[{"type":"subprocess","command":"echo b"}],"process_timeout_secs":10}
            ],
            "edges": [
                {"from":"A","to":"B","trigger":"all","event":"complete","exit_reason":"ok","threshold":2}
            ]
        }
    });
    let resp = client.post(format!("{base_url}/api/workflows")).json(&body).send().await.expect("create");
    let created: serde_json::Value = resp.json().await.expect("parse");
    let id = created["id"].as_str().expect("id");

    let graph: serde_json::Value = client
        .get(format!("{base_url}/api/workflows/{id}/graph"))
        .send().await.expect("graph").json().await.expect("parse");
    let edges = graph["edges"].as_array().expect("edges");
    assert_eq!(edges.len(), 1);
    let e = &edges[0];
    assert_eq!(e["trigger"], "all");
    assert_eq!(e["threshold"], 2);
    assert!(e["label"].as_str().unwrap().starts_with("all/"));
}

// ── API-19: route_policy MaxDuration roundtrip ────────────────

#[tokio::test]
async fn api_19_route_policy_max_duration_roundtrip() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "name": "max-duration-rp",
        "definition": {
            "nodes": [{
                "id":"w","providers":[{"type":"subprocess","command":"echo x"}],
                "process_timeout_secs":60,
                "route_policy":{"type":"max_duration","max_secs":300,"then_route":"timeout_exit"}
            }],
            "edges":[]
        }
    });
    let resp = client.post(format!("{base_url}/api/workflows")).json(&body).send().await.expect("create");
    let id = resp.json::<serde_json::Value>().await.expect("parse")["id"].as_str().unwrap().to_string();

    let wf: serde_json::Value = client.get(format!("{base_url}/api/workflows/{id}"))
        .send().await.expect("get").json().await.expect("parse");
    let def = match &wf["definition"] {
        serde_json::Value::String(s) => serde_json::from_str(s).expect("parse def str"),
        other => other.clone(),
    };
    let rp = &def["nodes"][0]["route_policy"];
    assert_eq!(rp["type"], "max_duration");
    assert_eq!(rp["max_secs"], 300);
    assert_eq!(rp["then_route"], "timeout_exit");
}

// ── API-20: HTTP provider workflow Roundtrip ──────────────────

#[tokio::test]
async fn api_20_http_provider_workflow_roundtrip() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "name": "http-provider-test",
        "definition": {
            "nodes": [{
                "id":"call","providers":[{
                    "type":"http","url":"https://example.com/api",
                    "method":"POST","headers":{"X-Key":"v"},"body":"{}"
                }],"process_timeout_secs":15
            }],
            "edges":[]
        }
    });
    let resp = client.post(format!("{base_url}/api/workflows")).json(&body).send().await.expect("create");
    let id = resp.json::<serde_json::Value>().await.expect("parse")["id"].as_str().unwrap().to_string();

    let graph: serde_json::Value = client.get(format!("{base_url}/api/workflows/{id}/graph"))
        .send().await.expect("graph").json().await.expect("parse");
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 1);
}
