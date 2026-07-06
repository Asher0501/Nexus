use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use petgraph::graph::NodeIndex;
use tokio::sync::{mpsc, Semaphore};

use crate::diagnostics::event::{self, EngineLifecycleEvent, NodeLifecycleEvent};
use crate::graph::{DataRouter, Scheduler};
use crate::model::{EngineConfig, EventType, ValidationError, WorkflowDef};
use crate::nodeshell::{NodeChunk, NodeContext, NodeExecutor, SpawnError};

/// Internal event type for the engine's event loop.
#[derive(Debug)]
enum EngineEvent {
    /// A node is ready to be executed.
    NodeReady {
        /// The node's graph index.
        node_id: NodeIndex,
    },
}

/// The top-level workflow execution engine.
///
/// The engine owns the [`Scheduler`], [`DataRouter`], and an event loop that
/// drives node execution via `mpsc` channel events.
///
/// # Execution concurrency
///
/// The engine uses a [`Semaphore`] to limit the number of concurrently
/// executing nodes.  When a `NodeReady` event arrives, the engine must
/// `acquire` a permit before spawning the subprocess.  If all permits
/// are taken, the event handler awaits until one becomes available.
#[derive(Debug)]
pub struct Engine {
    /// Graph execution scheduler (runtime state).
    scheduler: Scheduler,
    /// Data router for upstream-to-downstream data flow.
    data_router: DataRouter,
    /// Runtime configuration.
    config: EngineConfig,
    /// Number of currently running nodes.
    running_count: usize,
    /// Concurrency limiter — permits equal `max_concurrency`.
    semaphore: Arc<Semaphore>,
    /// Sender half of the internal event channel.
    event_tx: mpsc::UnboundedSender<EngineEvent>,
    /// Receiver half of the internal event channel.
    event_rx: mpsc::UnboundedReceiver<EngineEvent>,
}

impl Engine {
    /// Look up the node's workflow-definition ID from its graph index.
    fn node_id(&self, idx: NodeIndex) -> String {
        self.scheduler
            .graph()
            .node_weight(idx)
            .map(|nd| nd.id.clone())
            .unwrap_or_default()
    }

    /// Create a new `Engine` from a [`WorkflowDef`] and [`EngineConfig`].
    ///
    /// # Errors
    ///
    /// Returns a vector of [`ValidationError`]s if the workflow definition
    /// cannot be built into a valid graph.
    pub fn new(def: WorkflowDef, config: EngineConfig) -> Result<Self, Vec<ValidationError>> {
        let graph = crate::graph::Builder::build(&def)?;
        let node_id_to_index: HashMap<String, NodeIndex> = graph
            .node_indices()
            .filter_map(|idx| {
                graph
                    .node_weight(idx)
                    .map(|nd| (nd.id.clone(), idx))
            })
            .collect();
        let data_router = DataRouter::new(node_id_to_index, &def.dataflows);
        let scheduler = Scheduler::new(graph);

        let max_permits = config.effective_max_concurrency();
        let semaphore = Arc::new(Semaphore::new(max_permits));

        let (tx, rx) = mpsc::unbounded_channel();

        Ok(Self {
            scheduler,
            data_router,
            config,
            running_count: 0,
            semaphore,
            event_tx: tx,
            event_rx: rx,
        })
    }

