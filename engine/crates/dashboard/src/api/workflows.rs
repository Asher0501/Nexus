use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Value};

use crate::db::Store;
use crate::models::WorkflowRow;

/// GET /api/workflows — list all workflows.
pub async fn list(State(store): State<Store>) -> Json<Vec<WorkflowRow>> {
    let store = store.clone();
    match tokio::task::spawn_blocking(move || store.list_workflows()).await {
        Ok(Ok(rows)) => Json(rows),
        Ok(Err(e)) => {
            tracing::error!("[Workflows] list failed: {e}");
            Json(vec![])
        }
        Err(join_err) => {
            tracing::error!("[Workflows] list spawn_blocking panicked: {join_err}");
            Json(vec![])
        }
    }
}

/// POST /api/workflows — create a new workflow.
pub async fn create(
    State(store): State<Store>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let id = uuid::Uuid::new_v4().to_string();
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("unnamed")
        .to_string();
    let definition = body
        .get("definition")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => serde_json::to_string(other).unwrap_or_else(|_| "{}".into()),
        })
        .unwrap_or_else(|| "{}".into());

    let store = store.clone();
    let id_clone = id.clone();
    match tokio::task::spawn_blocking(move || store.create_workflow(&id_clone, &name, &definition)).await {
        Ok(Ok(_)) => Json(json!({"id": id, "status": "created"})),
        Ok(Err(e)) => {
            tracing::error!("[Workflows] create failed: {e}");
            Json(json!({"error": e.to_string()}))
        }
        Err(join_err) => {
            tracing::error!("[Workflows] create spawn_blocking panicked: {join_err}");
            Json(json!({"error": "internal error"}))
        }
    }
}

/// GET /api/workflows/{id} — get workflow details.
pub async fn get_by_id(
    State(store): State<Store>,
    Path(id): Path<String>,
) -> Json<Value> {
    let store = store.clone();
    match tokio::task::spawn_blocking(move || store.get_workflow(&id)).await {
        Ok(Ok(Some(wf))) => Json(json!({
            "id": wf.id,
            "name": wf.name,
            "definition": wf.definition,
            "created_at": wf.created_at,
            "updated_at": wf.updated_at,
        })),
        Ok(Ok(None)) => Json(json!({"error": "workflow not found"})),
        Ok(Err(e)) => {
            tracing::error!("[Workflows] get_by_id failed: {e}");
            Json(json!({"error": e.to_string()}))
        }
        Err(join_err) => {
            tracing::error!("[Workflows] get_by_id spawn_blocking panicked: {join_err}");
            Json(json!({"error": "internal error"}))
        }
    }
}

/// PUT /api/workflows/{id} — update a workflow.
pub async fn update(
    State(store): State<Store>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("unnamed");
    let definition = body
        .get("definition")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => serde_json::to_string(other).unwrap_or_else(|_| "{}".into()),
        })
        .unwrap_or_else(|| "{}".into());

    let store = store.clone();
    let id_clone = id.clone();
    let name = name.to_string();
    let definition = definition.to_string();
    match tokio::task::spawn_blocking(move || store.update_workflow(&id_clone, &name, &definition)).await {
        Ok(Ok(_)) => Json(json!({"id": id, "status": "updated"})),
        Ok(Err(e)) => {
            tracing::error!("[Workflows] update failed: {e}");
            Json(json!({"error": e.to_string()}))
        }
        Err(join_err) => {
            tracing::error!("[Workflows] update spawn_blocking panicked: {join_err}");
            Json(json!({"error": "internal error"}))
        }
    }
}

/// DELETE /api/workflows/{id} — delete a workflow.
pub async fn delete(
    State(store): State<Store>,
    Path(id): Path<String>,
) -> Json<Value> {
    let store = store.clone();
    let id_clone = id.clone();
    match tokio::task::spawn_blocking(move || store.delete_workflow(&id_clone)).await {
        Ok(Ok(_)) => Json(json!({"id": id, "status": "deleted"})),
        Ok(Err(e)) => {
            tracing::error!("[Workflows] delete failed: {e}");
            Json(json!({"error": e.to_string()}))
        }
        Err(join_err) => {
            tracing::error!("[Workflows] delete spawn_blocking panicked: {join_err}");
            Json(json!({"error": "internal error"}))
        }
    }
}

