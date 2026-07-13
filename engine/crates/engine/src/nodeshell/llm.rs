//! Generic LLM executor — delegates to `llm_node.py` for cross-platform
//! CLI resolution, subprocess management, and output parsing.
//!
//! The Rust side only renders `{{inputs.x}}` / `{{prompt}}` templates,
//! pipes a [`NodeContext`] to the Python script via stdin, and reads the
//! [`NodeOutput`] from stdout.  stderr is streamed to the engine log.

use std::collections::HashMap;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::mpsc;

use super::types::{exit_codes, NodeChunk, NodeContext, NodeOutcome, NodeOutput, SpawnError};

/// Executes an LLM CLI by delegating to the bundled `llm_node.py` script.
/// The Python script receives the fully-rendered command via `--cmd`
/// and handles cross-platform CLI resolution, execution, and output parsing.
#[derive(Debug, Clone)]
pub struct LlmExecutor {
    /// Command template with `{{prompt}}` placeholder.
    command_template: String,
    /// Prompt template with `{{inputs.x}}` placeholders.
    prompt_template: Option<String>,
    /// Expected route values.
    routes: Vec<String>,
    /// Path to the Python wrapper script.
    wrapper_path: String,
}

impl LlmExecutor {
    /// Fallback wrapper path, relative to CWD (used when exe-relative lookup fails).
    const DEFAULT_WRAPPER: &'static str = "scripts/llm_node.py";

    /// Search upward from the executable's directory for `scripts/llm_node.py`,
    /// up to `max_levels` parent levels.
    fn find_wrapper(max_levels: usize) -> Option<String> {
        let exe = std::env::current_exe().ok()?;
        let mut dir = exe.parent()?.to_path_buf();
        for _ in 0..=max_levels {
            let wrapper = dir.join("scripts").join("llm_node.py");
            if wrapper.exists() {
                return Some(wrapper.to_string_lossy().to_string());
            }
            if !dir.pop() {
                break;
            }
        }
        None
    }

    /// Resolve the path to `llm_node.py`, with the following priority:
    /// 1. `NEXUS_LLM_WRAPPER` environment variable (explicit override).
    /// 2. Exe-relative search (up to 3 levels up): supports `bin/nexus-cli.exe`
    ///    (Windows) and `bin/linux/nexus-cli` (Linux) release layouts.
    /// 3. CWD-relative fallback (`scripts/llm_node.py`, dev convenience).
    fn resolve_wrapper_path() -> String {
        // 1. Explicit env override takes precedence.
        if let Ok(path) = std::env::var("NEXUS_LLM_WRAPPER") {
            if !path.is_empty() {
                return path;
            }
        }

        // 2. Search upward from the executable for scripts/llm_node.py.
        if let Some(path) = Self::find_wrapper(3) {
            return path;
        }

        // 3. CWD-relative fallback (dev workflow or custom layouts).
        Self::DEFAULT_WRAPPER.to_string()
    }

    /// Create a new LLM executor.
    #[must_use]
    pub fn new(
        command_template: &str,
        prompt_template: Option<String>,
        routes: Vec<String>,
        _max_tokens: Option<u64>,
    ) -> Self {
        let wrapper_path = Self::resolve_wrapper_path();
        Self {
            command_template: command_template.to_string(),
            prompt_template,
            routes,
            wrapper_path,
        }
    }

    /// Render `{{inputs.key}}` placeholders.
    fn render_inputs(template: &str, inputs: &HashMap<String, String>) -> String {
        let mut result = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(start) = rest.find("{{inputs.") {
            result.push_str(&rest[..start]);
            let after = &rest[start + 9..];
            let end = after.find("}}").unwrap_or(0);
            if end == 0 {
                result.push_str(&rest[start..]);
                rest = "";
                break;
            }
            let key = &after[..end];
            result.push_str(inputs.get(key).map(|s| s.as_str()).unwrap_or(""));
            rest = &after[end + 2..];
        }
        result.push_str(rest);
        result
    }

    /// Build a NodeContext with the rendered prompt in extensions.
    fn build_context(&self, ctx: &NodeContext) -> NodeContext {
        let mut extensions = ctx.extensions.clone();

        // Render prompt template and store in extensions
        let prompt = if let Some(ref tmpl) = self.prompt_template {
            Self::render_inputs(tmpl, &ctx.inputs)
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
        }
    }

    /// Run the LLM node via the Python wrapper.
    pub async fn run(
        &self,
        ctx: NodeContext,
        timeout: Duration,
        node_id: &str,
        chunk_tx: Option<mpsc::Sender<NodeChunk>>,
    ) -> Result<NodeOutcome, SpawnError> {
        let ctx_with_prompt = self.build_context(&ctx);
        let ctx_json =
            serde_json::to_string(&ctx_with_prompt).map_err(|e| SpawnError {
                message: format!("serialize context: {e}"),
            })?;

        // Render the full command (replace {{prompt}} with the prompt from context)
        let prompt_text = ctx_with_prompt.extensions.get("prompt").map(|s| s.as_str()).unwrap_or("");
        let rendered_cmd = self.command_template.replace("{{prompt}}", prompt_text);

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
            let mut cmd = if cfg!(windows) {
                let mut c = Command::new("cmd.exe");
                c.arg("/c").arg(python);
                c
            } else {
                Command::new(*python)
            };
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
    use super::*;

    #[test]
    fn test_render_inputs() {
        let mut inputs = HashMap::new();
        inputs.insert("code".into(), "fn main() {}".into());
        let result = LlmExecutor::render_inputs("Review: {{inputs.code}}", &inputs);
        assert_eq!(result, "Review: fn main() {}");
    }

    #[test]
    fn test_render_inputs_no_match() {
        let inputs = HashMap::new();
        let result = LlmExecutor::render_inputs("No templates here", &inputs);
        assert_eq!(result, "No templates here");
    }

    #[test]
    fn test_new_stores_command_template() {
        let exe = LlmExecutor::new(
            "claude -p \"{{prompt}}\" --output-format json",
            None, vec![], None,
        );
        assert_eq!(exe.command_template, "claude -p \"{{prompt}}\" --output-format json");
    }

    #[test]
    fn test_new_any_cli() {
        let exe = LlmExecutor::new(
            "nga run --json \"{{prompt}}\"",
            None, vec!["ok".into()], Some(512),
        );
        assert_eq!(exe.command_template, "nga run --json \"{{prompt}}\"");
        assert_eq!(exe.routes, vec!["ok"]);
    }

    #[test]
    fn test_build_context_adds_prompt_and_routes() {
        let mut inputs = HashMap::new();
        inputs.insert("seed".into(), "hello".into());
        let ctx = NodeContext {
            inputs,
            extensions: HashMap::new(),
            metadata: super::super::types::NodeMetadata { run_count: 1, timed_out: false },
        };
        let exe = LlmExecutor::new(
            "claude -p \"{{prompt}}\"",
            Some("Review: {{inputs.seed}}".into()),
            vec!["ok".into(), "err".into()],
            None,
        );
        let built = exe.build_context(&ctx);
        assert_eq!(built.extensions.get("prompt").unwrap(), "Review: hello");
        assert_eq!(built.extensions.get("route").unwrap(), "ok,err");
    }
}