    /// Run the workflow to completion.
    ///
    /// The main event loop processes `NodeReady` events until the scheduler
    /// signals convergence (all nodes in a terminal state, queue empty).
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::IdleTimeout`] if the event loop is idle too long.
    pub async fn run(&mut self) -> Result<WorkflowResult, RuntimeError> {
        event::emit_engine(&EngineLifecycleEvent::Started {
            node_count: self.scheduler.graph().node_count(),
            max_concurrency: self.config.effective_max_concurrency(),
            default_timeout_secs: self.config.default_node_timeout_secs,
        });

        // Seed the scheduler with entry nodes.
        self.scheduler.enqueue_entries();

        // Publish initial NodeReady events for any enqueued entries.
        while let Some(node) = self.scheduler.dequeue() {
            let _ = self.event_tx.send(EngineEvent::NodeReady { node_id: node });
        }

        let node_timeout = self.config.node_timeout();
        let started_at = std::time::Instant::now();

        loop {
            // Check convergence: no running nodes, nothing in the queue, all done.
            if self.running_count == 0 && self.scheduler.is_converged() {
                break;
            }

            tokio::select! {
                event = self.event_rx.recv() => {
                    match event {
                        Some(e) => self.handle_event(e).await,
                        // Channel closed (all senders dropped) — no more events.
                        None => break,
                    }
                }
                _ = tokio::time::sleep(node_timeout) => {
                    event::emit_engine(&EngineLifecycleEvent::TimedOut {
                        duration: started_at.elapsed(),
                    });
                    return Err(RuntimeError::IdleTimeout);
                }
            }
        }

        event::emit_engine(&EngineLifecycleEvent::Converged {
            duration: started_at.elapsed(),
        });
        Ok(WorkflowResult {})
    }

    /// Process a single engine event.
    ///
    /// 1. Acquire a concurrency permit (may await if at `max_concurrency`).
    /// 2. Extract node data while holding `&mut self`.
    /// 3. Execute the node (relinquishes the `&mut self` borrow during I/O).
    /// 4. Handle outcome, emit metrics, propagate events.
    /// 5. Release the permit (via `_permit` drop).
    async fn handle_event(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::NodeReady { node_id } => {
                let nid = self.node_id(node_id);

                // Step 1: acquire a concurrency slot (await if all busy).
                let _permit = self
                    .semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("semaphore closed");
                self.running_count += 1;

                // Step 2: extract data while &mut self is available.
                let provider = self
                    .scheduler
                    .graph()
                    .node_weight(node_id)
                    .and_then(|nd| nd.providers.first());

                let command_label = match provider {
                    Some(p) => format!("{p:?}"),
                    None => String::new(),
                };

                let timeout = self
                    .scheduler
                    .graph()
                    .node_params(node_id)
                    .map(|p| Duration::from_secs(p.process_timeout_secs))
                    .unwrap_or(Duration::from_secs(30));

                event::emit_lifecycle(&NodeLifecycleEvent::Running {
                    node_id: nid.clone(),
                    command: command_label,
                });

                let inputs = self.data_router.build_input(node_id);

                let max_retries = self.config.max_retries;
                let tx = self.event_tx.clone();

                // Delegate executor creation to NodeShell — engine does not
                // know about specific provider variants.
                let executor = provider
                    .map(NodeExecutor::from_provider)
                    .unwrap_or_else(|| {
                        // Fallback: spawn with empty command (will fail).
                        NodeExecutor::from_provider(&crate::model::provider::ProviderDef::Subprocess {
                            command: String::new(),
                        })
                    });
                let ctx = NodeContext {
                    inputs,
                    extensions: HashMap::new(),
                };

                // Step 3: execute (I/O — &mut self is not held during this await).
                // Streams stdout/stderr via tracing in real time.
                // Create a chunk channel for real-time streaming output.
                let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::unbounded_channel::<NodeChunk>();
                let chunk_node_id = nid.clone();

                // Spawn a consumer that forwards chunks to diagnostics.
                tokio::spawn(async move {
                    while let Some(chunk) = chunk_rx.recv().await {
                        tracing::info!(
                            target: "nexus::node::chunk",
                            node_id = chunk_node_id,
                            text = chunk.text,
                        );
                    }
                });

                let outcome = executor.run(ctx, timeout, &nid, Some(chunk_tx)).await;

                // Step 4: process outcome.
                match outcome {
                    Ok(outcome) => {
                        let event_type = if outcome.timed_out {
                            EventType::Timeout
                        } else if outcome.exit_code == 0 {
                            EventType::Complete
                        } else {
                            EventType::Failed
                        };

                        // Store output.
                        self.data_router.store_output(node_id, &outcome.output);

                        // Handle retry logic for failures and timeouts.
                        if matches!(event_type, EventType::Failed | EventType::Timeout) {
                            let retry_count = self
                                .scheduler
                                .state()
                                .retry_counts
                                .get(&node_id)
                                .copied()
                                .unwrap_or(0);

                            if self.scheduler.retry_node(node_id, max_retries) {
                                event::emit_lifecycle(&NodeLifecycleEvent::Failed {
                                    node_id: nid,
                                    exit_reason: outcome
                                        .exit_reason
                                        .clone()
                                        .unwrap_or_else(|| "retrying".into()),
                                    retry_count,
                                });
                                // Retry scheduled — re-enqueue.
                                let _ = tx.send(EngineEvent::NodeReady { node_id });
                                self.running_count -= 1;
                                return;
                            }
                        }

                        // Emit lifecycle event for non-retried outcome.
                        match event_type {
                            EventType::Complete => {
                                event::emit_lifecycle(&NodeLifecycleEvent::Completed {
                                    node_id: nid,
                                    output_size: outcome.output.len(),
                                });
                            }
                            EventType::Failed => {
                                let reason = outcome
                                    .exit_reason
                                    .clone()
                                    .unwrap_or_else(|| "failed".into());
                                event::emit_lifecycle(&NodeLifecycleEvent::Failed {
                                    node_id: nid,
                                    exit_reason: reason.clone(),
                                    retry_count: self
                                        .scheduler
                                        .state()
                                        .retry_counts
                                        .get(&node_id)
                                        .copied()
                                        .unwrap_or(0),
                                });
                            }
                            EventType::Timeout => {
                                event::emit_lifecycle(&NodeLifecycleEvent::TimedOut {
                                    node_id: nid,
                                    timeout_secs: timeout.as_secs(),
                                });
                            }
                        }

                        // Process events through scheduler.
                        let ready_nodes = self.scheduler.handle_event(
                            node_id,
                            event_type,
                            outcome.exit_reason.as_deref(),
                        );

                        for target in ready_nodes {
                            let _ = tx.send(EngineEvent::NodeReady { node_id: target });
                        }
                    }
                    Err(_e) => {
                        event::emit_lifecycle(&NodeLifecycleEvent::Failed {
                            node_id: nid,
                            exit_reason: "spawn_error".into(),
                            retry_count: 0,
                        });
                        // Spawn failed — treat as Failed with no retry.
                        let ready_nodes = self.scheduler.handle_event(
                            node_id,
                            EventType::Failed,
                            Some("spawn_error"),
                        );
                        for target in ready_nodes {
                            let _ = tx.send(EngineEvent::NodeReady { node_id: target });
                        }
                    }
                }

                // Step 5: permit drops here → semaphore slot freed.
                self.running_count -= 1;
            }
        }
    }

    /// Get a reference to the scheduler (for diagnostics / snapshot).
    #[must_use]
    pub fn scheduler(&self) -> &Scheduler {
        &self.scheduler
    }

    /// Get the number of currently running nodes.
    #[must_use]
    pub fn running_count(&self) -> usize {
        self.running_count
    }
}

/// Result of a completed workflow run.
#[derive(Debug, Clone)]
pub struct WorkflowResult {}

/// Runtime errors that can occur during workflow execution.
#[derive(Debug)]
pub enum RuntimeError {
    /// The event loop was idle beyond the default node timeout.
    ///
    /// This fires when no events arrive on the channel for `default_node_timeout_secs`,
    /// indicating the workflow made no progress (e.g., a subprocess hung or the graph
    /// stalled).  It is distinct from a per-node `process_timeout`, which is enforced
    /// inside [`SubprocessExecutor::run`].
    IdleTimeout,
    /// A node could not be spawned for execution.
    SpawnError(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeError::IdleTimeout => {
                write!(f, "event loop idle timeout")
            }
            RuntimeError::SpawnError(msg) => {
                write!(f, "spawn error: {}", msg)
            }
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<SpawnError> for RuntimeError {
    fn from(err: SpawnError) -> Self {
        RuntimeError::SpawnError(err.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::WorkflowDef;

    /// A simple inline workflow definition for testing: A → B (chain).
    fn chain_workflow() -> WorkflowDef {
        use crate::model::predecessor::{EventType, SchedulingEdgeDef, TriggerExpr};
        use crate::model::provider::ProviderDef;

        WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: "cmd.exe".into(),
                    }],
                    process_timeout_secs: 10,
                    max_concurrency: None,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: "cmd.exe".into(),
                    }],
                    process_timeout_secs: 10,
                    max_concurrency: None,
                    returns: vec![],
                    max_retries: None,
                },
            ],
            edges: vec![SchedulingEdgeDef {
                from: "A".into(),
                to: "B".into(),
                trigger: TriggerExpr::All,
                event: EventType::Complete,
                exit_reason: None,
                threshold: 1,
            }],
            dataflows: vec![],
        }
    }

    #[tokio::test]
    async fn test_engine_new_success() {
        let def = chain_workflow();
        let config = EngineConfig::default();
        let engine = Engine::new(def, config);
        assert!(engine.is_ok(), "Engine::new should succeed for valid workflow");
    }

    #[tokio::test]
    async fn test_engine_new_invalid_workflow() {
        // Empty graph — no entry node, but still valid to build (just no nodes).
        let def = WorkflowDef {
            nodes: vec![],
            edges: vec![],
            dataflows: vec![],
        };
        let config = EngineConfig::default();
        let engine = Engine::new(def, config);
        // Empty graph passes validation (no errors), builds fine.
        assert!(engine.is_ok(), "empty workflow should be valid");
    }

    #[tokio::test]
    async fn test_engine_new_duplicate_id_fails() {
        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "X".into(),
                    providers: vec![],
                    process_timeout_secs: 10,
                    max_concurrency: None,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "X".into(),
                    providers: vec![],
                    process_timeout_secs: 10,
                    max_concurrency: None,
                    returns: vec![],
                    max_retries: None,
                },
            ],
            edges: vec![],
            dataflows: vec![],
        };
        let config = EngineConfig::default();
        let engine = Engine::new(def, config);
        assert!(engine.is_err(), "duplicate node ID should fail Engine::new");
    }

    #[tokio::test]
    async fn test_engine_run_empty_converges_immediately() {
        let def = WorkflowDef {
            nodes: vec![],
            edges: vec![],
            dataflows: vec![],
        };
        let config = EngineConfig::new(None, 3600, 3);
        let mut engine = Engine::new(def, config).expect("empty workflow");
        let result = engine.run().await;
        assert!(result.is_ok(), "empty workflow should converge immediately");
    }

    #[tokio::test]
    async fn test_engine_config_defaults_used() {
        let config = EngineConfig::new(Some(2), 7200, 5);
        assert_eq!(config.effective_max_concurrency(), 2);
        assert_eq!(config.node_timeout(), Duration::from_secs(7200));
    }
}
