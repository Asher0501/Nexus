use std::sync::Arc;

use axum::extract::{Path, State};
use axum::{http::StatusCode, Json};
use serde_json::{json, Value};

use crate::db::Store;
use crate::engine_bridge;
use crate::models::RunRow;
use crate::ws::WsRoom;
use nexus_engine::model::EngineConfig;

/// GET /api/workflows/{id}/run — list runs for a workflow.
pub async fn list(
    State(store): State<Store>,
    Path(wf_id): Path<String>,
) -> Json<Vec<RunRow>> {
    match store.list_runs_for_workflow(&wf_id) {
        Ok(rows) => Json(rows),
        Err(e) => {
            tracing::error!("[Runs] list failed: {e}");
            Json(vec![])
        }
    }
}

/// POST /api/workflows/{id}/run — trigger a workflow run.
pub async fn trigger(
    State(store): State<Store>,
    State(room): State<Arc<WsRoom>>,
    Path(wf_id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let wf = match store.get_workflow(&wf_id) {
        Ok(Some(w)) => w,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "workflow not found"})),
            );
        }
        Err(e) => {
            tracing::error!("[Runs] trigger get_workflow failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            );
        }
    };

    let run_id = uuid::Uuid::new_v4().to_string();
    if let Err(e) = store.create_run(&run_id, &wf_id) {
        tracing::error!("[Runs] trigger create_run failed: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        );
    }

    // Run the workflow in the background — does not block the HTTP response.
    let def = wf.definition.clone();
    let store_clone = store.clone();
    let room_clone = room.clone();
    let run_id_clone = run_id.clone();
    tokio::spawn(async move {
        let config = EngineConfig::default();
        match engine_bridge::run_workflow(&def, config, room_clone, &run_id_clone).await {
            Ok(_) => {
                if let Err(e) = store_clone.finish_run(&run_id_clone, "completed", None) {
                    tracing::error!("[Runs] finish_run failed: {e}");
                }
            }
            Err(e) => {
                tracing::error!("[Runs] workflow execution failed: {e}");
                if let Err(e2) = store_clone.finish_run(&run_id_clone, "failed", None) {
                    tracing::error!("[Runs] finish_run (failed) failed: {e2}");
                }
            }
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(json!({"run_id": run_id, "status": "accepted"})),
    )
}

/// GET /api/runs — list all runs.
pub async fn list_all(State(store): State<Store>) -> Json<Vec<RunRow>> {
    match store.list_runs() {
        Ok(rows) => Json(rows),
        Err(e) => {
            tracing::error!("[Runs] list_all failed: {e}");
            Json(vec![])
        }
    }
}

/// GET /api/runs/{run_id} — get run details.
pub async fn get_by_id(
    State(store): State<Store>,
    Path(run_id): Path<String>,
) -> Json<Value> {
    match store.get_run(&run_id) {
        Ok(Some(run)) => Json(json!({
            "run_id": run.id,
            "workflow_id": run.workflow_id,
            "status": run.status,
            "started_at": run.started_at,
            "finished_at": run.finished_at,
        })),
        Ok(None) => Json(json!({"error": "run not found"})),
        Err(e) => {
            tracing::error!("[Runs] get_by_id failed: {e}");
            Json(json!({"error": e.to_string()}))
        }
    }
}

/// POST /api/runs/{run_id}/stop — stop a running workflow.
pub async fn stop(
    State(_store): State<Store>,
    Path(run_id): Path<String>,
) -> Json<Value> {
    // TODO(#future): implement actual running-node cancellation via engine handle.
    Json(json!({"run_id": run_id, "status": "stopping"}))
}

/// GET /api/runs/{run_id}/graph — live graph status for a run.
pub async fn graph_status(
    Path(run_id): Path<String>,
) -> Json<Value> {
    Json(json!({
        "run_id": run_id,
        "nodes": [],
        "edges": []
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use std::sync::LazyLock;
    use tower::ServiceExt;
    use axum::body::Body;
    use axum::http::Request;

    static STORE: LazyLock<Store> = LazyLock::new(|| {
        Store::new(Some(std::env::temp_dir().join("nexus-test-runs.db")))
            .expect("test db creation")
    });

    fn app() -> Router {
        let room = WsRoom::new();
        let state = crate::state::AppState {
            store: STORE.clone(),
            room,
        };
        Router::new()
            .route("/workflows/{id}/run", axum::routing::get(super::list).post(super::trigger))
            .route("/runs", axum::routing::get(super::list_all))
            .route("/runs/{run_id}", axum::routing::get(super::get_by_id))
            .route("/runs/{run_id}/stop", axum::routing::post(super::stop))
            .route("/runs/{run_id}/graph", axum::routing::get(super::graph_status))
            .with_state(state)
    }

    #[tokio::test]
    async fn test_trigger_run() {
        // Insert a workflow directly into the store so the trigger has something to find.
        let wf_id = uuid::Uuid::new_v4().to_string();
        STORE
            .create_workflow(&wf_id, "trigger-test", r#"{"nodes":[],"edges":[]}"#)
            .expect("create workflow");

        let app = app();
        let res = app
            .oneshot(
                Request::post(&format!("/workflows/{wf_id}/run"))
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 202);
        let body: Value = serde_json::from_slice(
            &axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["status"], "accepted");
        assert!(body["run_id"].as_str().is_some());
    }

    #[tokio::test]
    async fn test_get_run_by_id() {
        // Insert a workflow directly into the store, then trigger a run.
        let wf_id = uuid::Uuid::new_v4().to_string();
        STORE
            .create_workflow(&wf_id, "get-test", r#"{"nodes":[],"edges":[]}"#)
            .expect("create workflow");

        let run_id = uuid::Uuid::new_v4().to_string();
        STORE.create_run(&run_id, &wf_id).expect("create run");

        let app = app();
        let res = app
            .oneshot(Request::get(&format!("/runs/{run_id}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        let body: Value = serde_json::from_slice(
            &axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["run_id"], run_id);
        assert!(body["workflow_id"].as_str().is_some());
    }

    #[tokio::test]
    async fn test_graph_status() {
        let app = app();
        let res = app
            .oneshot(Request::get("/runs/test-run-1/graph").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        let body: Value = serde_json::from_slice(
            &axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(body["nodes"].is_array());
        assert!(body["edges"].is_array());
    }
}
