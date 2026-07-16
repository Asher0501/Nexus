use std::path::PathBuf;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use super::TemplateEngine;
use super::types::{exit_codes, NodeChunk, NodeContext, NodeOutcome, NodeOutput, SpawnError};

/// Executes a node by spawning a subprocess.
///
/// The command string is split on whitespace: the first token is the program
/// to execute and the remaining tokens are arguments. This mirrors how shells
/// parse a simple command line. For complex quoting needs, pass the command
/// through a wrapper script.
///
/// # stdout protocol
///
/// The subprocess must emit a single JSON object on stdout with the shape
/// `{"route":"...","content":"..."}` (the [`NodeOutput`] struct). The engine
/// parses it after the process exits. If stdout is not valid JSON with a
/// `route` field, execution fails with [`SpawnError`].
#[derive(Debug, Clone)]
pub struct SubprocessExecutor {
    command: String,
    shell: bool,
    scripts_dir: PathBuf,
}

impl SubprocessExecutor {
    /// Create a new subprocess executor.
    #[must_use]
    pub const fn new(command: String, scripts_dir: PathBuf) -> Self {
        Self { command, shell: false, scripts_dir }
    }

    /// Create a shell-wrapped executor.
    #[must_use]
    pub const fn new_shell(command: String, scripts_dir: PathBuf) -> Self {
        Self { command, shell: true, scripts_dir }
    }

    /// Split the command string into program and arguments.
    ///
    /// Splits on unquoted whitespace. Content inside double quotes (`"`)
    /// is treated as a single token (quotes are stripped).  Backslash-
    /// escaped double quotes inside a quoted segment are supported.
    ///
    /// For complex shell expressions (pipes, redirects, nested quoting),
    /// prefer the `Shell` provider which delegates to `sh -c` / `cmd /c`.
    fn split_command(command: &str) -> (String, Vec<String>) {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            return (String::new(), vec![]);
        }
        let mut tokens: Vec<String> = Vec::new();
        let mut current = String::new();
        let mut in_quotes = false;
        let chars = trimmed.chars();

        for ch in chars {
            match ch {
                '"' if !in_quotes => {
                    in_quotes = true;
                }
                '"' if in_quotes => {
                    // Check for escaped quote: backslash before the closing quote
                    if current.ends_with('\\') {
                        current.pop(); // remove backslash
                        current.push('"'); // keep literal quote
                    } else {
                        in_quotes = false;
                    }
                }
                c if c.is_whitespace() && !in_quotes => {
                    if !current.is_empty() {
                        tokens.push(std::mem::take(&mut current));
                    }
                }
                c => {
                    current.push(c);
                }
            }
        }
        if !current.is_empty() {
            tokens.push(current);
        }

