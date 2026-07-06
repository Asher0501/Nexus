use serde::{Deserialize, Serialize};

/// How a node is executed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProviderDef {
    /// Execute as a subprocess.
    #[serde(rename = "subprocess")]
    Subprocess {
        /// The command to execute.
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
}
