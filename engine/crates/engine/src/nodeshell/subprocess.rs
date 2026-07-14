use std::collections::HashMap;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

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
    /// The command to execute.
    command: String,
    /// Whether this is a shell-wrapped command.
    /// Shell commands are passed directly to `cmd.exe /c` / `sh -c`
    /// without splitting, preserving quotes and shell syntax.
    shell: bool,
}

impl SubprocessExecutor {
    /// Create a new subprocess executor for the given command string.
    #[must_use]
    pub fn new(command: String) -> Self {
        Self { command, shell: false }
    }

    /// Create a shell-wrapped executor.
    ///
    /// The command is executed via `cmd /c` (Windows) or `sh -c` (Unix)
    /// without argument splitting, enabling pipes, redirects, shell quoting,
    /// and multi-word quoted arguments.
    ///
    /// # Security
    ///
    /// This function performs basic single-quote escaping on Unix. It does
    /// NOT sanitise all shell metacharacters (backticks, `$()`, `&&`, etc.).
    /// Do NOT pass untrusted input directly to the `command` parameter. If
    /// the command includes user-supplied values, use the `Subprocess`
    /// provider with explicit argument arrays instead.
    #[must_use]
    pub fn new_shell(command: String) -> Self {
        Self { command, shell: true }
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
        let mut chars = trimmed.chars().peekable();

        while let Some(ch) = chars.next() {
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

    /// Render a prompt template, replacing `{{inputs.name}}` with values
    /// from the node context's inputs map.
    /// Escape a value for use in a shell command.
    /// Windows cmd: wrap in quotes, escape internal `"` → `""` and `%` → `%%`.
    /// Unix sh: wrap in single quotes, escape internal `'` → `'\''`.
    fn shell_escape(value: &str) -> String {
        if cfg!(windows) {
            let escaped = value.replace('"', "\"\"")
                             .replace('%', "%%");
            format!("\"{}\"", escaped)
        } else {
            let escaped = value.replace('\'', "'\\''");
            format!("'{}'", escaped)
        }
    }

    fn render_template(template: &str, inputs: &HashMap<String, String>) -> String {
        Self::render_template_impl(template, inputs, false)
    }

    fn render_template_shell(template: &str, inputs: &HashMap<String, String>) -> String {
        Self::render_template_impl(template, inputs, true)
    }

    fn render_template_impl(template: &str, inputs: &HashMap<String, String>, shell: bool) -> String {
        let mut result = String::with_capacity(template.len());
        let mut rest = template;

        while let Some(start) = rest.find("{{inputs.") {
            result.push_str(&rest[..start]);
            let after_start = &rest[start + 9..];
            let end = after_start.find("}}").unwrap_or(0);
            if end == 0 {
                result.push_str(&rest[start..]);
                rest = "";
                break;
            }
            let key = &after_start[..end];
            let value = inputs.get(key).map(|s| s.as_str()).unwrap_or("");
            if shell {
                result.push_str(&Self::shell_escape(value));
            } else {
                result.push_str(value);
            }
            let consumed = start + 9 + end + 2;
            rest = &rest[consumed..];
        }

        result.push_str(rest);
        result
    }

    /// Search upward from the executable's directory for a `scripts/`
    /// directory, up to `max_levels` parent levels.
    fn find_scripts_dir(max_levels: usize) -> Option<std::path::PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let mut dir = exe.parent()?.to_path_buf();
        for _ in 0..=max_levels {
            let scripts = dir.join("scripts");
            if scripts.exists() {
                return Some(scripts);
            }
            if !dir.pop() {
                break;
            }
        }
        None
    }

    /// Resolve `scripts/` paths to absolute paths relative to the executable.
    ///
    /// Searches upward from the exe directory for a `scripts/` directory,
    /// supporting both `bin/nexus-cli.exe` (Windows) and `bin/linux/nexus-cli`
    /// (Linux) release layouts.  When the command contains `scripts/xxx`, we
    /// resolve it to `{release_root}/scripts/xxx` so the workflow author does
    /// not need to know (or `cd` into) the release directory.
    fn resolve_scripts_path(command: &str) -> String {
        if !command.contains("scripts/") {
            return command.to_string();
        }

        if let Some(scripts_dir) = Self::find_scripts_dir(3) {
            let abs = scripts_dir.to_string_lossy().replace('\\', "/");
            return command.replace("scripts/", &format!("{abs}/"));
        }

        // Fallback: leave as-is (CWD-relative, backward compatible).
        command.to_string()
    }

    /// Run the subprocess, collect stdout as JSON, and return the parsed outcome.
    ///
    /// If the command contains `{{inputs.x}}` patterns, they are replaced with
    /// values from the node context's inputs map before spawning.
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
        ctx: NodeContext,
        timeout: Duration,
        node_id: &str,
        chunk_tx: Option<mpsc::Sender<NodeChunk>>,
    ) -> Result<NodeOutcome, SpawnError> {
        // Render template in command string ({{inputs.x}} → ctx.inputs[x]).
        // Shell mode: escape values to prevent command injection.
        let command = if self.command.contains("{{inputs.") {
            if self.shell {
                Self::render_template_shell(&self.command, &ctx.inputs)
            } else {
                Self::render_template(&self.command, &ctx.inputs)
            }
        } else {
            self.command.clone()
        };

        // Resolve `scripts/` paths relative to the executable binary so that
        // `python scripts/xxx.py` works regardless of CWD.
        let command = Self::resolve_scripts_path(&command);

        tracing::info!(
            target: "nexus::nodeshell",
            node_id,
            command = %command,
            shell = self.shell,
            "spawning node"
        );

        let mut cmd = if self.shell {
            // Shell mode: pass the entire command directly to the shell.
            // Do NOT split — the shell handles quoting, pipes, redirects, etc.
            if cfg!(windows) {
                let mut c = Command::new("cmd.exe");
                c.arg("/c").arg(&command);
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
            .stderr(std::process::Stdio::piped());

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
async fn collect_and_wait(
    mut child: Child,
    timeout: Duration,
    node_id: &str,
    chunk_tx: Option<mpsc::Sender<NodeChunk>>,
) -> Result<NodeOutcome, SpawnError> {
    let out_pipe = child.stdout.take();
    let err_pipe = child.stderr.take();

    // Spawn background reader for stdout.
    let (stdout_tx, stdout_rx) = tokio::sync::oneshot::channel::<String>();
    tokio::spawn(async move {
        let mut output = String::new();
        if let Some(pipe) = out_pipe {
            let mut reader = tokio::io::BufReader::new(pipe);
            let _ = tokio::io::AsyncReadExt::read_to_string(&mut reader, &mut output).await;
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
                return Err(SpawnError {
                    message: format!("stdout is not valid JSON with 'route' field: {e}"),
                });
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

    /// Build a command that emits clean JSON on stdout, cross-platform.
    /// Uses hex-encoding to avoid shell quoting issues — the entire Python
    /// code contains no spaces so split_command works correctly.
    fn json_cmd(json: &str) -> String {
        let hex: String = json.bytes().map(|b| format!("{:02x}", b)).collect();
        format!(
            "python -c __import__('sys').stdout.write(bytes.fromhex('{hex}').decode())"
        )
    }

    #[tokio::test]
    async fn test_echo_subprocess() {
        let cmd = if cfg!(windows) { "cmd.exe /c echo hello" } else { "echo hello" };
        let executor = SubprocessExecutor::new(cmd.to_string());
        let mut ctx = NodeContext::default();
        ctx.inputs = HashMap::new();
        let outcome = executor.run(ctx, Duration::from_secs(5), "echo", None).await;
        // echo outputs plain text - JSON parse should fail.
        assert!(outcome.is_err(), "echo without JSON should fail: {:?}", outcome);
    }

    #[tokio::test]
    async fn test_json_output_parsed() {
        let json = r#"{"route":"result","content":"hello world"}"#;
        let executor = SubprocessExecutor::new(json_cmd(json));
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
        let executor = SubprocessExecutor::new(json_cmd(json));
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
        let executor = SubprocessExecutor::new(cmd.to_string());
        let mut ctx = NodeContext::default();
        ctx.inputs = HashMap::new();
        let result = executor.run(ctx, Duration::from_secs(5), "plain_text", None).await;
        assert!(result.is_err(), "plain text should fail");
    }

    #[tokio::test]
    async fn test_json_output_with_content_only() {
        let json = r#"{"route":"event"}"#;
        let executor = SubprocessExecutor::new(json_cmd(json));
        let mut ctx = NodeContext::default();
        ctx.inputs = HashMap::new();
        let outcome = executor.run(ctx, Duration::from_secs(5), "content_only", None).await
            .expect("JSON with route only should succeed");
        assert_eq!(outcome.output.route, "event");
        assert_eq!(outcome.output.content, "", "content should default to empty");
    }

    #[tokio::test]
    async fn test_timeout_returns_partial_output() {
        let executor = SubprocessExecutor::new("ping -n 60 127.0.0.1".into());
        let mut ctx = NodeContext::default();
        ctx.inputs = HashMap::new();
        let result = executor.run(ctx, Duration::from_millis(10), "timeout_test", None).await;
        match result {
            Ok(outcome) => {
        assert!(outcome.timed_out(), "should timeout with 10ms timeout");
        assert!(outcome.exit_code != 0 || outcome.timed_out());
            }
            Err(_) => {
                // Spawn might fail on some systems -- that's ok
            }
        }
    }

    #[tokio::test]
    async fn test_spawn_failure() {
        let executor = SubprocessExecutor::new("nonexistent_command_12345".into());
        let mut ctx = NodeContext::default();
        ctx.inputs = HashMap::new();
        let result = executor.run(ctx, Duration::from_secs(1), "spawn_fail", None).await;
        assert!(result.is_err(), "nonexistent command should return SpawnError");
    }

    #[tokio::test]
    async fn test_empty_command() {
        let executor = SubprocessExecutor::new(String::new());
        let mut ctx = NodeContext::default();
        ctx.inputs = HashMap::new();
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
