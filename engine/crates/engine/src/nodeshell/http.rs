//! HTTP executor — makes HTTP requests and maps responses to the Nexus
//! node protocol (`{route, content}`).
//!
//! # Route resolution (in priority order)
//!
//! 1. If the response body is valid JSON with a `route` field → use it.
//! 2. Otherwise, map the HTTP status code:
//!    - 2xx → `"ok"`
//!    - 4xx → `"client_error"`
//!    - 5xx → `"server_error"`
//!    - everything else → `"unknown"`
//!
//! # Content
//!
//! `content` is always the UTF-8 response body.  If the body is not valid
//! UTF-8 it is lossy-decoded.

use std::collections::HashMap;
use std::time::Duration;

use crate::model::provider::ProviderDef;
use crate::nodeshell::template::TemplateEngine;
use crate::nodeshell::{NodeChunk, NodeContext, NodeOutcome, NodeOutput, SpawnError};

/// Executes an HTTP request and maps the response to `{route, content}`.
#[derive(Debug, Clone)]
pub struct HttpExecutor {
    /// Full URL (after template rendering).
    url: String,
    /// HTTP method (GET, POST, PUT, DELETE, PATCH).  Defaults to GET.
    method: String,
    /// Optional headers.
    headers: HashMap<String, String>,
    /// Optional request body (string; Content-Type defaults to application/json).
    body: Option<String>,
    /// Scripts directory for template rendering.
    scripts_dir: String,
}

impl HttpExecutor {
    /// Create an executor from a [`ProviderDef::Http`].
    #[must_use]
    pub fn from_provider(def: &ProviderDef, scripts_dir: &std::path::Path) -> Self {
        match def {
            ProviderDef::Http {
                url,
                method,
                headers,
                body,
            } => Self {
                url: url.clone(),
                method: method
                    .clone().map_or_else(|| "GET".into(), |m| m.to_uppercase()),
                headers: headers.clone().unwrap_or_default(),
                body: body.clone(),
                scripts_dir: scripts_dir.to_string_lossy().to_string(),
            },
            _ => unreachable!("HttpExecutor::from_provider called on non-Http provider"),
        }
    }

    /// Run the HTTP request.
    pub async fn run(
        &self,
        ctx: NodeContext,
        timeout: Duration,
        _node_id: &str,
        chunk_tx: Option<tokio::sync::mpsc::Sender<NodeChunk>>,
    ) -> Result<NodeOutcome, SpawnError> {
        // Render templates in URL.
        let url = TemplateEngine::render(
            &self.url,
            &ctx.metadata,
            &ctx.upstream,
            &self.scripts_dir,
        );

        // Render templates in body.
        let body = self.body.as_ref().map(|b| {
            TemplateEngine::render(b, &ctx.metadata, &ctx.upstream, &self.scripts_dir)
        });

        // Build the request.
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| SpawnError {
                message: format!("failed to create HTTP client: {e}"),
            })?;

        let mut req = match self.method.as_str() {
            "GET" => client.get(&url),
            "POST" => client.post(&url),
            "PUT" => client.put(&url),
            "DELETE" => client.delete(&url),
            "PATCH" => client.patch(&url),
            other => {
                return Err(SpawnError {
                    message: format!("unsupported HTTP method: {other}"),
                });
            }
        };

        // Attach headers.
        for (k, v) in &self.headers {
            req = req.header(k.as_str(), v.as_str());
        }

        // Attach body for methods that support it.
        if let Some(ref b) = body {
            req = req.header("Content-Type", "application/json").body(b.clone());
        }

        // Notify the chunk consumer (one "sending request" chunk for
        // observability).
        if let Some(ref tx) = chunk_tx {
            let _ = tx
                .send(NodeChunk {
                    text: format!("[HTTP] {method} {url}", method = self.method, url = url),
                })
                .await;
        }

        // Execute the request.
        let response = req.send().await.map_err(|e| {
            // Distinguish timeout from other errors.
            let msg = if e.is_timeout() {
                format!("HTTP request timed out after {timeout:?}: {e}")
            } else {
                format!("HTTP request failed: {e}")
            };
            SpawnError { message: msg }
        })?;

