use serde::{Deserialize, Serialize};

/// How a node is executed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Execute via HTTP call.
    #[serde(rename = "http")]
    Http {
        /// The URL to call.  Supports template interpolation
        /// (`{{datarouter.*.*}}`, `{{metadata.*}}`).
        url: String,
        /// HTTP method (GET, POST, PUT, DELETE, PATCH).
        /// Defaults to GET if omitted.
        #[serde(default)]
        method: Option<String>,
        /// Optional HTTP headers (e.g. `Authorization`, `Content-Type`).
        /// Keys and values support template interpolation.
        #[serde(default)]
        headers: Option<std::collections::HashMap<String, String>>,
        /// Optional request body (UTF-8 string).
        /// Supports template interpolation.
        /// Default `Content-Type` is `application/json` when body is present.
        #[serde(default)]
        body: Option<String>,
    },
    /// Execute an LLM CLI (claude, opencode, etc.) as a subprocess.
    ///
    /// The engine renders `{{metadata.*}}` and `{{datarouter.*.*}}`
    /// templates, spawns the command, streams stderr to the log
    /// (not routing), and parses stdout with JSON tolerance for routing.
    #[serde(rename = "llm")]
    Llm {
        /// Shell command template. `{{prompt}}` is replaced with the
        /// rendered prompt before execution.
        /// Examples:
        ///   `claude -p "{{prompt}}" --output-format json`
        ///   `opencode run --format json --auto -- "{{prompt}}"`
        command: String,
        /// Prompt template. `{{metadata.*}}` and `{{datarouter.*.*}}`
        /// are rendered by the engine. If omitted, inputs are serialised
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
    /// Execute an LLM via the Anthropic Python SDK.
    #[serde(rename = "llm_sdk")]
    LlmSdk {
        /// The model identifier to use.
        model: String,
        /// Environment variable containing the API key.
        #[serde(default)]
        api_key_env: Option<String>,
        /// System-level prompt template.
        #[serde(default)]
        system_prompt: Option<String>,
        /// Prompt template. `{{metadata.*}}` and `{{datarouter.*.*}}`
        /// are rendered by the engine.
        #[serde(default)]
        prompt: Option<String>,
        /// Expected route values. Injected into the prompt as guidance
        /// so the LLM knows which routes are valid.
        #[serde(default)]
        routes: Vec<String>,
        /// Maximum output tokens.
        /// `None` means "let the API use its default."
        #[serde(default)]
        max_tokens: Option<u64>,
    },
}