/// GET /api/workflows/{id}/graph — workflow DAG topology.
///
/// Parses the stored definition and returns the node/edge structure.
pub async fn graph(
    State(store): State<Store>,
    Path(id): Path<String>,
) -> Json<Value> {
    let store = store.clone();
    let wf = match tokio::task::spawn_blocking(move || store.get_workflow(&id)).await {
        Ok(Ok(Some(w))) => w,
        Ok(Ok(None)) => return Json(json!({"error": "workflow not found"})),
        Ok(Err(e)) => {
            tracing::error!("[Workflows] graph failed: {e}");
            return Json(json!({"error": e.to_string()}));
        }
        Err(join_err) => {
            tracing::error!("[Workflows] graph spawn_blocking panicked: {join_err}");
            return Json(json!({"error": "internal error"}));
        }
    };

    let parsed: Value = match serde_json::from_str(&wf.definition) {
        Ok(v) => v,
        Err(e) => return Json(json!({"error": format!("invalid definition: {e}")})),
    };

    let nodes = parsed
        .get("nodes")
        .map(|ns| {
            ns.as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|n| {
                            json!({
                                "id": n.get("id"),
                                "label": n.get("id"),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();

    let edges = parsed
        .get("edges")
        .map(|es| {
            es.as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|e| {
                            let label = format!(
                                "{}{}",
                                e.get("event").and_then(|v| v.as_str()).unwrap_or("complete"),
                                e.get("exit_reason")
                                    .and_then(|v| v.as_str())
                                    .filter(|s| !s.is_empty())
                                    .map(|r| format!("/{}", r))
                                    .unwrap_or_default()
                            );
                            json!({
                                "from": e.get("from"),
                                "to": e.get("to"),
                                "event": e.get("event"),
                                "exit_reason": e.get("exit_reason"),
                                "label": label,
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();

    let dataflows = parsed
        .get("dataflows")
        .map(|dfs| dfs.clone())
        .unwrap_or(json!([]));

    Json(json!({
        "nodes": nodes,
        "edges": edges,
        "dataflows": dataflows,
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
        Store::new(Some(std::env::temp_dir().join("nexus-test-workflows.db")))
            .expect("test db creation")
    });

    fn app() -> Router {
        Router::new()
            .route("/workflows", axum::routing::get(super::list).post(super::create))
            .route(
                "/workflows/{id}",
                axum::routing::get(super::get_by_id)
                    .put(super::update)
                    .delete(super::delete),
            )
            .route("/workflows/{id}/graph", axum::routing::get(super::graph))
            .with_state(STORE.clone())
    }

    #[tokio::test]
    async fn test_list_workflows() {
        let res = app()
            .oneshot(Request::get("/workflows").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        let body: Value =
            serde_json::from_slice(&axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert!(body.is_array());
    }

    #[tokio::test]
    async fn test_create_and_get_workflow() {
        let app = app();
        // Create
        let create_res = app
            .clone()
            .oneshot(
                Request::post("/workflows")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"name":"test-wf","definition":{"nodes":[],"edges":[]}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_res.status(), 200);
        let create_body: Value = serde_json::from_slice(
            &axum::body::to_bytes(create_res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        let id = create_body["id"].as_str().unwrap().to_string();

        // Get by id
        let get_res = app
            .oneshot(Request::get(&format!("/workflows/{id}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(get_res.status(), 200);
        let get_body: Value = serde_json::from_slice(
            &axum::body::to_bytes(get_res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(get_body["id"], id);
        assert_eq!(get_body["name"], "test-wf");
    }

    #[tokio::test]
    async fn test_get_graph() {
        let app = app();
        // Create a workflow with nodes and edges
        let create_res = app
            .clone()
            .oneshot(
                Request::post("/workflows")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"name":"graph-test","definition":{"nodes":[{"id":"fetch"},{"id":"validate"}],"edges":[{"from":"fetch","to":"validate"}]}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let create_body: Value = serde_json::from_slice(
            &axum::body::to_bytes(create_res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        let id = create_body["id"].as_str().unwrap();

        let res = app
            .oneshot(Request::get(&format!("/workflows/{id}/graph")).body(Body::empty()).unwrap())
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
        assert_eq!(body["nodes"][0]["id"], "fetch");
    }
}
