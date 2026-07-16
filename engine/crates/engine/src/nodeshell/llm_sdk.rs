//! Anthropic SDK executor — delegates to `llm_sdk.py` for direct API calls.
//!
//! Unlike [`LlmExecutor`](super::LlmExecutor) which spawns a CLI tool,
//! this executor uses the Anthropic Python SDK (`pip install anthropic`)
//! to call the API directly.  The Python script handles streaming, tool
//! calling, and structured output.


use std::path::PathBuf;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::mpsc;

use super::TemplateEngine;
use super::types::{exit_codes, NodeChunk, NodeContext, NodeOutcome, NodeOutput, SpawnError};

/// Executes an LLM node via the Anthropic Python SDK.
///
/// Delegates to the bundled `llm_sdk.py` script which calls
/// `anthropic.Anthropic().messages.stream()` and streams tokens to stderr.
#[derive(Debug, Clone)]
pub struct LlmSdkExecutor {
    /// Anthropic model ID.
    model: String,
    /// Env var name for the API key.
    api_key_env: Option<String>,
    /// Optional system prompt.
    system_prompt: Option<String>,
    /// Prompt template with `{{metadata.*}}` / `{{datarouter.*.*}}` placeholders.
    prompt_template: Option<String>,
    /// Expected route values.
    routes: Vec<String>,
    /// Maximum output tokens.
    max_tokens: Option<u64>,
    /// Path to the `llm_sdk.py` wrapper script.
    wrapper_path: PathBuf,
    scripts_dir: PathBuf,
}

impl LlmSdkExecutor {
    /// Env var for overriding the wrapper path.
    const WRAPPER_ENV: &'static str = "NEXUS_LLM_SDK_WRAPPER";

    /// Create a new SDK executor.
    ///
    /// The `scripts_dir` is the resolved scripts directory for the owning node.
    #[must_use]
    pub fn new(
        model: &str,
        api_key_env: Option<String>,
        system_prompt: Option<String>,
        prompt_template: Option<String>,
        routes: Vec<String>,
        max_tokens: Option<u64>,
        scripts_dir: PathBuf,
    ) -> Self {
        let wrapper_path = std::env::var(Self::WRAPPER_ENV)
            .ok()
            .filter(|s| !s.is_empty()).map_or_else(|| scripts_dir.join("llm_sdk.py"), PathBuf::from);

        Self {
            model: model.to_string(),
            api_key_env,
            system_prompt,
            prompt_template,
            routes,
            max_tokens,
            wrapper_path,
            scripts_dir,
        }
    }

