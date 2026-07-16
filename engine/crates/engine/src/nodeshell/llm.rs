//! Generic LLM executor — delegates to `llm_node.py` for cross-platform
//! CLI resolution, subprocess management, and output parsing.
//!
//! The Rust side renders `{{metadata.*}}` / `{{datarouter.*.*}}` templates,
//! pipes a [`NodeContext`] to the Python script via stdin, and reads the
//! [`NodeOutput`] from stdout.  stderr is streamed to the engine log.

use std::path::PathBuf;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::mpsc;

use super::TemplateEngine;
use super::types::{exit_codes, NodeChunk, NodeContext, NodeOutcome, NodeOutput, SpawnError};

/// Executes an LLM CLI by delegating to the bundled `llm_node.py` script.
///
/// The Python script receives the fully-rendered command via `--cmd`
/// and handles cross-platform CLI resolution, execution, and output parsing.
#[derive(Debug, Clone)]
pub struct LlmExecutor {
    command_template: String,
    prompt_template: Option<String>,
    routes: Vec<String>,
    wrapper_path: PathBuf,
    scripts_dir: PathBuf,
}

impl LlmExecutor {
    const WRAPPER_ENV: &'static str = "NEXUS_LLM_WRAPPER";

    /// Create a new LLM executor.
    #[must_use]
    pub fn new(
        command_template: &str,
        prompt_template: Option<String>,
        routes: Vec<String>,
        _max_tokens: Option<u64>,
        scripts_dir: PathBuf,
    ) -> Self {
        let wrapper_path = std::env::var(Self::WRAPPER_ENV)
            .ok()
            .filter(|s| !s.is_empty()).map_or_else(|| scripts_dir.join("llm_node.py"), PathBuf::from);
        Self { command_template: command_template.to_string(), prompt_template, routes, wrapper_path, scripts_dir }
    }

    /// Build a `NodeContext` with the rendered prompt in extensions.
    fn build_context(&self, ctx: &NodeContext) -> NodeContext {
        let mut extensions = ctx.extensions.clone();

        let prompt = if let Some(ref tmpl) = self.prompt_template {
            TemplateEngine::render(tmpl, &ctx.metadata, &ctx.upstream, &self.scripts_dir.to_string_lossy())
        } else if !ctx.inputs.is_empty() {
            ctx.inputs
                .iter()
                .map(|(k, v)| format!("[{k}]: {v}"))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            String::new()
        };
        extensions.insert("prompt".to_string(), prompt);

        // Route info
        if !self.routes.is_empty() {
            extensions.insert("route".to_string(), self.routes.join(","));
        }

        NodeContext {
            inputs: ctx.inputs.clone(),
            extensions,
            metadata: ctx.metadata.clone(),
            upstream: ctx.upstream.clone(),
        }
    }

    /// Run the LLM node via the Python wrapper.
    pub async fn run(
        &self,
        mut ctx: NodeContext,
        timeout: Duration,
        node_id: &str,
        chunk_tx: Option<mpsc::Sender<NodeChunk>>,
    ) -> Result<NodeOutcome, SpawnError> {
        ctx.sanitize_surrogates();
        let ctx_with_prompt = self.build_context(&ctx);
        let ctx_json =
            serde_json::to_string(&ctx_with_prompt).map_err(|e| SpawnError {
                message: format!("serialize context: {e}"),
            })?;

        // Render command: replace {{prompt}} with placeholder that llm_node.py
        // will fill from stdin context.  This avoids double-escaping problems
        // when passing quoted prompt text through cmd.exe on Windows.
        let _prompt_text = ctx_with_prompt.extensions.get("prompt").map_or("", std::string::String::as_str);
        let rendered_cmd = if self.command_template.contains("{{prompt}}") {
            // Leave {{prompt}} as-is; llm_node.py replaces it from stdin.
            self.command_template.clone()
        } else {
            self.command_template.clone()
        };

        // Build Python command — pass the full CLI command via --cmd.
        // Try multiple Python interpreter names: on Windows, some users
        // only have the `py` launcher, not `python` on PATH.
        let python_candidates: &[&str] = if cfg!(windows) {
            &["python", "py"]
        } else {
            &["python3", "python"]
        };

        let mut last_err: Option<SpawnError> = None;
        let mut child = None;

        for python in python_candidates {
            let mut cmd = Command::new(*python);
            cmd.arg(&self.wrapper_path)
                .arg("--cmd")
                .arg(&rendered_cmd)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());

            match cmd.spawn() {
                Ok(c) => {
                    child = Some(c);
                    break;
                }
                Err(e) => {
                    last_err = Some(SpawnError {
                        message: format!("llm_node.py spawn failed ({python}): {e}"),
                    });
                }
            }
        }

