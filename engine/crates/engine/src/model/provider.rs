use serde::{Deserialize, Serialize};

/// How a node is executed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProviderDef {
    /// Execute as a subprocess (direct spawn, no shell).
    #[serde(rename = "subprocess")]
    Subprocess {
        /// The command to execute.
        command: String,
    },
    /// Execute via a shell (cmd /c on Windows, sh -c on Unix).
    /// Supports pipes, redirects, and quoting.
    #[serde(rename = "shell")]
    Shell {
        /// The command to execute (will be wrapped in cmd /c or sh -c).
        command: String,
    },
    /// Execute via HTTP call (placeholder for future implementation).
    #[serde(rename = "http")]
    Http {
        /// The URL to call.
        url: String,
        /// HTTP method (defaults to POST).
        #[serde(default)]
        method: Option<String>,
    },
    /// Execute an LLM CLI (claude, opencode, etc.) as a subprocess.
    ///
    /// The engine renders `{{prompt}}` and `{{inputs.x}}` templates,
    /// spawns the command, streams stderr to the log (not routing),
    /// and parses stdout with JSON tolerance for routing.
    #[serde(rename = "llm")]
    Llm {
        /// Shell command template. `{{prompt}}` is replaced with the
        /// rendered prompt before execution.
        /// Examples:
        ///   `claude -p "{{prompt}}" --output-format json`
        ///   `opencode run --format json --auto -- "{{prompt}}"`
        command: String,
        /// Prompt template. `{{inputs.node_id}}` is replaced with
        /// upstream node outputs. If omitted, inputs are serialised
        /// as JSON and appended to the command.
        #[serde(default)]
        prompt: Option<String>,
        /// Expected route values. Injected into the prompt as guidance
        /// so the LLM knows which routes are valid.
        #[serde(default)]
        routes: Vec<String>,
        /// Maximum output tokens (passed to CLI if it supports the flag).
        /// `None` means "let the CLI use its default."
        #[serde(default)]
        max_tokens: Option<u64>,
    },
}
