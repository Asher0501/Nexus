use std::time::Duration;

use serde::{Deserialize, Serialize};

fn default_node_timeout() -> u64 {
    3600
}

fn default_max_retries() -> u64 {
    3
}

/// Runtime engine configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineConfig {
    /// Maximum number of nodes that can run concurrently.
    /// If None, defaults to CPU core count at runtime.
    pub max_concurrency: Option<usize>,

    /// Default timeout per node in seconds.
    /// A node's `process_timeout_secs` overrides this value.
    /// Defaults to 3600.
    #[serde(default = "default_node_timeout")]
    pub default_node_timeout_secs: u64,

    /// Maximum number of retries for a failed node.
    /// Defaults to 3.
    #[serde(default = "default_max_retries")]
    pub max_retries: u64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_concurrency: None,
            default_node_timeout_secs: default_node_timeout(),
            max_retries: default_max_retries(),
        }
    }
}

impl EngineConfig {
    /// Create a new `EngineConfig` with explicit values.
    pub fn new(
        max_concurrency: Option<usize>,
        default_node_timeout_secs: u64,
        max_retries: u64,
    ) -> Self {
        Self {
            max_concurrency,
            default_node_timeout_secs,
            max_retries,
        }
    }

    /// Get the default node timeout as a [`Duration`].
    pub fn node_timeout(&self) -> Duration {
        Duration::from_secs(self.default_node_timeout_secs)
    }

    /// Get the effective max concurrency, using system CPU count if not set.
    pub fn effective_max_concurrency(&self) -> usize {
        self.max_concurrency
            .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4))
    }
}