        let mut child = child.ok_or_else(|| last_err.unwrap_or_else(|| SpawnError {
            message: "llm_node.py spawn failed: no Python interpreter found".into(),
        }))?;

        // Write context JSON to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(ctx_json.as_bytes()).await.map_err(|e| SpawnError {
                message: format!("stdin write failed: {e}"),
            })?;
        }

        // Run with timeout, streaming stderr to chunk_tx
        let (exit_code, timed_out, stdout, stderr) =
            run_child_with_timeout(child, timeout, node_id, chunk_tx).await;

        if !stderr.is_empty() {
            tracing::warn!(target: "nexus::node::stderr", node_id, stderr = %stderr);
        }

        if timed_out {
            return Ok(NodeOutcome {
                output: NodeOutput { route: "timeout".into(), content: stdout },
                exit_code,
                exit_reason: Some("timeout".into()),
            });
        }

        // Parse stdout as NodeOutput JSON
        let output = serde_json::from_str::<NodeOutput>(&stdout).unwrap_or_else(|_| {
            NodeOutput { route: String::new(), content: stdout }
        });

        // Log the LLM response so it's visible in the run log
        tracing::info!(
            target: "nexus::llm::response",
            node_id,
            route = output.route,
            content = output.content,
        );

        let exit_reason = if output.route.is_empty() { None } else { Some(output.route.clone()) };

        Ok(NodeOutcome { output, exit_code, exit_reason })
    }
}

// ── subprocess helpers ──────────────────────────────────────────

