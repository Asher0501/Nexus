//! Node execution adapters.
//!
//! This module provides types for executing workflow nodes:
//! - [`NodeContext`]: Input data passed to a node
//! - [`NodeOutcome`]: Result produced by node execution
//! - [`SubprocessExecutor`]: Runs a command as a subprocess
//! - [`NodeExecutor`]: Enum dispatch over executor types

use std::time::Duration;

use tokio::sync::mpsc::Sender;

use crate::model::provider::ProviderDef;

mod subprocess;
mod types;

pub use subprocess::SubprocessExecutor;
pub use types::{NodeChunk, NodeContext, NodeOutcome, SpawnError};

/// Enum dispatch for node executors.
pub enum NodeExecutor {
    /// Execute as a subprocess.
    Subprocess(SubprocessExecutor),
    /// Execute via HTTP (placeholder).
    Http(()),
}

impl NodeExecutor {
    /// Create a [`NodeExecutor`] from a [`ProviderDef`].
    ///
    /// The engine does not need to know about specific provider variants —
    /// it just calls this factory.
    #[must_use]
    pub fn from_provider(provider: &ProviderDef) -> Self {
        match provider {
            ProviderDef::Subprocess { command } => {
                NodeExecutor::Subprocess(SubprocessExecutor::new(command.clone()))
            }
            ProviderDef::Http { .. } => NodeExecutor::Http(()),
        }
    }

    /// Run the node with the given context and timeout.
    ///
    /// If `chunk_tx` is provided, real-time output lines are sent through
    /// the channel as they arrive from the subprocess. The full output
    /// is also accumulated in [`NodeOutcome::output`].
    ///
    /// # Errors
    ///
    /// Returns [`SpawnError`] when execution cannot be started.
    pub async fn run(
        &self,
        ctx: NodeContext,
        timeout: Duration,
        node_id: &str,
        chunk_tx: Option<Sender<NodeChunk>>,
    ) -> Result<NodeOutcome, SpawnError> {
        match self {
            NodeExecutor::Subprocess(exe) => {
                exe.run(ctx, timeout, node_id, chunk_tx).await
            }
            NodeExecutor::Http(_) => Err(SpawnError {
                message: "HTTP executor not implemented".into(),
            }),
        }
    }
}
