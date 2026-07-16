use axum::extract::{Path, State};
use axum::{http::StatusCode, Json};
use serde_json::{json, Value};

use crate::db::Store;
use crate::engine_bridge;
use crate::models::RunRow;
#[cfg(test)]
use crate::ws::WsRoom;
use nexus_engine::model::EngineConfig;

/// GET /api/workflows/{id}/run — list runs for a workflow.
pub async fn list(
    State(store): State<Store>,
    Path(wf_id): Path<String>,
) -> Json<Vec<RunRow>> {
    let store = store.clone();
    match tokio::task::spawn_blocking(move || store.list_runs_for_workflow(&wf_id)).await {
        Ok(Ok(rows)) => Json(rows),
        Ok(Err(e)) => {
            tracing::error!("[Runs] list failed: {e}");
            Json(vec![])
        }
        Err(join_err) => {
            tracing::error!("[Runs] list spawn_blocking panicked: {join_err}");
            Json(vec![])
        }
    }
}

/// POST /api/workflows/{id}/run — trigger a workflow run.
///
/// Optional JSON body: `{"max_concurrency": N}` to override the default concurrency.
pub async fn trigger(
    State(state): State<crate::state::AppState>,
    Path(wf_id): Path<String>,
    body: Option<Json<Value>>,
) -> (StatusCode, Json<Value>) {
    let store = state.store.clone();
    let room = state.room.clone();
    let store_for_wf = store.clone();
    let wf_id_for_wf = wf_id.clone();
    let wf = match tokio::task::spawn_blocking(move || store_for_wf.get_workflow(&wf_id_for_wf)).await {
        Ok(Ok(Some(w))) => w,
        Ok(Ok(None)) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "workflow not found"})),
            );
        }
        Ok(Err(e)) => {
            tracing::error!("[Runs] trigger get_workflow failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            );
        }
        Err(join_err) => {
            tracing::error!("[Runs] trigger get_workflow spawn_blocking panicked: {join_err}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "internal error"})),
            );
        }
    };

    let run_id = uuid::Uuid::new_v4().to_string();
    let store_for_create = store.clone();
    let run_id_clone = run_id.clone();
    let wf_id_clone = wf_id.clone();
    match tokio::task::spawn_blocking(move || store_for_create.create_run(&run_id_clone, &wf_id_clone)).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            tracing::error!("[Runs] trigger create_run failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            );
        }
        Err(join_err) => {
            tracing::error!("[Runs] trigger create_run spawn_blocking panicked: {join_err}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "internal error"})),
            );
        }
    }

    // Register cancel flag before spawning.
    let cancel_flag = state.register_cancel(&run_id);

    // Run the workflow in the background — does not block the HTTP response.
    let def = wf.definition.clone();
    let store_clone = store.clone();
    let room_clone = room.clone();
    let run_id_clone = run_id.clone();
    let max_concurrency = body
        .and_then(|b| b.get("max_concurrency").and_then(serde_json::Value::as_u64))
        .map(|n| n as usize);
    tokio::spawn(async move {
        let config = EngineConfig::new(max_concurrency, 3600, 3);
        match engine_bridge::run_workflow(&def, config, room_clone, &run_id_clone, Some(cancel_flag)).await {
            Ok(()) => {
                if let Err(e) = store_clone.finish_run(&run_id_clone, "completed", None) {
                    tracing::error!("[Runs] finish_run failed: {e}");
                }
            }
            Err(e) => {
                tracing::error!("[Runs] workflow execution failed: {e}");
                if let Err(e2) = store_clone.finish_run(&run_id_clone, "failed", Some(&e)) {
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
    let store = store.clone();
    match tokio::task::spawn_blocking(move || store.list_runs()).await {
        Ok(Ok(rows)) => Json(rows),
        Ok(Err(e)) => {
            tracing::error!("[Runs] list_all failed: {e}");
            Json(vec![])
        }
        Err(join_err) => {
            tracing::error!("[Runs] list_all spawn_blocking panicked: {join_err}");
            Json(vec![])
        }
    }
}

/// GET /`api/runs/{run_id`} — get run details.
pub async fn get_by_id(
    State(store): State<Store>,
    Path(run_id): Path<String>,
) -> Json<Value> {
    let store = store.clone();
    match tokio::task::spawn_blocking(move || store.get_run(&run_id)).await {
        Ok(Ok(Some(run))) => Json(json!({
            "run_id": run.id,
            "workflow_id": run.workflow_id,
            "status": run.status,
            "started_at": run.started_at,
            "finished_at": run.finished_at,
        })),
        Ok(Ok(None)) => Json(json!({"error": "run not found"})),
        Ok(Err(e)) => {
            tracing::error!("[Runs] get_by_id failed: {e}");
            Json(json!({"error": e.to_string()}))
        }
        Err(join_err) => {
            tracing::error!("[Runs] get_by_id spawn_blocking panicked: {join_err}");
            Json(json!({"error": "internal error"}))
        }
    }
}

/// POST /`api/runs/{run_id}/stop` — stop a running workflow.
pub async fn stop(
    State(state): State<crate::state::AppState>,
    Path(run_id): Path<String>,
) -> (StatusCode, Json<Value>) {
    if state.cancel_run(&run_id) {
        (
            StatusCode::OK,
            Json(json!({"run_id": run_id, "status": "stopping"})),
        )
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"run_id": run_id, "error": "run not found or already completed"})),
        )
    }
}

/// GET /`api/runs/{run_id}/graph` — live graph status for a run.
///
/// Not yet implemented — requires the engine to expose a run-id-keyed
/// graph snapshot.  Returns 501 Not Implemented.
pub async fn graph_status(
    Path(run_id): Path<String>,
) -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({"run_id": run_id, "error": "live graph is not yet implemented"})),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

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
            cancel_flags: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
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
    async fn test_graph_status_returns_501() {
        let app = app();
        let res = app
            .oneshot(Request::get("/runs/test-run-1/graph").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_IMPLEMENTED);
        let body: Value = serde_json::from_slice(
            &axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(body["error"].as_str().is_some());
    }
}