    /// Build a [`NodeContext`] with SDK config and rendered prompt in
    /// `extensions`.  The Python script reads `_sdk_*` keys from extensions
    /// to configure the Anthropic client.
    fn build_context(&self, ctx: &NodeContext) -> NodeContext {
        let mut extensions = ctx.extensions.clone();

        // SDK configuration
        extensions.insert("_sdk_model".to_string(), self.model.clone());
        if let Some(ref env) = self.api_key_env {
            extensions.insert("_sdk_api_key_env".to_string(), env.clone());
        }
        if let Some(ref sys) = self.system_prompt {
            extensions.insert("_sdk_system_prompt".to_string(), sys.clone());
        }
        if let Some(mt) = self.max_tokens {
            extensions.insert("_sdk_max_tokens".to_string(), mt.to_string());
        }

        // Render prompt template
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

        // Expected routes
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

    /// Run the LLM SDK node via the Python wrapper.
    pub async fn run(
        &self,
        mut ctx: NodeContext,
        timeout: Duration,
        node_id: &str,
        chunk_tx: Option<mpsc::Sender<NodeChunk>>,
    ) -> Result<NodeOutcome, SpawnError> {
        ctx.sanitize_surrogates();
        let ctx_with_config = self.build_context(&ctx);
        let ctx_json =
            serde_json::to_string(&ctx_with_config).map_err(|e| SpawnError {
                message: format!("serialize context: {e}"),
            })?;

        tracing::info!(
            target: "nexus::nodeshell",
            node_id,
            model = %self.model,
            wrapper = %self.wrapper_path.display(),
            "spawning llm_sdk node"
        );

        // Try multiple Python interpreter names.
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
                        message: format!("llm_sdk.py spawn failed ({python}): {e}"),
                    });
                }
            }
        }

        let mut child = child.ok_or_else(|| {
            last_err.unwrap_or_else(|| SpawnError {
                message: "llm_sdk.py spawn failed: no Python interpreter found".into(),
            })
        })?;

        // Write context JSON to stdin, then close → child sees EOF.
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(ctx_json.as_bytes()).await.map_err(|e| SpawnError {
                message: format!("stdin write failed: {e}"),
            })?;
        }

        let (exit_code, timed_out, stdout, stderr) =
            run_child_with_timeout(child, timeout, chunk_tx).await;

        if !stderr.is_empty() {
            tracing::info!(target: "nexus::node::stderr", node_id, stderr = %stderr);
        }

        if timed_out {
            return Ok(NodeOutcome {
                output: NodeOutput {
                    route: "timeout".into(),
                    content: stdout,
                },
                exit_code,
                exit_reason: Some("timeout".into()),
            });
        }

        let output = serde_json::from_str::<NodeOutput>(&stdout).unwrap_or_else(|_| {
            NodeOutput {
                route: String::new(),
                content: stdout,
            }
        });

        tracing::info!(
            target: "nexus::llm_sdk::response",
            node_id,
            route = output.route,
            content_len = output.content.len(),
        );

        let exit_reason = if output.route.is_empty() {
            None
        } else {
            Some(output.route.clone())
        };

        Ok(NodeOutcome {
            output,
            exit_code,
            exit_reason,
        })
    }
}

// ── subprocess helper ──────────────────────────────────────────────

async fn run_child_with_timeout(
    mut child: tokio::process::Child,
    timeout: Duration,
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
            Ok(Ok(s)) => {
                let _ = exit_tx.send((s.code().unwrap_or(-1), false));
            }
            Ok(Err(_)) => {
                let _ = exit_tx.send((-1, false));
            }
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
    use std::path::PathBuf;

    use super::*;

    fn test_scripts_dir() -> PathBuf {
        PathBuf::from("scripts")
    }

    #[test]
    fn test_new_stores_config() {
        let exe = LlmSdkExecutor::new(
            "claude-sonnet-5-20251001",
            None,
            Some("You are a code reviewer.".into()),
            Some("Review: {{datarouter.code.content}}".into()),
            vec!["ok".into(), "reject".into()],
            Some(4096),
            test_scripts_dir(),
        );
        assert_eq!(exe.model, "claude-sonnet-5-20251001");
        assert_eq!(exe.system_prompt.as_deref(), Some("You are a code reviewer."));
        assert_eq!(exe.routes, vec!["ok", "reject"]);
        assert_eq!(exe.max_tokens, Some(4096));
    }

    #[test]
    fn test_build_context_adds_sdk_config() {
        use super::super::types::NodeMetadata;
        use std::collections::HashMap;

        let exe = LlmSdkExecutor::new(
            "claude-sonnet-5-20251001",
            Some("MY_KEY_ENV".into()),
            Some("Be helpful.".into()),
            Some("Say hello".into()),
            vec!["ok".into()],
            Some(2048),
            test_scripts_dir(),
        );

        let ctx = NodeContext {
            inputs: HashMap::new(),
            extensions: HashMap::new(),
            metadata: NodeMetadata { run_count: 1, timed_out: false },
            upstream: HashMap::new(),
        };

        let built = exe.build_context(&ctx);
        assert_eq!(built.extensions.get("_sdk_model").unwrap(), "claude-sonnet-5-20251001");
        assert_eq!(built.extensions.get("_sdk_api_key_env").unwrap(), "MY_KEY_ENV");
        assert_eq!(built.extensions.get("_sdk_system_prompt").unwrap(), "Be helpful.");
        assert_eq!(built.extensions.get("_sdk_max_tokens").unwrap(), "2048");
        assert_eq!(built.extensions.get("prompt").unwrap(), "Say hello");
        assert_eq!(built.extensions.get("route").unwrap(), "ok");
    }
}
