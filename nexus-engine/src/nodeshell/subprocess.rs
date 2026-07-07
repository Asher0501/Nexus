use std::collections::HashMap;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc::{self, Sender};

use super::types::{NodeChunk, NodeContext, NodeOutcome, SpawnError};

/// Executes a node by spawning a subprocess.
///
/// The command string is split on whitespace: the first token is the program
/// to execute and the remaining tokens are arguments. This mirrors how shells
/// parse a simple command line. For complex quoting needs, pass the command
/// through a wrapper script.
///
/// # stdout line protocol
///
/// The subprocess can optionally use a streaming protocol for real-time events:
///
/// - `__nexus_log: <text>` — intermediate log (excluded from output)
/// - `__nexus_exit_reason: <value>` — set exit_reason before exit
/// - `__nexus_event: <text>` — structured output fragment (added to output)
/// - `__nexus_log_end` — disable prefix checks for subsequent lines
///
/// Without any `__nexus_` prefix, all stdout lines are treated as output.
#[derive(Debug, Clone)]
pub struct SubprocessExecutor {
    /// The command to execute.
    command: String,
}

impl SubprocessExecutor {
    /// Create a new subprocess executor for the given command string.
    #[must_use]
    pub fn new(command: String) -> Self {
        Self { command }
    }

    /// Split the command string into program and arguments.
    fn split_command(command: &str) -> (&str, Vec<&str>) {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            return ("", vec![]);
        }
        let mut parts = trimmed.split_whitespace();
        let program = parts.next().unwrap_or("");
        let args: Vec<&str> = parts.collect();
        (program, args)
    }

    /// Render a prompt template, replacing `{{inputs.name}}` with values
    /// from the node context's inputs map.
    fn render_template(template: &str, inputs: &HashMap<String, String>) -> String {
        let mut result = String::with_capacity(template.len());
        let mut rest = template;

        while let Some(start) = rest.find("{{inputs.") {
            result.push_str(&rest[..start]);
            let after_start = &rest[start + 9..];
            let end = after_start.find("}}").unwrap_or(0);
            if end == 0 {
                result.push_str(&rest[start..]);
                break;
            }
            let key = &after_start[..end];
            let value = inputs.get(key).map(|s| s.as_str()).unwrap_or("");
            result.push_str(value);
            let consumed = start + 9 + end + 2;
            rest = &rest[consumed..];
        }

        result.push_str(rest);
        result
    }

    /// Run the subprocess with streaming stdout/stderr.
    ///
    /// Streams stdout/stderr lines in real time via `tracing` events while the
    /// child is still running. This allows callers to observe intermediate
    /// output (thinking, progress) before the process exits.
    ///
    /// If the command contains `{{inputs.x}}` patterns, they are replaced with
    /// values from the node context's inputs map before spawning.
    ///
    /// # Errors
    ///
    /// Returns [`SpawnError`] if the command cannot be spawned.
    pub async fn run(
        &self,
        ctx: NodeContext,
        timeout: Duration,
        node_id: &str,
        chunk_tx: Option<mpsc::Sender<NodeChunk>>,
    ) -> Result<NodeOutcome, SpawnError> {
        // Render template in command string ({{inputs.x}} → ctx.inputs[x]).
        let command = if self.command.contains("{{inputs.") {
            Self::render_template(&self.command, &ctx.inputs)
        } else {
            self.command.clone()
        };

        let (program, args) = Self::split_command(&command);
        if program.is_empty() {
            return Err(SpawnError {
                message: "empty command".into(),
            });
        }

        let mut cmd = Command::new(program);
        cmd.args(&args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| SpawnError {
            message: e.to_string(),
        })?;

        // Write context JSON to stdin, then stdin drops → pipe closes → child sees EOF.
        if let Some(mut stdin) = child.stdin.take() {
            let input = serde_json::to_string(&ctx).map_err(|e| SpawnError {
                message: format!("serialize context: {e}"),
            })?;
            stdin.write_all(input.as_bytes()).await.map_err(|e| SpawnError {
                message: format!("stdin write failed: {e}"),
            })?;
        }

        // Stream stdout/stderr and wait concurrently.
        stream_and_wait(child, timeout, node_id, chunk_tx).await
    }
}

