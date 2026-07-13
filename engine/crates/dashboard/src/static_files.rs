//! Static file serving fallback handler.
//!
//! Serves files from the directory specified by `NEXUS_STATIC_DIR`
//! (default `./static`). When a requested file is not found, falls back
//! to serving `index.html` for SPA-style client-side routing. If neither
//! exists, returns 404.
//!
//! This handler intentionally does not depend on `AppState` — it reads
//! the directory path from the environment on each request, making it
//! compatible with any axum router without state threading.

use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::Response;

/// Returns a MIME type string for common static file extensions.
fn mime_type(path: &str) -> &'static str {
    if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".js") {
        "application/javascript; charset=utf-8"
    } else if path.ends_with(".json") {
        "application/json; charset=utf-8"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".wasm") {
        "application/wasm"
    } else {
        "application/octet-stream"
    }
}

/// Fallback handler: serves static files from `NEXUS_STATIC_DIR`.
///
/// - If the requested path maps to an existing file → serve it.
/// - If not, try `index.html` (SPA fallback).
/// - If neither exists → 404.
pub async fn handler(uri: axum::http::Uri) -> Response<Body> {
    let base = std::env::var("NEXUS_STATIC_DIR").unwrap_or_else(|_| "./static".into());
    let req_path = uri.path().trim_start_matches('/');

    // Resolve the requested file path (prevent directory traversal).
    let file_path = std::path::Path::new(&base).join(if req_path.is_empty() { "index.html" } else { req_path });

    // Try the exact file first, then index.html fallback.
    for candidate in [file_path.as_path(), std::path::Path::new(&base).join("index.html").as_ref()] {
        if let Ok(content) = std::fs::read(candidate) {
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime_type(candidate.to_string_lossy().as_ref()))
                .body(Body::from(content))
                .unwrap();
        }
    }

    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from("Not Found"))
        .unwrap()
}
