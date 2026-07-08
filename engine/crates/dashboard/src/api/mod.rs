pub mod runs;
pub mod workflows;

use axum::Router;

use crate::state::AppState;

/// Assemble all REST API routes under `/api`.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/workflows", axum::routing::get(workflows::list).post(workflows::create))
        .route(
            "/workflows/{id}",
            axum::routing::get(workflows::get_by_id)
                .put(workflows::update)
                .delete(workflows::delete),
        )
        .route("/workflows/{id}/graph", axum::routing::get(workflows::graph))
        .route("/workflows/{id}/run", axum::routing::get(runs::list).post(runs::trigger))
        .route("/runs", axum::routing::get(runs::list_all))
        .route("/runs/{run_id}", axum::routing::get(runs::get_by_id))
        .route("/runs/{run_id}/stop", axum::routing::post(runs::stop))
        .route("/runs/{run_id}/graph", axum::routing::get(runs::graph_status))
}