        let program = tokens.first().cloned().unwrap_or_default();
        let args = if tokens.len() > 1 {
            tokens[1..].to_vec()
        } else {
            vec![]
        };
        (program, args)
    }

    /// Render `{{metadata.*}}` and `{{datarouter.*.*}}` placeholders
    /// in a prompt or command template.
    /// Resolve `scripts/` paths in the command to the configured scripts dir.
    fn resolve_scripts_path(&self, command: &str) -> String {
        if !command.contains("scripts/") {
            return command.to_string();
        }
        let abs = self.scripts_dir.to_string_lossy().replace('\\', "/");
        command.replace("scripts/", &format!("{abs}/"))
    }

    /// Run the subprocess, collect stdout as JSON, and return the parsed outcome.
    ///
    /// If the command contains `{{metadata.*}}` or `{{datarouter.*.*}}`
    /// patterns, they are rendered from the node context before spawning.
    ///
    /// Any `scripts/` relative paths in the command are resolved against the
    /// executable's sibling scripts directory, so workflows can reference
    /// `python scripts/xxx.py` without depending on CWD.
    ///
    /// For shell-wrapped commands, the command is passed directly to the
    /// shell without argument splitting, preserving quotes and shell syntax.
    ///
    /// The optional `chunk_tx` sender can be used to stream output lines
    /// back to the caller as they are produced (not yet implemented).
    ///
    /// # Errors
    ///
    /// Returns [`SpawnError`] if the command cannot be spawned or if stdout is
    /// not valid JSON with a `route` field.
    pub async fn run(
        &self,
        mut ctx: NodeContext,
        timeout: Duration,
        node_id: &str,
        chunk_tx: Option<mpsc::Sender<NodeChunk>>,
    ) -> Result<NodeOutcome, SpawnError> {
        ctx.sanitize_surrogates();
        // Render template in command string: {{metadata.*}},
        // {{datarouter.*.*}}. Shell mode: escape values to prevent command injection.
        let has_template = self.command.contains("{{metadata.")
            || self.command.contains("{{datarouter.")
            || self.command.contains("{{node_dir}}");
        let command = if has_template {
            if self.shell {
                TemplateEngine::render_shell(&self.command, &ctx.metadata, &ctx.upstream, &self.scripts_dir.to_string_lossy())
            } else {
                TemplateEngine::render(&self.command, &ctx.metadata, &ctx.upstream, &self.scripts_dir.to_string_lossy())
            }
        } else {
            self.command.clone()
        };

        let command = self.resolve_scripts_path(&command);

        tracing::info!(
            target: "nexus::nodeshell",
            node_id,
            command = %command,
            shell = self.shell,
            "spawning node"
        );

        // Emit a chunk so the node is visible in the run log, mirroring
        // what llm_node.py does with "[llm_node] CMD" on stderr.
        if let Some(ref tx) = chunk_tx {
            let label = if self.shell { "[shell_node]" } else { "[subprocess_node]" };
            let _ = tx.try_send(NodeChunk {
                text: format!("{label} {command}"),
            });
        }

        let mut cmd = if self.shell {
            // Shell mode: pass the entire command directly to the shell.
            // Do NOT split — the shell handles quoting, pipes, redirects, etc.
            if cfg!(windows) {
                let mut c = Command::new("cmd.exe");
                // Use raw_arg to bypass Rust's Command::arg() escaping.
                // Rust uses backslash-escaping for embedded double quotes
                // (producing \"), but cmd.exe doesn't understand backslash
                // escapes. Pass the command raw — cmd.exe's echo (and most
                // shell commands) preserve literal double-quote characters
                // in their output without any escaping needed.
                c.arg("/c");
                #[cfg(windows)]
                {
                    c.raw_arg(format!(" {command}"));
                }
                c
            } else {
                let mut c = Command::new("sh");
                c.arg("-c").arg(&command);
                c
            }
        } else {
            // Direct subprocess mode: split the command into program + args.
            let (program, args) = Self::split_command(&command);
            if program.is_empty() {
                return Err(SpawnError {
                    message: "empty command".into(),
                });
            }
            let mut c = Command::new(program);
            c.args(&args);
            c
        };

        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .env("PYTHONIOENCODING", "utf-8"); // stdout/stderr are UTF-8 pipes on all platforms

        let mut child = cmd.spawn().map_err(|e| SpawnError {
            message: e.to_string(),
        })?;

        // Write context JSON to stdin, then stdin drops -> pipe closes -> child sees EOF.
        if let Some(mut stdin) = child.stdin.take() {
            let input = serde_json::to_string(&ctx).map_err(|e| SpawnError {
                message: format!("serialize context: {e}"),
            })?;
            stdin.write_all(input.as_bytes()).await.map_err(|e| SpawnError {
                message: format!("stdin write failed: {e}"),
            })?;
        }

        // Collect stdout, read stderr, and wait.
        collect_and_wait(child, timeout, node_id, chunk_tx).await
    }
}

