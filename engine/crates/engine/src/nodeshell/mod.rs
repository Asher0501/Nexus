//! Node execution adapters.
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use crate::model::provider::ProviderDef;

mod llm;
mod llm_sdk;
pub(crate) mod http;
mod subprocess;
mod template;
mod types;

pub use llm::LlmExecutor;
pub use llm_sdk::LlmSdkExecutor;
pub use http::HttpExecutor;
pub use subprocess::SubprocessExecutor;
pub use template::TemplateEngine;
pub use types::{NodeChunk, NodeContext, NodeMetadata, NodeOutput, NodeOutcome, SpawnError};

/// Resolve the effective scripts directory for a node.
/// Precedence: node > workflow > `NEXUS_SCRIPTS_DIR` env > exe-search > \"scripts\" cwd.
#[must_use]
pub fn resolve_scripts_dir(
    node_scripts_dir: Option<&str>,
    workflow_scripts_dir: Option<&str>,
) -> PathBuf {
    if let Some(dir) = node_scripts_dir { return PathBuf::from(dir); }
    if let Some(dir) = workflow_scripts_dir { return PathBuf::from(dir); }
    if let Ok(dir) = std::env::var("NEXUS_SCRIPTS_DIR") && !dir.is_empty() { return PathBuf::from(dir); }
    if let Some(dir) = search_exe_relative_scripts(3) { return dir; }
    PathBuf::from("scripts")
}

fn search_exe_relative_scripts(max_levels: usize) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?.to_path_buf();
    for _ in 0..=max_levels {
        let scripts = dir.join("scripts");
        if scripts.exists() { return Some(scripts); }
        if !dir.pop() { break; }
    }
    None
}

/// Enum dispatch for node executors.
pub enum NodeExecutor {
    /// Subprocess executor.
    Subprocess(SubprocessExecutor),
    /// LLM CLI executor.
    Llm(LlmExecutor),
    /// LLM SDK executor.
    LlmSdk(LlmSdkExecutor),
    /// HTTP executor.
    Http(HttpExecutor),
}

impl NodeExecutor {
    /// Create a [`NodeExecutor`] from a [`ProviderDef`].
    #[must_use]
    pub fn from_provider(provider: &ProviderDef, scripts_dir: &std::path::Path) -> Self {
        match provider {
            ProviderDef::Subprocess { command } => {
                Self::Subprocess(SubprocessExecutor::new(command.clone(), scripts_dir.to_path_buf()))
            }
            ProviderDef::Shell { command } => {
                Self::Subprocess(SubprocessExecutor::new_shell(command.clone(), scripts_dir.to_path_buf()))
            }
            ProviderDef::Llm { command, prompt, routes, max_tokens } => {
                Self::Llm(LlmExecutor::new(command, prompt.clone(), routes.clone(), *max_tokens, scripts_dir.to_path_buf()))
            }
            ProviderDef::LlmSdk { model, api_key_env, system_prompt, prompt, routes, max_tokens } => {
                Self::LlmSdk(LlmSdkExecutor::new(model, api_key_env.clone(), system_prompt.clone(), prompt.clone(), routes.clone(), *max_tokens, scripts_dir.to_path_buf()))
            }
            ProviderDef::Http { .. } => Self::Http(HttpExecutor::from_provider(provider, scripts_dir)),
        }
    }

    /// Run the node with the given context and timeout.
    pub async fn run(
        &self, ctx: NodeContext, timeout: Duration, node_id: &str,
        chunk_tx: Option<mpsc::Sender<NodeChunk>>,
    ) -> Result<NodeOutcome, SpawnError> {
        match self {
            Self::Subprocess(exe) => exe.run(ctx, timeout, node_id, chunk_tx).await,
            Self::Llm(exe) => exe.run(ctx, timeout, node_id, chunk_tx).await,
            Self::LlmSdk(exe) => exe.run(ctx, timeout, node_id, chunk_tx).await,
            Self::Http(exe) => exe.run(ctx, timeout, node_id, chunk_tx).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_node_level_takes_precedence() {
        assert_eq!(resolve_scripts_dir(Some("./my_scripts"), Some("./global")), PathBuf::from("./my_scripts"));
    }
    #[test]
    fn resolve_workflow_level_when_node_not_set() {
        assert_eq!(resolve_scripts_dir(None, Some("./global")), PathBuf::from("./global"));
    }
    #[test]
    fn resolve_cwd_fallback() {
        let path = resolve_scripts_dir(None, None);
        assert!(path.ends_with("scripts"), "got: {path:?}");
    }
}