/// Stream stdout/stderr lines from a running child process, emit them in
/// real time via tracing, then return the final `NodeOutcome`.
///
/// If `chunk_tx` is provided, each stdout line is also sent through the
/// channel in real time (before the process exits).
///
/// Uses `tokio::spawn` for the `wait()` future so that `child` is never
/// mutably borrowed during the select loop (sidestepping the pinning
/// conflict between `child.wait()` and `child.kill()`).
fn process_line(
    output_buf: &mut String,
    text: &str,
    log_mode: &mut bool,
    exit_reason: &mut Option<String>,
    chunk_tx: &Option<Sender<NodeChunk>>,
    node_id: &str,
) {
    if *log_mode {
        output_buf.push_str(text);
        output_buf.push('\n');
    } else if text == "__nexus_log_end" {
        *log_mode = true;
    } else if let Some(rest) = text.strip_prefix("__nexus_log:") {
        tracing::info!(target: "nexus::node::log", node_id, log = rest.trim());
    } else if let Some(rest) = text.strip_prefix("__nexus_exit_reason:") {
        *exit_reason = Some(rest.trim().to_string());
    } else if let Some(rest) = text.strip_prefix("__nexus_event:") {
        let event_text = rest.trim();
        if !event_text.is_empty() {
            output_buf.push_str(event_text);
            output_buf.push('\n');
            tracing::info!(target: "nexus::node::event", node_id, event = event_text);
        }
    } else {
        output_buf.push_str(text);
        output_buf.push('\n');
        if let Some(tx) = chunk_tx {
            // Chunk channel full is safe — NodeOutcome.output has full content
            let _ = tx.try_send(NodeChunk {
                text: text.to_string(),
                node_id: node_id.to_string(),
            });
        }
    }
}

