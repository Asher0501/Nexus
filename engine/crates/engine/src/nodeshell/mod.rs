//! Node execution adapters.
//!
//! This module provides types for executing workflow nodes:
//! - [`NodeContext`]: Input data passed to a node
//! - [`NodeOutput`]: Structured output from a node
//! - [`NodeOutcome`]: Result produced by node execution
//! - [`SubprocessExecutor`]: Runs a command as a subprocess
//! - [`LlmExecutor`]: Runs an LLM CLI with template rendering and tolerant parsing
//! - [`NodeExecutor`]: Enum dispatch over executor types

use std::time::Duration;

use tokio::sync::mpsc;

use crate::model::provider::ProviderDef;

mod llm;
mod subprocess;
mod types;

pub use llm::LlmExecutor;
pub use subprocess::SubprocessExecutor;
pub use types::{NodeChunk, NodeContext, NodeMetadata, NodeOutput, NodeOutcome, SpawnError};

/// Enum dispatch for node executors.
pub enum NodeExecutor {
    /// Execute as a subprocess.
    Subprocess(SubprocessExecutor),
    /// Execute an LLM CLI with template rendering.
    Llm(LlmExecutor),
    /// Execute via HTTP (placeholder).
    Http(()),
}

impl NodeExecutor {
    /// Create a [`NodeExecutor`] from a [`ProviderDef`].
    #[must_use]
    pub fn from_provider(provider: &ProviderDef) -> Self {
        match provider {
            ProviderDef::Subprocess { command } => {
                NodeExecutor::Subprocess(SubprocessExecutor::new(command.clone()))
            }
            ProviderDef::Shell { command } => {
                NodeExecutor::Subprocess(SubprocessExecutor::new_shell(command.clone()))
            }
            ProviderDef::Llm { command, prompt, routes, max_tokens } => {
                NodeExecutor::Llm(LlmExecutor::new(
                    command,
                    prompt.clone(),
                    routes.clone(),
                    *max_tokens,
                ))
            }
            ProviderDef::Http { .. } => NodeExecutor::Http(()),
        }
    }

    /// Run the node with the given context and timeout.
    pub async fn run(
        &self,
        ctx: NodeContext,
        timeout: Duration,
        node_id: &str,
        chunk_tx: Option<mpsc::Sender<NodeChunk>>,
    ) -> Result<NodeOutcome, SpawnError> {
        match self {
            NodeExecutor::Subprocess(exe) => {
                exe.run(ctx, timeout, node_id, chunk_tx).await
            }
            NodeExecutor::Llm(exe) => {
                exe.run(ctx, timeout, node_id, chunk_tx).await
            }
            NodeExecutor::Http(_) => Err(SpawnError {
                message: "HTTP executor not implemented".into(),
            }),
        }
    }
}