async fn run_child_with_timeout(
    mut child: tokio::process::Child,
    timeout: Duration,
    _node_id: &str,
    chunk_tx: Option<mpsc::Sender<NodeChunk>>,
) -> (i32, bool, String, String) {
    let out_pipe = child.stdout.take();
    let err_pipe = child.stderr.take();

    let (stdout_tx, stdout_rx) = tokio::sync::oneshot::channel::<String>();
    tokio::spawn(async move {
        let mut output = String::new();
        if let Some(pipe) = out_pipe {
            let mut reader = tokio::io::BufReader::new(pipe);
            let _ = tokio::io::AsyncReadExt::read_to_string(&mut reader, &mut output).await;
        }
        let _ = stdout_tx.send(output);
    });

    let (stderr_tx, stderr_rx) = tokio::sync::oneshot::channel::<String>();
    tokio::spawn(async move {
        let mut full = String::new();
        if let Some(pipe) = err_pipe {
            let reader = tokio::io::BufReader::new(pipe);
            let mut lines = tokio::io::AsyncBufReadExt::lines(reader);
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(ref tx) = chunk_tx {
                    let _ = tx.send(NodeChunk { text: line.clone() }).await;
                }
                full.push_str(&line);
                full.push('\n');
            }
        }
        let _ = stderr_tx.send(full);
    });

    let (exit_tx, exit_rx) = tokio::sync::oneshot::channel::<(i32, bool)>();
    tokio::spawn(async move {
        let waited = tokio::time::timeout(timeout, child.wait()).await;
        match waited {
            Ok(Ok(s)) => { let _ = exit_tx.send((s.code().unwrap_or(-1), false)); }
            Ok(Err(_)) => { let _ = exit_tx.send((-1, false)); }
            Err(_) => {
                let _ = child.kill().await;
                let _ = exit_tx.send((exit_codes::TIMEOUT, true));
            }
        }
    });

    let (exit_code, timed_out) = exit_rx.await.unwrap_or((exit_codes::WAIT_FAILED, false));
    let stdout = stdout_rx.await.unwrap_or_default();
    let stderr = stderr_rx.await.unwrap_or_default();
    (exit_code, timed_out, stdout, stderr)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    
    use super::*;

    fn test_scripts_dir() -> PathBuf {
        PathBuf::from("scripts")
    }

    #[test]
    fn test_render_inputs() {
        let metadata = super::super::types::NodeMetadata { run_count: 1, timed_out: false };
        let upstream = HashMap::new();
        let result = TemplateEngine::render("No templates here", &metadata, &upstream, "scripts");
        assert_eq!(result, "No templates here");
    }

    #[test]
    fn test_render_metadata_run_count() {
        let metadata = super::super::types::NodeMetadata { run_count: 3, timed_out: false };
        let upstream = HashMap::new();
        let result = TemplateEngine::render("Round {{metadata.run_count}}", &metadata, &upstream, "scripts");
        assert_eq!(result, "Round 3");
    }

    #[test]
    fn test_render_metadata_timed_out() {
        let metadata = super::super::types::NodeMetadata { run_count: 1, timed_out: true };
        let upstream = HashMap::new();
        let result = TemplateEngine::render("timed_out={{metadata.timed_out}}", &metadata, &upstream, "scripts");
        assert_eq!(result, "timed_out=true");
    }

    #[test]
    fn test_render_datarouter_route() {
        let metadata = super::super::types::NodeMetadata { run_count: 1, timed_out: false };
        let mut upstream = HashMap::new();
        upstream.insert("merge".into(), super::super::types::NodeOutput { route: "dispatch".into(), content: "summary text".into() });
        let result = TemplateEngine::render("Route was {{datarouter.merge.route}}", &metadata, &upstream, "scripts");
        assert_eq!(result, "Route was dispatch");
    }

    #[test]
    fn test_render_datarouter_content() {
        let metadata = super::super::types::NodeMetadata { run_count: 1, timed_out: false };
        let mut upstream = HashMap::new();
        upstream.insert("merge".into(), super::super::types::NodeOutput { route: "dispatch".into(), content: "summary text".into() });
        let result = TemplateEngine::render("Content: {{datarouter.merge.content}}", &metadata, &upstream, "scripts");
        assert_eq!(result, "Content: summary text");
    }

    #[test]
    fn test_new_stores_command_template() {
        let exe = LlmExecutor::new("claude -p \"{{prompt}}\" --output-format json", None, vec![], None, test_scripts_dir());
        assert_eq!(exe.command_template, "claude -p \"{{prompt}}\" --output-format json");
    }

    #[test]
    fn test_new_any_cli() {
        let exe = LlmExecutor::new("nga run --json \"{{prompt}}\"", None, vec!["ok".into()], Some(512), test_scripts_dir());
        assert_eq!(exe.command_template, "nga run --json \"{{prompt}}\"");
        assert_eq!(exe.routes, vec!["ok"]);
    }

    #[test]
    fn test_build_context_adds_prompt_and_routes() {
        let mut inputs = HashMap::new();
        inputs.insert("seed".into(), "hello".into());
        let mut upstream = HashMap::new();
        upstream.insert("seed".into(), super::super::types::NodeOutput { route: "ok".into(), content: "hello".into() });
        let ctx = NodeContext { inputs, extensions: HashMap::new(), metadata: super::super::types::NodeMetadata { run_count: 1, timed_out: false }, upstream };
        let exe = LlmExecutor::new("claude -p \"{{prompt}}\"", Some("Review: {{datarouter.seed.content}}".into()), vec!["ok".into(), "err".into()], None, test_scripts_dir());
        let built = exe.build_context(&ctx);
        assert_eq!(built.extensions.get("prompt").unwrap(), "Review: hello");
        assert_eq!(built.extensions.get("route").unwrap(), "ok,err");
    }
}