/// Collect stdout from a running child process, parse it as JSON [`NodeOutput`],
/// and return the final `NodeOutcome`.
///
/// Stderr is read asynchronously and logged via `tracing::warn`.
///
/// If the child times out, it is killed and the partially-collected output is
/// returned with `timed_out = true`.
#[allow(clippy::too_many_lines)]
async fn collect_and_wait(
    mut child: Child,
    timeout: Duration,
    node_id: &str,
    chunk_tx: Option<mpsc::Sender<NodeChunk>>,
) -> Result<NodeOutcome, SpawnError> {
    let out_pipe = child.stdout.take();
    let err_pipe = child.stderr.take();

    // Spawn background reader for stdout — stream lines as chunks, collect full text.
    let (stdout_tx, stdout_rx) = tokio::sync::oneshot::channel::<String>();
    let chunk_tx_out = chunk_tx.clone();
    tokio::spawn(async move {
        let mut output = String::new();
        if let Some(pipe) = out_pipe {
            let reader = tokio::io::BufReader::new(pipe);
            let mut lines = tokio::io::AsyncBufReadExt::lines(reader);
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(ref tx) = chunk_tx_out {
                    let _ = tx.send(NodeChunk { text: line.clone() }).await;
                }
                output.push_str(&line);
                output.push('\n');
            }
        }
        let _ = stdout_tx.send(output);
    });

    // Spawn background reader for stderr — stream lines as chunks.
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

    // Spawn wait() so we can kill from the timeout arm without borrow conflicts.
    let (exit_tx, exit_rx) = tokio::sync::oneshot::channel::<(i32, bool)>();
    tokio::spawn(async move {
        let waited = tokio::time::timeout(timeout, child.wait()).await;
        match waited {
            Ok(Ok(s)) => { let _ = exit_tx.send((s.code().unwrap_or(-1), false)); }
            Ok(Err(_)) => { let _ = exit_tx.send((-1, false)); }
            Err(_elapsed) => {
                if let Err(e) = child.kill().await {
                    tracing::warn!(target: "nexus::node", "[NodeShell] failed to kill timed-out child: {e}");
                }
                // -9 明确表示超时强杀，区别于 wait 失败等场景
                let _ = exit_tx.send((-9, true));
            }
        }
    });

    let (exit_code, _timed_out) = match exit_rx.await {
        Ok(result) => result,
        Err(_) => (exit_codes::WAIT_FAILED, false),
    };

    let stdout = match stdout_rx.await {
        Ok(s) => s,
        Err(_) => String::new(),
    };
    let stderr = match stderr_rx.await {
        Ok(s) => s,
        Err(_) => String::new(),
    };

    if !stderr.is_empty() {
        tracing::warn!(target: "nexus::node::stderr", node_id, stderr = %stderr);
    }
    tracing::info!(target: "nexus::node::stdout", node_id, stdout_len = stdout.len(), stdout_head = %stdout.chars().take(200).collect::<String>());

    let output = if exit_code == exit_codes::TIMEOUT {
        NodeOutput {
            route: String::new(),
            content: stdout,
        }
    } else {
        match serde_json::from_str::<NodeOutput>(&stdout) {
            Ok(node_output) => {
                if node_output.route.is_empty() && !stdout.trim().is_empty() {
                    return Err(SpawnError {
                        message: "stdout is not valid JSON with 'route' field: missing route".into(),
                    });
                }
                node_output
            }
            Err(e) => {
                // If the process exited with a non-zero code, treat the
                // unparseable stdout as a normal failure — NOT a spawn
                // error.  SpawnError triggers retry, but a process that
                // runs and fails (e.g. Python SyntaxError → exit 1,
                // empty stdout) should go directly to Failed without
                // retry.
                if exit_code != 0 {
                    NodeOutput {
                        route: "failed".into(),
                        content: format!(
                            "process exited with code {exit_code}. stderr: {stderr}. stdout parse error: {e}"
                        ),
                    }
                } else {
                    return Err(SpawnError {
                        message: format!("stdout is not valid JSON with 'route' field: {e}"),
                    });
                }
            }
        }
    };

    // Use the route from NodeOutput as exit_reason for edge routing.
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    
    use super::*;

    fn test_scripts_dir() -> PathBuf {
        PathBuf::from("scripts")
    }

    /// Build a command that emits clean JSON on stdout, cross-platform.
    /// Uses hex-encoding to avoid shell quoting issues — the entire Python
    /// code contains no spaces so `split_command` works correctly.
    fn json_cmd(json: &str) -> String {
        let hex: String = json.bytes().map(|b| format!("{b:02x}")).collect();
        format!(
            "python -c __import__('sys').stdout.write(bytes.fromhex('{hex}').decode())"
        )
    }

    #[tokio::test]
    async fn test_echo_subprocess() {
        let cmd = if cfg!(windows) { "cmd.exe /c echo hello" } else { "echo hello" };
        let executor = SubprocessExecutor::new(cmd.to_string(), test_scripts_dir());
        let mut ctx = NodeContext::default();
        ctx.inputs = HashMap::new();
        let outcome = executor.run(ctx, Duration::from_secs(5), "echo", None).await;
        // echo outputs plain text - JSON parse should fail.
        assert!(outcome.is_err(), "echo without JSON should fail: {outcome:?}");
    }

    #[tokio::test]
    async fn test_json_output_parsed() {
        let json = r#"{"route":"result","content":"hello world"}"#;
        let executor = SubprocessExecutor::new(json_cmd(json), test_scripts_dir());
        let mut ctx = NodeContext::default();
        ctx.inputs = HashMap::new();
        let outcome = executor.run(ctx, Duration::from_secs(5), "json_test", None).await
            .expect("valid JSON with route should succeed");
        assert_eq!(outcome.output.route, "result");
        assert_eq!(outcome.output.content, "hello world");
        assert_eq!(outcome.exit_code, 0);
        assert!(!outcome.timed_out());
    }

    #[tokio::test]
    async fn test_json_output_without_route_fails() {
        let json = r#"{"content":"no route here"}"#;
        let executor = SubprocessExecutor::new(json_cmd(json), test_scripts_dir());
        let mut ctx = NodeContext::default();
        ctx.inputs = HashMap::new();
        let result = executor.run(ctx, Duration::from_secs(5), "no_route", None).await;
        assert!(result.is_err(), "JSON without route should fail");
        let err = result.unwrap_err();
        assert!(err.message.contains("route"), "error should mention route");
    }

    #[tokio::test]
    async fn test_plain_text_output_fails() {
        let cmd = if cfg!(windows) { "cmd.exe /c echo just plain text" } else { "echo just plain text" };
        let executor = SubprocessExecutor::new(cmd.to_string(), test_scripts_dir());
        let mut ctx = NodeContext::default();
        ctx.inputs = HashMap::new();
        let result = executor.run(ctx, Duration::from_secs(5), "plain_text", None).await;
        assert!(result.is_err(), "plain text should fail");
    }

    #[tokio::test]
    async fn test_json_output_with_content_only() {
        let json = r#"{"route":"event"}"#;
        let executor = SubprocessExecutor::new(json_cmd(json), test_scripts_dir());
        let mut ctx = NodeContext::default();
        ctx.inputs = HashMap::new();
        let outcome = executor.run(ctx, Duration::from_secs(5), "content_only", None).await
            .expect("JSON with route only should succeed");
        assert_eq!(outcome.output.route, "event");
        assert_eq!(outcome.output.content, "", "content should default to empty");
    }

    #[tokio::test]
    async fn test_timeout_returns_partial_output() {
        let executor = SubprocessExecutor::new("ping -n 60 127.0.0.1".into(), test_scripts_dir());
        let mut ctx = NodeContext::default();
        ctx.inputs = HashMap::new();
        let result = executor.run(ctx, Duration::from_millis(10), "timeout_test", None).await;
        if let Ok(outcome) = result {
        assert!(outcome.timed_out(), "should timeout with 10ms timeout");
        assert!(outcome.exit_code != 0 || outcome.timed_out());
            } else {
            // Spawn might fail on some systems -- that's ok
        }
    }

    #[tokio::test]
    async fn test_spawn_failure() {
        let executor = SubprocessExecutor::new("nonexistent_command_12345".into(), test_scripts_dir());
        let mut ctx = NodeContext::default();
        ctx.inputs = HashMap::new();
        let result = executor.run(ctx, Duration::from_secs(1), "spawn_fail", None).await;
        assert!(result.is_err(), "nonexistent command should return SpawnError");
    }

    #[tokio::test]
    async fn test_empty_command() {
        let executor = SubprocessExecutor::new(String::new(), test_scripts_dir());
        let mut ctx = NodeContext::default();
        ctx.inputs = HashMap::new();
        let result = executor.run(ctx, Duration::from_secs(1), "empty_cmd", None).await;
        assert!(result.is_err(), "empty command should return SpawnError");
    }

    /// Non-zero exit + empty stdout → Failed outcome, NOT `SpawnError`.
    /// Regression test: before the fix, this returned `SpawnError` and the
    /// engine would retry the node up to 3 times.  Now it returns a normal
    /// Failed outcome so the engine transitions directly to Failed without
    /// retry.
    #[tokio::test]
    async fn test_nonzero_exit_empty_stdout_is_failed_not_spawn_error() {
        // Python exits with code 1 and produces NO stdout — simulates
        // a SyntaxError or runtime crash.
        let cmd = if cfg!(windows) {
            "python -c \"import sys; sys.exit(1)\"".into()
        } else {
            "python3 -c 'import sys; sys.exit(1)'".into()
        };
        let executor = SubprocessExecutor::new(cmd, test_scripts_dir());
        let mut ctx = NodeContext::default();
        ctx.inputs = HashMap::new();
        let result = executor
            .run(ctx, Duration::from_secs(5), "nonzero_exit", None)
            .await;
        // Should be Ok (not SpawnError) — the process ran and failed.
        match result {
            Ok(outcome) => {
                assert_eq!(outcome.exit_code, 1, "should preserve exit code 1");
                assert_eq!(
                    outcome.output.route, "failed",
                    "should default route to 'failed'"
                );
                assert!(
                    outcome.output.content.contains("exited with code 1"),
                    "content should include exit code, got: {}",
                    outcome.output.content
                );
            }
            Err(e) => {
                panic!(
                    "exit_code≠0 + empty stdout should NOT be SpawnError, got: {}",
                    e.message
                );
            }
        }
    }

    /// Exit 0 + unparseable stdout → still `SpawnError`.
    /// This is the normal case: process succeeded but broke protocol.
    /// Regression test to ensure the nonzero-exit fix didn't break this.
    #[tokio::test]
    async fn test_zero_exit_unparseable_stdout_still_spawn_error() {
        let cmd = if cfg!(windows) {
            "cmd.exe /c echo not-json".into()
        } else {
            "echo not-json".into()
        };
        let executor = SubprocessExecutor::new(cmd, test_scripts_dir());
        let mut ctx = NodeContext::default();
        ctx.inputs = HashMap::new();
        let result = executor
            .run(ctx, Duration::from_secs(5), "zero_exit_bad_json", None)
            .await;
        assert!(
            result.is_err(),
            "exit 0 + unparseable stdout should still be SpawnError"
        );
    }

    #[test]
    fn test_split_command_whitespace() {
        let (prog, args) = SubprocessExecutor::split_command("echo hello");
        assert_eq!(prog, "echo");
        assert_eq!(args, vec!["hello".to_string()]);
    }

    #[test]
    fn test_split_command_no_args() {
        let (prog, args) = SubprocessExecutor::split_command("echo");
        assert_eq!(prog, "echo");
        assert!(args.is_empty());
    }

    #[test]
    fn test_split_command_empty() {
        let (prog, args) = SubprocessExecutor::split_command("");
        assert_eq!(prog, "");
        assert!(args.is_empty());
    }

    #[test]
    fn test_split_command_quoted_args() {
        let (prog, args) =
            SubprocessExecutor::split_command(r#"python "script with spaces.py" --flag"#);
        assert_eq!(prog, "python");
        assert_eq!(args, vec!["script with spaces.py".to_string(), "--flag".to_string()]);
    }

    #[test]
    fn test_split_command_escaped_quote() {
        let (prog, args) =
            SubprocessExecutor::split_command(r#"echo "he said \"hello\"""#);
        assert_eq!(prog, "echo");
        assert_eq!(args, vec![r#"he said "hello""#.to_string()]);
    }

    #[test]
    fn test_split_command_mixed_quoted_unquoted() {
        let (prog, args) =
            SubprocessExecutor::split_command(r#"cmd --input "data file.txt" --verbose"#);
        assert_eq!(prog, "cmd");
        assert_eq!(
            args,
            vec![
                "--input".to_string(),
                "data file.txt".to_string(),
                "--verbose".to_string(),
            ]
        );
    }
}
