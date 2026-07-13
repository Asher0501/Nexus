//! Scenario chain integration tests for nexus-dashboard.
//!
//! Each chain is a multi-step orthogonal operation sequence that exercises
//! real inter-action dependencies through the full HTTP API.
//!
//! Chains covered:
//!   Chain A  Create workflow → Get details → Get graph → Trigger run → Check run status
//!   Chain B  Create workflow → Trigger run → List runs → List workflows (exists)
//!   Chain C  Create multiple runs → List run history → Verify status labels
//!   Chain D  Create workflow with invalid def → Trigger run → Verify status tracking
//!   Chain E  Create workflow → Update → Verify update persisted → Delete → Verify deletion
//!   Chain F  Create workflow with edges → Get graph → Verify nodes/edges match definition

#![allow(missing_docs)]
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

async fn spawn_server() -> (String, Store, Arc<WsRoom>) {
    let dir = std::env::temp_dir().join(format!("nexus-int-chain-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).ok();
    let db_path = dir.join("test.db");

    let store = Store::new(Some(db_path)).expect("test db creation");
    let room = WsRoom::new();
    let state = AppState {
        store: store.clone(),
        room: room.clone(),
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
async fn create_workflow(client: &reqwest::Client, base_url: &str, name: &str, definition: &serde_json::Value) -> String {
    let body = serde_json::json!({
        "name": name,
        "definition": definition,
    });
    let resp = client
        .post(format!("{base_url}/api/workflows"))
        .json(&body)
        .send()
        .await
        .expect("create workflow");
    assert_eq!(resp.status(), 200);
    let val: serde_json::Value = resp.json().await.expect("parse create response");
    val["id"].as_str().expect("id present").to_string()
}

// ---------------------------------------------------------------------------
// Chain A: Create workflow → Get details → Get graph → Trigger run → Check run status
// ---------------------------------------------------------------------------
#[tokio::test]
async fn chain_a_create_get_graph_trigger_check() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    // Step 1: Create workflow
    let definition = serde_json::json!({
        "nodes": [{"id": "fetch"}, {"id": "process"}],
        "edges": [{"from": "fetch", "to": "process"}]
    });
    let wf_id = create_workflow(&client, &base_url, "chain-a", &definition).await;

    // Step 2: Get workflow details
    let resp = client
        .get(format!("{base_url}/api/workflows/{wf_id}"))
        .send()
        .await
        .expect("get workflow");
    assert_eq!(resp.status(), 200);
    let detail: serde_json::Value = resp.json().await.expect("parse detail");
    assert_eq!(detail["name"], "chain-a");
    assert_eq!(detail["id"], wf_id);

    // Step 3: Get graph
    let resp = client
        .get(format!("{base_url}/api/workflows/{wf_id}/graph"))
        .send()
        .await
        .expect("get graph");
    assert_eq!(resp.status(), 200);
    let graph: serde_json::Value = resp.json().await.expect("parse graph");
    let nodes = graph["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 2, "should have 2 nodes");

    // Step 4: Trigger run
    let resp = client
        .post(format!("{base_url}/api/workflows/{wf_id}/run"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("trigger run");
    assert_eq!(resp.status(), 202);
    let run: serde_json::Value = resp.json().await.expect("parse run response");
    let run_id = run["run_id"].as_str().expect("run_id").to_string();
    assert_eq!(run["status"], "accepted");

    // Step 5: Check run status via /api/runs/{run_id}
    let resp = client
        .get(format!("{base_url}/api/runs/{run_id}"))
        .send()
        .await
        .expect("get run");
    assert_eq!(resp.status(), 200);
    let run_status: serde_json::Value = resp.json().await.expect("parse run status");
    assert_eq!(run_status["run_id"], run_id);
    assert_eq!(run_status["workflow_id"], wf_id);
}

// ---------------------------------------------------------------------------
// Chain B: Create workflow → Trigger run → List runs → List workflows
// ---------------------------------------------------------------------------
#[tokio::test]
async fn chain_b_create_trigger_list_runs_list_workflows() {
    let (base_url, store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    // Step 1: Create workflow (direct store insert for reliable trigger)
    let wf_id = uuid::Uuid::new_v4().to_string();
    store
        .create_workflow(&wf_id, "chain-b", r#"{"nodes":[],"edges":[]}"#)
        .expect("create workflow");

    // Step 2: Trigger run
    let resp = client
        .post(format!("{base_url}/api/workflows/{wf_id}/run"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("trigger run");
    assert_eq!(resp.status(), 202);
    let run: serde_json::Value = resp.json().await.expect("parse run response");
    let run_id = run["run_id"].as_str().expect("run_id").to_string();

    // Step 3: List all runs — should contain the one we just triggered
    let resp = client
        .get(format!("{base_url}/api/runs"))
        .send()
        .await
        .expect("list runs");
    assert_eq!(resp.status(), 200);
    let runs: serde_json::Value = resp.json().await.expect("parse runs list");
    let runs_arr = runs.as_array().expect("runs array");
    let found = runs_arr.iter().any(|r| r["id"] == run_id);
    assert!(found, "triggered run should appear in /api/runs");

    // Step 4: List workflows — should contain ours
    let resp = client
        .get(format!("{base_url}/api/workflows"))
        .send()
        .await
        .expect("list workflows");
    assert_eq!(resp.status(), 200);
    let wfs: serde_json::Value = resp.json().await.expect("parse workflows list");
    let wfs_arr = wfs.as_array().expect("workflows array");
    let found_wf = wfs_arr.iter().any(|w| w["id"] == wf_id);
    assert!(found_wf, "workflow should appear in /api/workflows");
}

// ---------------------------------------------------------------------------
// Chain C: Create multiple runs → List run history → Verify status labels
// ---------------------------------------------------------------------------
#[tokio::test]
async fn chain_c_multiple_runs_status_labels() {
    let (base_url, store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    // Create a workflow
    let wf_id = uuid::Uuid::new_v4().to_string();
    store
        .create_workflow(&wf_id, "chain-c", r#"{"nodes":[],"edges":[]}"#)
        .expect("create workflow");

    // Create multiple runs directly in store with different statuses
    let run_ids: Vec<String> = (0..3)
        .map(|i| {
            let rid = uuid::Uuid::new_v4().to_string();
            store.create_run(&rid, &wf_id).expect("create run");
            // Finish some with specific statuses
            match i {
                0 => store.finish_run(&rid, "completed", None).expect("finish run"),
                1 => store.finish_run(&rid, "failed", None).expect("finish run"),
                _ => { /* leave running */ }
            }
            rid
        })
        .collect();

    // Trigger one more run via API
    let resp = client
        .post(format!("{base_url}/api/workflows/{wf_id}/run"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("trigger run");
    let api_run: serde_json::Value = resp.json().await.expect("parse trigger response");
    let api_run_id = api_run["run_id"].as_str().expect("run_id").to_string();

    // List all runs
    let resp = client
        .get(format!("{base_url}/api/runs"))
        .send()
        .await
        .expect("list runs");
    assert_eq!(resp.status(), 200);
    let runs: serde_json::Value = resp.json().await.expect("parse runs list");
    let runs_arr = runs.as_array().expect("runs array");

    // Should have at least 4 runs
    assert!(runs_arr.len() >= 4, "should have at least 4 runs, got {}", runs_arr.len());

    // Check status labels by fetching individual runs
    for rid in &run_ids {
        let resp = client
            .get(format!("{base_url}/api/runs/{rid}"))
            .send()
            .await
            .expect("get run");
        let run_detail: serde_json::Value = resp.json().await.expect("parse run detail");
        // Status should be one of the valid values
        let status = run_detail["status"].as_str().expect("status");
        assert!(
            ["running", "completed", "failed"].contains(&status),
            "unexpected status: {status}"
        );
    }

    // API-triggered run should exist
    let resp = client
        .get(format!("{base_url}/api/runs/{api_run_id}"))
        .send()
        .await
        .expect("get api-triggered run");
    assert_eq!(resp.status(), 200);
}

// ---------------------------------------------------------------------------
// Chain D: Create workflow → Trigger run → Verify run tracking
// ---------------------------------------------------------------------------
#[tokio::test]
async fn chain_d_trigger_then_verify_run_tracking() {
    let (base_url, store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    // Create a workflow
    let wf_id = uuid::Uuid::new_v4().to_string();
    store
        .create_workflow(&wf_id, "chain-d", r#"{"nodes":[],"edges":[]}"#)
        .expect("create workflow");

    // Trigger a run
    let resp = client
        .post(format!("{base_url}/api/workflows/{wf_id}/run"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("trigger run");
    assert_eq!(resp.status(), 202);
    let run: serde_json::Value = resp.json().await.expect("parse run");

    // The run_id is unique
    let run_id = run["run_id"].as_str().expect("run_id");
    assert!(!run_id.is_empty(), "run_id should not be empty");

    // Verify run tracking — the run should be listed under the workflow
    let resp = client
        .get(format!("{base_url}/api/workflows/{wf_id}/run"))
        .send()
        .await
        .expect("list runs for workflow");
    let runs: serde_json::Value = resp.json().await.expect("parse runs list");
    let runs_arr = runs.as_array().expect("runs array");
    let found = runs_arr.iter().any(|r| r["id"] == run_id);
    assert!(found, "run should appear under workflow's run list");

    // Verify graph status endpoint — returns 501 (not yet implemented)
    let resp = client
        .get(format!("{base_url}/api/runs/{run_id}/graph"))
        .send()
        .await
        .expect("get run graph status");
    assert_eq!(resp.status(), 501);
    let graph: serde_json::Value = resp.json().await.expect("parse graph status");
    assert_eq!(graph["run_id"], run_id);
}

// ---------------------------------------------------------------------------
// Chain E: Create workflow → Update → Verify update → Delete → Verify deletion
// ---------------------------------------------------------------------------
#[tokio::test]
async fn chain_e_create_update_delete() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    // Step 1: Create
    let definition = serde_json::json!({
        "nodes": [{"id": "initial"}],
        "edges": []
    });
    let wf_id = create_workflow(&client, &base_url, "chain-e", &definition).await;

    // Step 2: Update
    let updated_def = serde_json::json!({
        "nodes": [{"id": "initial"}, {"id": "added"}],
        "edges": [{"from": "initial", "to": "added"}]
    });
    let resp = client
        .put(format!("{base_url}/api/workflows/{wf_id}"))
        .json(&serde_json::json!({
            "name": "chain-e-updated",
            "definition": updated_def,
        }))
        .send()
        .await
        .expect("update workflow");
    assert_eq!(resp.status(), 200);

    // Step 3: Verify update persisted
    let resp = client
        .get(format!("{base_url}/api/workflows/{wf_id}"))
        .send()
        .await
        .expect("get workflow");
    let detail: serde_json::Value = resp.json().await.expect("parse detail");
    assert_eq!(detail["name"], "chain-e-updated");

    // Step 4: Delete
    let resp = client
        .delete(format!("{base_url}/api/workflows/{wf_id}"))
        .send()
        .await
        .expect("delete workflow");
    assert_eq!(resp.status(), 200);

    // Step 5: Verify deletion
    let resp = client
        .get(format!("{base_url}/api/workflows/{wf_id}"))
        .send()
        .await
        .expect("get deleted workflow");
    let deleted: serde_json::Value = resp.json().await.expect("parse response");
    assert_eq!(deleted["error"], "workflow not found");
}

// ---------------------------------------------------------------------------
// Chain F: Create workflow with nodes/edges → Get graph → Verify structure
// ---------------------------------------------------------------------------
#[tokio::test]
async fn chain_f_graph_structure_verification() {
    let (base_url, _store, _room) = spawn_server().await;
    let client = reqwest::Client::new();

    // Create workflow with specific topology
    let definition = serde_json::json!({
        "nodes": [
            {"id": "ingest", "providers": [{"type": "subprocess", "command": "cat"}]},
            {"id": "validate"},
            {"id": "transform"},
            {"id": "export"}
        ],
        "edges": [
            {"from": "ingest", "to": "validate", "trigger": "all", "event": "complete"},
            {"from": "validate", "to": "transform", "trigger": "all", "event": "complete"},
            {"from": "transform", "to": "export", "trigger": "all", "event": "complete"}
        ],
        "dataflows": [
            {"from": "ingest", "to": "validate", "alias": "raw_data"}
        ]
    });
    let wf_id = create_workflow(&client, &base_url, "chain-f", &definition).await;

    // Get graph via API
    let resp = client
        .get(format!("{base_url}/api/workflows/{wf_id}/graph"))
        .send()
        .await
        .expect("get graph");
    assert_eq!(resp.status(), 200);
    let graph: serde_json::Value = resp.json().await.expect("parse graph");

    // Verify nodes match definition
    let nodes = graph["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 4, "should have 4 nodes");

    let node_ids: Vec<&str> = nodes.iter().map(|n| n["id"].as_str().unwrap_or("?")).collect();
    assert!(node_ids.contains(&"ingest"), "should contain 'ingest'");
    assert!(node_ids.contains(&"validate"), "should contain 'validate'");
    assert!(node_ids.contains(&"transform"), "should contain 'transform'");
    assert!(node_ids.contains(&"export"), "should contain 'export'");

    // Verify edges match definition
    let edges = graph["edges"].as_array().expect("edges array");
    assert_eq!(edges.len(), 3, "should have 3 edges");

    let edge_pairs: Vec<(&str, &str)> = edges
        .iter()
        .map(|e| {
            (
                e["from"].as_str().unwrap_or("?"),
                e["to"].as_str().unwrap_or("?"),
            )
        })
        .collect();
    assert!(edge_pairs.contains(&("ingest", "validate")));
    assert!(edge_pairs.contains(&("validate", "transform")));
    assert!(edge_pairs.contains(&("transform", "export")));

    // Verify dataflows
    let dataflows = graph["dataflows"].as_array().expect("dataflows array");
    assert_eq!(dataflows.len(), 1, "should have 1 dataflow");
    assert_eq!(dataflows[0]["from"], "ingest");
    assert_eq!(dataflows[0]["to"], "validate");
}