        let status = response.status().as_u16();
        let response_body = response
            .text()
            .await
            .unwrap_or_else(|e| format!("[error reading response body: {e}]"));

        // Notify chunk consumer with status.
        if let Some(ref tx) = chunk_tx {
            let _ = tx
                .send(NodeChunk {
                    text: format!("[HTTP] {status} ({len} bytes)", len = response_body.len()),
                })
                .await;
        }

        // Resolve route.
        let route = Self::resolve_route(status, &response_body);

        // Resolve exit_code: 0 for 2xx, 1 for non-2xx (so Failed edges can fire).
        let exit_code = i32::from(!(200..300).contains(&status));

        Ok(NodeOutcome {
            output: NodeOutput {
                route: route.clone(),
                content: response_body,
            },
            exit_code,
            exit_reason: Some(route),
        })
    }

    /// Resolve the `route` value from an HTTP response.
    ///
    /// 1. Try to parse the body as JSON and extract a `route` field.
    /// 2. Fall back to status-code-based routing.
    fn resolve_route(status: u16, body: &str) -> String {
        // Try JSON route extraction.
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(body)
            && let Some(route) = json.get("route").and_then(|v| v.as_str())
                && !route.is_empty() {
                    return route.to_string();
                }

        // Fall back to status-code → route mapping.
        match status {
            200..=299 => "ok".to_string(),
            400..=499 => "client_error".to_string(),
            500..=599 => "server_error".to_string(),
            _ => "unknown".to_string(),
        }
    }
}

// ── Embedded test HTTP server ─────────────────────────────
//
// Used by unit tests and integration tests to exercise every
// scenario without an external dependency.
//
// Spawns on `127.0.0.1:0` (OS-assigned port); call `.addr()`
// to get the actual address.