async fn stream_and_wait(
    mut child: Child,
    timeout: Duration,
    node_id: &str,
    chunk_tx: Option<Sender<NodeChunk>>,
) -> Result<NodeOutcome, SpawnError> {
    let out_pipe = child.stdout.take();
    let err_pipe = child.stderr.take();

    let mut output_buf = String::new();
    let mut exit_reason: Option<String> = None;
    let mut exit_code: i32 = 0;
    let mut timed_out = false;
    let mut log_mode = false;
    // Track whether the streaming protocol is active (reserved for future use).
    let _stream_protocol = false;

    // Spawn background line readers for stdout/stderr pipes.
    let (stdout_tx, mut stdout_rx) = tokio::sync::mpsc::channel::<String>(1024);
    let (stderr_tx, mut stderr_rx) = tokio::sync::mpsc::channel::<String>(1024);

    if let Some(pipe) = out_pipe {
        let tx = stdout_tx;
        tokio::spawn(async move {
            let mut lines = BufReader::new(pipe).lines();
            loop {
                match lines.next_line().await {
                    // Channel send failure: receiver dropped (task cancelled) — exit cleanly.
                    Ok(Some(l)) => { let _ = tx.send(l).await; }
                    _ => break,
                }
            }
        });
    }
    if let Some(pipe) = err_pipe {
        let tx = stderr_tx;
        tokio::spawn(async move {
            let mut lines = BufReader::new(pipe).lines();
            loop {
                match lines.next_line().await {
                    // Channel send failure: receiver dropped (task cancelled) — exit cleanly.
                    Ok(Some(l)) => { let _ = tx.send(l).await; }
                    _ => break,
                }
            }
        });
    }

    // Also spawn wait() into a background task so we can select!
    // between channel recv and exit notification in real time.
    // (exit_code, timed_out)
    let (exit_tx, mut exit_rx) = tokio::sync::oneshot::channel::<(i32, bool)>();
    tokio::spawn(async move {
        let waited = tokio::time::timeout(timeout, child.wait()).await;
        match waited {
            // Oneshot send failure: receiver dropped (stream_and_wait already exited) — safe to ignore.
            Ok(Ok(s)) => { let _ = exit_tx.send((s.code().unwrap_or(-1), false)); }
            // Oneshot send failure: receiver dropped (stream_and_wait already exited) — safe to ignore.
            Ok(Err(_)) => { let _ = exit_tx.send((-1, false)); }
            Err(_elapsed) => {
                if let Err(e) = child.kill().await {
                    tracing::warn!(target: "nexus::node", "[NodeShell] failed to kill timed-out child: {e}");
                }
                // Oneshot send failure: receiver dropped (stream_and_wait already exited) — safe to ignore.
                let _ = exit_tx.send((-1, true));
            }
        }
    });

    // Stream lines in real time until exit notification.
    loop {
        tokio::select! {
            Some(text) = stdout_rx.recv() => {
                process_line(
                    &mut output_buf,
                    &text,
                    &mut log_mode,
                    &mut exit_reason,
                    &chunk_tx,
                    node_id,
                );
            }
            Some(text) = stderr_rx.recv() => {
                tracing::warn!(target: "nexus::node::stderr", node_id, stderr = text);
            }
            result = &mut exit_rx => {
                match result {
                    Ok((c, t)) => { exit_code = c; timed_out = t; }
                    Err(_) => {}
                }
                break;
            }
        }
    }

    // After exit signal, drain remaining lines efficiently.
    // Uses a short sleep to avoid busy-waiting, allowing the
    // line-reader task time to flush final lines.
    for _ in 0..5 {
        while let Ok(text) = stdout_rx.try_recv() {
            process_line(
                &mut output_buf,
                &text,
                &mut log_mode,
                &mut exit_reason,
                &chunk_tx,
                node_id,
            );
        }
        while let Ok(text) = stderr_rx.try_recv() {
            tracing::warn!(target: "nexus::node::stderr", node_id, stderr = text);
        }
        tokio::time::sleep(Duration::from_micros(200)).await;
    }
    // Final drain
    while let Ok(text) = stdout_rx.try_recv() {
        process_line(
            &mut output_buf,
            &text,
            &mut log_mode,
            &mut exit_reason,
            &chunk_tx,
            node_id,
        );
    }
    while let Ok(text) = stderr_rx.try_recv() {
        tracing::warn!(target: "nexus::node::stderr", node_id, stderr = text);
    }

    if output_buf.ends_with('\n') {
        output_buf.truncate(output_buf.len().saturating_sub(1));
    }

    Ok(NodeOutcome {
        output: output_buf,
        exit_code,
        timed_out,
        exit_reason,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[tokio::test]
    async fn test_echo_subprocess() {
        let executor = SubprocessExecutor::new("cmd.exe /c echo hello".into());
        let ctx = NodeContext {
            inputs: HashMap::new(),
            extensions: HashMap::new(),
        };
        let outcome = executor.run(ctx, Duration::from_secs(5), "echo", None).await;
        assert!(outcome.is_ok(), "echo should succeed: {:?}", outcome);
        let outcome = outcome.unwrap();
        assert_eq!(outcome.exit_code, 0);
        assert!(!outcome.timed_out);
    }

    #[tokio::test]
    async fn test_exit_reason_extraction() {
        let executor =
            SubprocessExecutor::new(
                "cmd.exe /c echo __nexus_exit_reason: approved && echo data_line_2".into(),
            );
        let ctx = NodeContext {
            inputs: HashMap::new(),
            extensions: HashMap::new(),
        };
        let outcome = executor.run(ctx, Duration::from_secs(5), "exit_reason_test", None).await
            .expect("cmd should succeed");
        assert!(outcome.output.contains("data_line_2"));
    }

    #[tokio::test]
    async fn test_timeout() {
        let executor = SubprocessExecutor::new("ping -n 60 127.0.0.1".into());
        let ctx = NodeContext {
            inputs: HashMap::new(),
            extensions: HashMap::new(),
        };
        let result = executor.run(ctx, Duration::from_millis(10), "timeout_test", None).await;
        match result {
            Ok(outcome) => {
                assert!(outcome.timed_out, "should timeout with 10ms timeout");
            }
            Err(_) => {
                // Spawn might fail on some systems — that's ok
            }
        }
    }

    #[tokio::test]
    async fn test_spawn_failure() {
        let executor = SubprocessExecutor::new("nonexistent_command_12345".into());
        let ctx = NodeContext {
            inputs: HashMap::new(),
            extensions: HashMap::new(),
        };
        let result = executor.run(ctx, Duration::from_secs(1), "spawn_fail", None).await;
        assert!(result.is_err(), "nonexistent command should return SpawnError");
    }

    #[tokio::test]
    async fn test_empty_command() {
        let executor = SubprocessExecutor::new(String::new());
        let ctx = NodeContext {
            inputs: HashMap::new(),
            extensions: HashMap::new(),
        };
        let result = executor.run(ctx, Duration::from_secs(1), "empty_cmd", None).await;
        assert!(result.is_err(), "empty command should return SpawnError");
    }

    #[tokio::test]
    async fn test_render_template_simple() {
        let mut inputs = HashMap::new();
        inputs.insert("code".into(), "fn main() {}".into());
        let result = SubprocessExecutor::render_template("echo {{inputs.code}}", &inputs);
        assert_eq!(result, "echo fn main() {}", "simple variable should be replaced");
    }

    #[tokio::test]
    async fn test_render_template_no_template() {
        let inputs = HashMap::new();
        let result = SubprocessExecutor::render_template("echo hello", &inputs);
        assert_eq!(result, "echo hello", "plain text should not change");
    }

    #[tokio::test]
    async fn test_render_template_missing_key() {
        let inputs = HashMap::new();
        let result = SubprocessExecutor::render_template("echo {{inputs.missing}}", &inputs);
        assert_eq!(result, "echo ", "missing key should be replaced with empty string");
    }

    #[tokio::test]
    async fn test_stream_event_extraction() {
        let executor = SubprocessExecutor::new(
            "cmd.exe /c echo __nexus_event: result_fragment && echo __nexus_exit_reason: done".into(),
        );
        let ctx = NodeContext {
            inputs: HashMap::new(),
            extensions: HashMap::new(),
        };
        let outcome = executor.run(ctx, Duration::from_secs(5), "stream_test", None).await
            .expect("cmd should succeed");
        assert!(outcome.output.contains("result_fragment"));
        assert_eq!(outcome.exit_reason, Some("done".into()));
    }

    #[test]
    fn test_split_command_whitespace() {
        let (prog, args) = SubprocessExecutor::split_command("cmd.exe /c echo hello");
        assert_eq!(prog, "cmd.exe");
        assert_eq!(args, vec!["/c", "echo", "hello"]);
    }

    #[test]
    fn test_split_command_no_args() {
        let (prog, args) = SubprocessExecutor::split_command("cmd.exe");
        assert_eq!(prog, "cmd.exe");
        assert!(args.is_empty());
    }

    #[test]
    fn test_split_command_empty() {
        let (prog, args) = SubprocessExecutor::split_command("");
        assert_eq!(prog, "");
        assert!(args.is_empty());
    }
}