#[cfg(test)]
pub mod test_server {
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    /// A minimal HTTP test server that handles one connection at a time.
    ///
    /// # Route table (path-based)
    ///
    /// | Path | Method | Behavior |
    /// |------|--------|----------|
    /// | `/ok` | any | 200 `{"route":"ok","content":"all good"}` |
    /// | `/err` | any | 200 `{"route":"err","content":"something wrong"}` |
    /// | `/plain` | any | 200 `plain text response` |
    /// | `/echo` | POST | 200 `{"route":"echo","content":"<request body>"}` |
    /// | `/status/201` | any | 201 `{"route":"created","content":"..."}` |
    /// | `/status/400` | any | 400 `{"error":"bad request"}` |
    /// | `/status/500` | any | 500 `{"error":"internal"}` |
    /// | `/slow` | any | 200 after 3 s delay `{"route":"slow","content":"done"}` |
    /// | `/auth` | any | 200 if `Authorization: Bearer secret`, else 401 |
    /// | `/headers` | any | 200 with `X-Custom: echo` header |
    /// | anything else | any | 404 `{"error":"not found"}` |
    pub struct TestServer {
        addr: String,
        shutdown: Arc<AtomicBool>,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl TestServer {
        /// Start the test server on an OS-assigned port.
        ///
        /// Blocks until the server is actually listening to avoid race
        /// conditions between the listener thread and the test code.
        pub fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
            let addr = format!("http://{}", listener.local_addr().unwrap());
            let shutdown = Arc::new(AtomicBool::new(false));
            let shutdown_clone = shutdown.clone();

            // Signal that the server is ready.
            let (ready_tx, ready_rx) = std::sync::mpsc::channel();

            let handle = thread::spawn(move || {
                ready_tx.send(()).ok(); // signal readiness
                listener
                    .set_nonblocking(true)
                    .expect("set nonblocking");
                loop {
                    if shutdown_clone.load(Ordering::Relaxed) {
                        break;
                    }
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if shutdown_clone.load(Ordering::Relaxed) {
                                break;
                            }
                            handle_connection(stream);
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(50));
                        }
                        Err(_) => break,
                    }
                }
            });

            // Wait for the thread to actually start and bind.
            ready_rx.recv().ok();

            Self {
                addr,
                shutdown,
                handle: Some(handle),
            }
        }

        /// The base URL of the server (e.g. `http://127.0.0.1:54321`).
        pub fn addr(&self) -> &str {
            &self.addr
        }

        /// Build a full URL for a path.
        pub fn url(&self, path: &str) -> String {
            format!("{}{path}", self.addr)
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.shutdown.store(true, Ordering::Relaxed);
            if let Some(h) = self.handle.take() {
                // Give the thread a moment to see the shutdown flag.
                let _ = h.join();
            }
        }
    }

    fn handle_connection(mut stream: TcpStream) {
        let mut reader = BufReader::new(stream.try_clone().unwrap_or_else(|_| {
            panic!("clone stream")
        }));
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() {
            return;
        }

        // Parse: METHOD /path HTTP/1.1
        let parts: Vec<&str> = request_line.split_whitespace().collect();
        let method = parts.first().copied().unwrap_or("GET");
        let path = parts.get(1).copied().unwrap_or("/");

        // Read headers to find Content-Length.
        let mut content_length: usize = 0;
        let mut auth_header = String::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }
            if trimmed.to_lowercase().starts_with("content-length:") {
                content_length = trimmed
                    .split(':')
                    .nth(1)
                    .and_then(|s| s.trim().parse().ok())
                    .unwrap_or(0);
            }
            if trimmed.to_lowercase().starts_with("authorization:") {
                auth_header = trimmed.split(':').nth(1).unwrap_or("").trim().to_string();
            }
        }

        // Read body if present.
        let mut body = String::new();
        if content_length > 0 {
            let mut buf = vec![0u8; content_length];
            use std::io::Read;
            if reader.read_exact(&mut buf).is_ok() {
                body = String::from_utf8_lossy(&buf).to_string();
            }
        }

        // Route.
        let (status, response_body) = route_request(method, path, &body, &auth_header);

        // Special: delay for /slow.
        if path == "/slow" {
            thread::sleep(Duration::from_secs(3));
        }

        let response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Length: {len}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{response_body}",
            reason = status_reason(status),
            len = response_body.len(),
        );
        let _ = stream.write_all(response.as_bytes());
    }

    fn route_request(
        method: &str,
        path: &str,
        body: &str,
        auth_header: &str,
    ) -> (u16, String) {
        match path {
            "/ok" => (200, r#"{"route":"ok","content":"all good"}"#.into()),
            "/err" => (200, r#"{"route":"err","content":"something wrong"}"#.into()),
            "/plain" => (200, "plain text response".into()),
            "/echo" if method == "POST" => {
                let resp = serde_json::json!({"route":"echo","content": body});
                (200, resp.to_string())
            }
            "/status/201" => (201, r#"{"route":"created","content":"resource created"}"#.into()),
            "/status/400" => (400, r#"{"error":"bad request"}"#.into()),
            "/status/500" => (500, r#"{"error":"internal server error"}"#.into()),
            "/slow" => (200, r#"{"route":"slow","content":"done"}"#.into()),
            "/auth" => {
                if auth_header == "Bearer secret" {
                    (200, r#"{"route":"authenticated","content":"welcome"}"#.into())
                } else {
                    (401, r#"{"error":"unauthorized"}"#.into())
                }
            }
            "/headers" => (200, r#"{"route":"ok","content":"headers received"}"#.into()),
            _ => (404, r#"{"error":"not found"}"#.into()),
        }
    }

    fn status_reason(code: u16) -> &'static str {
        match code {
            200 => "OK",
            201 => "Created",
            400 => "Bad Request",
            401 => "Unauthorized",
            404 => "Not Found",
            500 => "Internal Server Error",
            _ => "Unknown",
        }
    }

    // ── Tests for the test server itself ──────────────────

    #[test]
    fn test_server_ok_returns_json_with_route() {
        let srv = TestServer::start();
        let resp = ureq::get(&srv.url("/ok")).call().unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        let body = resp.into_body().read_to_string().unwrap();
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["route"], "ok");
        assert_eq!(json["content"], "all good");
    }

    #[test]
    fn test_server_plain_returns_text() {
        let srv = TestServer::start();
        let resp = ureq::get(&srv.url("/plain")).call().unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        let body = resp.into_body().read_to_string().unwrap();
        assert_eq!(body, "plain text response");
    }

    #[test]
    fn test_server_echo_post() {
        let srv = TestServer::start();
        let resp = ureq::post(&srv.url("/echo"))
            .send(r#"{"hello":"world"}"#)
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        let body = resp.into_body().read_to_string().unwrap();
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["route"], "echo");
        assert!(json["content"].as_str().unwrap().contains("hello"));
    }

    #[test]
    fn test_server_error_status() {
        let srv = TestServer::start();
        let result = ureq::get(&srv.url("/status/500")).call();
        // ureq v3 returns Err(StatusCode(code)) for non-2xx.
        match result {
            Err(ureq::Error::StatusCode(code)) => assert_eq!(code, 500),
            Ok(resp) => assert_eq!(resp.status().as_u16(), 500),
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[test]
    fn test_server_not_found() {
        let srv = TestServer::start();
        let result = ureq::get(&srv.url("/nonexistent")).call();
        match result {
            Err(ureq::Error::StatusCode(code)) => assert_eq!(code, 404),
            Ok(resp) => assert_eq!(resp.status().as_u16(), 404),
            other => panic!("unexpected result: {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodeshell::NodeMetadata;
    use test_server::TestServer;

    /// Build a minimal `NodeContext` with no upstream data.
    fn empty_ctx() -> NodeContext {
        NodeContext {
            inputs: HashMap::new(),
            extensions: HashMap::new(),
            metadata: NodeMetadata {
                run_count: 1,
                timed_out: false,
            },
            upstream: HashMap::new(),
        }
    }

    // ── Unit tests: route resolution ────────────────────

    #[test]
    fn resolve_route_from_json() {
        let route = HttpExecutor::resolve_route(200, r#"{"route":"approved","content":"x"}"#);
        assert_eq!(route, "approved");
    }

    #[test]
    fn resolve_route_empty_route_field_falls_back() {
        // route="" → fall back to status-based
        let route = HttpExecutor::resolve_route(200, r#"{"route":"","content":"x"}"#);
        assert_eq!(route, "ok"); // 2xx fallback
    }

    #[test]
    fn resolve_route_missing_route_field_falls_back() {
        let route = HttpExecutor::resolve_route(200, r#"{"content":"x"}"#);
        assert_eq!(route, "ok");
    }

    #[test]
    fn resolve_route_2xx_status() {
        let route = HttpExecutor::resolve_route(200, "not json");
        assert_eq!(route, "ok");
    }

    #[test]
    fn resolve_route_4xx_status() {
        let route = HttpExecutor::resolve_route(404, "not found");
        assert_eq!(route, "client_error");
    }

    #[test]
    fn resolve_route_5xx_status() {
        let route = HttpExecutor::resolve_route(502, "bad gateway");
        assert_eq!(route, "server_error");
    }

    #[test]
    fn resolve_route_3xx_status() {
        // 3xx is not 2xx, 4xx, or 5xx → falls to "unknown"
        let route = HttpExecutor::resolve_route(302, "redirect");
        assert_eq!(route, "unknown");
    }

    // ── Unit tests: executor ────────────────────────────

    #[tokio::test]
    async fn http_executor_get_ok_returns_route_from_json() {
        let srv = TestServer::start();
        let def = ProviderDef::Http {
            url: srv.url("/ok"),
            method: Some("GET".into()),
            headers: None,
            body: None,
        };
        let exe = HttpExecutor::from_provider(&def, &std::path::PathBuf::from("."));
        let outcome = exe
            .run(empty_ctx(), Duration::from_secs(10), "test_node", None)
            .await
            .expect("request should succeed");

        assert_eq!(outcome.exit_code, 0);
        assert_eq!(outcome.output.route, "ok");
        assert!(outcome.output.content.contains("all good"));
    }

    #[tokio::test]
    async fn http_executor_get_plain_text_falls_back_to_status_route() {
        let srv = TestServer::start();
        let def = ProviderDef::Http {
            url: srv.url("/plain"),
            method: Some("GET".into()),
            headers: None,
            body: None,
        };
        let exe = HttpExecutor::from_provider(&def, &std::path::PathBuf::from("."));
        let outcome = exe
            .run(empty_ctx(), Duration::from_secs(10), "test_node", None)
            .await
            .expect("request should succeed");

        assert_eq!(outcome.exit_code, 0);
        assert_eq!(outcome.output.route, "ok"); // 2xx fallback
        assert_eq!(outcome.output.content, "plain text response");
    }

    #[tokio::test]
    async fn http_executor_post_echo() {
        let srv = TestServer::start();
        let def = ProviderDef::Http {
            url: srv.url("/echo"),
            method: Some("POST".into()),
            headers: None,
            body: Some(r#"{"payload":"test"}"#.into()),
        };
        let exe = HttpExecutor::from_provider(&def, &std::path::PathBuf::from("."));
        let outcome = exe
            .run(empty_ctx(), Duration::from_secs(10), "test_node", None)
            .await
            .expect("POST should succeed");

        assert_eq!(outcome.exit_code, 0);
        assert_eq!(outcome.output.route, "echo");
        assert!(outcome.output.content.contains("payload"));
    }

    #[tokio::test]
    async fn http_executor_500_status_maps_to_route_and_nonzero_exit() {
        let srv = TestServer::start();
        let def = ProviderDef::Http {
            url: srv.url("/status/500"),
            method: Some("GET".into()),
            headers: None,
            body: None,
        };
        let exe = HttpExecutor::from_provider(&def, &std::path::PathBuf::from("."));
        let outcome = exe
            .run(empty_ctx(), Duration::from_secs(10), "test_node", None)
            .await
            .expect("request should complete (server error is still a valid response)");

        assert_eq!(outcome.exit_code, 1); // non-2xx → exit 1
        assert_eq!(outcome.output.route, "server_error");
        assert!(outcome.output.content.contains("internal"));
    }

    #[tokio::test]
    async fn http_executor_404_exit_code_1() {
        let srv = TestServer::start();
        let def = ProviderDef::Http {
            url: srv.url("/nonexistent"),
            method: Some("GET".into()),
            headers: None,
            body: None,
        };
        let exe = HttpExecutor::from_provider(&def, &std::path::PathBuf::from("."));
        let outcome = exe
            .run(empty_ctx(), Duration::from_secs(10), "test_node", None)
            .await
            .expect("request should complete");

        assert_eq!(outcome.exit_code, 1);
        assert_eq!(outcome.output.route, "client_error");
    }

    #[tokio::test]
    async fn http_executor_chunk_notifications() {
        let srv = TestServer::start();
        let def = ProviderDef::Http {
            url: srv.url("/ok"),
            method: Some("GET".into()),
            headers: None,
            body: None,
        };
        let exe = HttpExecutor::from_provider(&def, &std::path::PathBuf::from("."));

        let (tx, mut rx) = tokio::sync::mpsc::channel::<NodeChunk>(16);
        exe.run(empty_ctx(), Duration::from_secs(10), "test_node", Some(tx))
            .await
            .expect("request should succeed");

        // Should have received at least 2 chunks: request + response.
        let chunks: Vec<_> = {
            let mut v = Vec::new();
            while let Ok(c) = rx.try_recv() {
                v.push(c.text);
            }
            v
        };
        assert!(
            chunks.iter().any(|c| c.contains("[HTTP] GET")),
            "should contain request chunk, got: {chunks:?}"
        );
        assert!(
            chunks.iter().any(|c| c.contains("[HTTP] 200")),
            "should contain response chunk, got: {chunks:?}"
        );
    }

    #[tokio::test]
    async fn http_executor_template_interpolation_in_url() {
        let srv = TestServer::start();
        // Build a context with upstream data that should be interpolated into the URL.
        let ctx = NodeContext {
            inputs: HashMap::new(),
            extensions: HashMap::new(),
            metadata: NodeMetadata {
                run_count: 2,
                timed_out: false,
            },
            upstream: HashMap::from([(
                "source".to_string(),
                NodeOutput {
                    route: "ok".into(),
                    content: "echo".into(),
                },
            )]),
        };

        // Use {{datarouter.source.content}} in the URL path.
        let def = ProviderDef::Http {
            url: format!("{}/{{{{datarouter.source.content}}}}", srv.addr()),
            method: Some("POST".into()),
            headers: None,
            body: Some(r#"{"from":"upstream"}"#.into()),
        };
        let exe = HttpExecutor::from_provider(&def, &std::path::PathBuf::from("."));
        let outcome = exe
            .run(ctx, Duration::from_secs(10), "test_node", None)
            .await
            .expect("request should succeed");

        // The URL should have been interpolated to .../echo, hitting the POST /echo handler.
        assert_eq!(outcome.output.route, "echo");
    }

    #[tokio::test]
    async fn http_executor_template_in_body() {
        let srv = TestServer::start();
        let ctx = NodeContext {
            inputs: HashMap::new(),
            extensions: HashMap::new(),
            metadata: NodeMetadata {
                run_count: 1,
                timed_out: false,
            },
            upstream: HashMap::from([(
                "data".to_string(),
                NodeOutput {
                    route: "ok".into(),
                    content: "hello-world".into(),
                },
            )]),
        };

        let def = ProviderDef::Http {
            url: srv.url("/echo"),
            method: Some("POST".into()),
            headers: None,
            body: Some(r#"{"msg":"{{datarouter.data.content}}"}"#.into()),
        };
        let exe = HttpExecutor::from_provider(&def, &std::path::PathBuf::from("."));
        let outcome = exe
            .run(ctx, Duration::from_secs(10), "test_node", None)
            .await
            .expect("POST should succeed");

        assert_eq!(outcome.output.route, "echo");
        assert!(
            outcome.output.content.contains("hello-world"),
            "body should contain interpolated value, got: {}",
            outcome.output.content
        );
    }

    #[tokio::test]
    async fn http_executor_timeout() {
        let srv = TestServer::start();
        let def = ProviderDef::Http {
            url: srv.url("/slow"),
            method: Some("GET".into()),
            headers: None,
            body: None,
        };
        let exe = HttpExecutor::from_provider(&def, &std::path::PathBuf::from("."));
        let outcome = exe
            .run(empty_ctx(), Duration::from_millis(500), "test_node", None)
            .await;

        // /slow delays 3 s, timeout is 500 ms → should timeout.
        match outcome {
            Ok(o) => {
                assert!(
                    o.timed_out(),
                    "expected timeout but got exit_code={}, route={}",
                    o.exit_code,
                    o.output.route
                );
            }
            Err(e) => {
                assert!(
                    e.message.contains("timeout") || e.message.contains("timed out"),
                    "expected timeout message, got: {}",
                    e.message
                );
            }
        }
    }

    #[tokio::test]
    async fn http_executor_method_defaults_to_get() {
        let srv = TestServer::start();
        let def = ProviderDef::Http {
            url: srv.url("/ok"),
            method: None, // should default to GET
            headers: None,
            body: None,
        };
        let exe = HttpExecutor::from_provider(&def, &std::path::PathBuf::from("."));
        let outcome = exe
            .run(empty_ctx(), Duration::from_secs(10), "test_node", None)
            .await
            .expect("request should succeed");

        assert_eq!(outcome.exit_code, 0);
        assert_eq!(outcome.output.route, "ok");
    }

    #[tokio::test]
    async fn http_executor_custom_headers() {
        let srv = TestServer::start();
        let def = ProviderDef::Http {
            url: srv.url("/auth"),
            method: Some("GET".into()),
            headers: Some(HashMap::from([(
                "Authorization".into(),
                "Bearer secret".into(),
            )])),
            body: None,
        };
        let exe = HttpExecutor::from_provider(&def, &std::path::PathBuf::from("."));
        let outcome = exe
            .run(empty_ctx(), Duration::from_secs(10), "test_node", None)
            .await
            .expect("request should succeed");

        assert_eq!(outcome.output.route, "authenticated");
    }

    #[tokio::test]
    async fn http_executor_connection_refused() {
        // Point to a port that's unlikely to be listening.
        let def = ProviderDef::Http {
            url: "http://127.0.0.1:1/nope".into(),
            method: Some("GET".into()),
            headers: None,
            body: None,
        };
        let exe = HttpExecutor::from_provider(&def, &std::path::PathBuf::from("."));
        let outcome = exe
            .run(empty_ctx(), Duration::from_secs(2), "test_node", None)
            .await;

        assert!(
            outcome.is_err(),
            "connection to dead port should fail, got: {outcome:?}"
        );
        let err = outcome.unwrap_err();
        assert!(
            err.message.contains("HTTP request failed")
                || err.message.contains("Connection")
                || err.message.contains("timed out"),
            "error message should indicate connection failure, got: {}",
            err.message
        );
    }
}
