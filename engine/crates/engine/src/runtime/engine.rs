use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use petgraph::graph::NodeIndex;
use tokio::sync::{mpsc, Semaphore};

use crate::diagnostics::event::{self, EngineLifecycleEvent, NodeLifecycleEvent};
use crate::diagnostics::snapshot::EngineSnapshot;
use crate::graph::{DataRouter, Scheduler};
use crate::model::{EngineConfig, EventType, ValidationError, WorkflowDef};
use crate::nodeshell::{NodeChunk, NodeContext, NodeExecutor, NodeOutcome, SpawnError};

/// Callback for real-time node lifecycle events consumed by the CLI.
///
/// The engine emits these events alongside the existing tracing events.
/// CLI layers use this to drive a live status display.
pub type NodeEventCb = Arc<dyn Fn(NodeEvent) + Send + Sync>;

/// Node lifecycle event consumed by the CLI status display.
#[derive(Debug, Clone)]
pub enum NodeEvent {
    /// A node started execution.
    NodeRunning {
        /// The node's ID.
        node_id: String,
        /// The command being executed (debug representation).
        command: String,
    },
    /// A node produced a line of output.
    NodeChunk {
        /// The node's ID.
        node_id: String,
        /// The output text line.
        text: String,
    },
    /// A node completed successfully.
    NodeCompleted {
        /// The node's ID.
        node_id: String,
    },
    /// A node failed.
    NodeFailed {
        /// The node's ID.
        node_id: String,
    },
    /// A node timed out.
    NodeTimedOut {
        /// The node's ID.
        node_id: String,
    },
    /// Generic lifecycle event (timeout, spawn failure, etc.).
    Lifecycle(NodeLifecycleEvent),
}

/// Internal event type for the engine's event loop.
#[derive(Debug)]
enum EngineEvent {
    /// A node is ready to be executed.
    NodeReady {
        /// The node's graph index.
        node_id: NodeIndex,
    },
    /// A node execution completed (dispatched from a spawned task).
    NodeCompleted {
        /// The node's graph index.
        node_id: NodeIndex,
        /// The outcome of execution.
        outcome: Result<NodeOutcome, SpawnError>,
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
    event_tx: mpsc::Sender<EngineEvent>,
    /// Receiver half of the internal event channel.
    event_rx: mpsc::Receiver<EngineEvent>,
    /// Optional callback for real-time node lifecycle events.
    event_cb: Option<NodeEventCb>,
}

impl fmt::Debug for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Engine")
            .field("scheduler", &self.scheduler)
            .field("config", &self.config)
            .field("running_count", &self.running_count)
            .field("event_cb", &self.event_cb.as_ref().map(|_| "Some(...)"))
            .finish_non_exhaustive()
    }
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
    /// Returns a vector of [`ValidationError]s if the workflow definition
    /// cannot be built into a valid graph.
    pub fn new(
        def: WorkflowDef,
        config: EngineConfig,
        event_cb: Option<NodeEventCb>,
    ) -> Result<Self, Vec<ValidationError>> {
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

        let (tx, rx) = mpsc::channel(512);

        Ok(Self {
            scheduler,
            data_router,
            config,
            running_count: 0,
            semaphore,
            event_tx: tx,
            event_rx: rx,
            event_cb,
        })
    }

    /// Run the workflow to completion.
    ///
    /// The main event loop processes `NodeReady` events until the scheduler
    /// signals convergence (all nodes in a terminal state, queue empty).
    pub async fn run(&mut self) -> Result<WorkflowResult, RuntimeError> {
        event::emit_engine(&EngineLifecycleEvent::Started {
            node_count: self.scheduler.graph().node_count(),
            max_concurrency: self.config.effective_max_concurrency(),
            default_timeout_secs: self.config.default_node_timeout_secs,
        });

        // Seed entry nodes.
        for &entry in self.scheduler.graph().entry_nodes() {
            let _ = self.event_tx.send(EngineEvent::NodeReady { node_id: entry }).await;
        }

        let started_at = std::time::Instant::now();

        loop {
            // Check convergence: no running nodes, nothing in the queue, all done.
            if self.running_count == 0 && self.scheduler.is_converged() {
                break;
            }

            match self.event_rx.recv().await {
                Some(e) => self.handle_event(e).await,
                // Channel closed (all senders dropped) — no more events.
                None => break,
            }
        }

        event::emit_engine(&EngineLifecycleEvent::Converged {
            duration: started_at.elapsed(),
        });
        let snapshot = EngineSnapshot::capture(&self.scheduler, self.running_count, started_at);
        Ok(WorkflowResult { snapshot })
    }

    /// Process a single engine event.
    ///
    /// 1. Acquire a concurrency permit (may await if at `max_concurrency`).
    /// 2. Extract node data while holding `&mut self`.
    /// 3. Spawn node execution (does not block the event loop).
    /// 4. Outcome handling is done via `NodeCompleted` event.
    async fn handle_event(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::NodeReady { node_id } => {
                let nid = self.node_id(node_id);

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
                    command: command_label.clone(),
                });

                if let Some(ref cb) = self.event_cb {
                    cb(NodeEvent::NodeRunning {
                        node_id: nid.clone(),
                        command: command_label,
                    });
                }

                let inputs = self.data_router.build_input(node_id);

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

                // 设置节点状态为 Running
                if let Some(ns) = self.scheduler.state_mut().states.get_mut(&node_id) {
                    ns.status = crate::graph::NodeStatus::Running;
                }

                // Step 3: spawn execution in a background task so the event
                // loop can process other NodeReady events concurrently.
                let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::channel::<NodeChunk>(256);
                let chunk_node_id = nid.clone();

                // Spawn a consumer that forwards chunks to diagnostics and callbacks.
                let chunk_cb = self.event_cb.clone();
                tokio::spawn(async move {
                    while let Some(chunk) = chunk_rx.recv().await {
                        tracing::info!(
                            target: "nexus::node::chunk",
                            node_id = chunk_node_id,
                            text = chunk.text,
                        );
                        if let Some(ref cb) = chunk_cb {
                            cb(NodeEvent::NodeChunk {
                                node_id: chunk_node_id.clone(),
                                text: chunk.text,
                            });
                        }
                    }
                });

                // Spawn the actual node execution; outcome comes back via event.
                let outcome_tx = tx.clone();
                let semaphore = self.semaphore.clone();
                tokio::spawn(async move {
                    let _permit = semaphore.acquire_owned().await.expect("semaphore closed");
                    let outcome = executor.run(ctx, timeout, &nid, Some(chunk_tx)).await;
                    let _ = outcome_tx.send(EngineEvent::NodeCompleted {
                        node_id,
                        outcome,
                    }).await;
                });
            }

            EngineEvent::NodeCompleted { node_id, outcome } => {
                let nid = self.node_id(node_id);

                let should_retry = match &outcome {
                    Ok(outcome) if outcome.timed_out => {
                        let ev = NodeLifecycleEvent::TimedOut {
                            node_id: nid.clone(),
                            timeout_secs: self.config.default_node_timeout_secs,
                        };
                        event::emit_lifecycle(&ev);
                        if let Some(ref cb) = self.event_cb {
                            cb(NodeEvent::Lifecycle(ev));
                        }
                        self.scheduler.retry_node(node_id, self.config.max_timeout_retries)
                    }
                    Err(_) => {
                        let ev = NodeLifecycleEvent::Failed {
                            node_id: nid.clone(),
                            exit_reason: "spawn_error".into(),
                            retry_count: 0,
                        };
                        event::emit_lifecycle(&ev);
                        if let Some(ref cb) = self.event_cb {
                            cb(NodeEvent::Lifecycle(ev));
                        }
                        self.scheduler.retry_node(node_id, self.config.max_timeout_retries)
                    }
                    _ => false,
                };

                if should_retry {
                    self.data_router.clear_output(node_id);
                    let _ = self.event_tx.send(EngineEvent::NodeReady { node_id }).await;
                } else {
                    if let Ok(outcome) = &outcome {
                        self.data_router.store_output(node_id, &outcome.output);

                        let event_type = if outcome.timed_out {
                            EventType::Timeout
                        } else if outcome.exit_code == 0 {
                            EventType::Complete
                        } else {
                            EventType::Failed
                        };

                        self.scheduler.handle_event(
                            node_id,
                            event_type,
                            outcome.exit_reason.as_deref(),
                        );

                        match event_type {
                            EventType::Complete => {
                                let ev = NodeLifecycleEvent::Completed {
                                    node_id: nid.clone(),
                                    output_size: outcome.output.len(),
                                };
                                event::emit_lifecycle(&ev);
                                if let Some(ref cb) = self.event_cb {
                                    cb(NodeEvent::NodeCompleted { node_id: nid.clone() });
                                }
                            }
                            EventType::Failed => {
                                let retry_count = self
                                    .scheduler
                                    .state()
                                    .retry_counts
                                    .get(&node_id)
                                    .copied()
                                    .unwrap_or(0);
                                let reason = outcome.exit_reason.clone().unwrap_or_else(|| "failed".into());
                                let ev = NodeLifecycleEvent::Failed {
                                    node_id: nid.clone(),
                                    exit_reason: reason,
                                    retry_count,
                                };
                                event::emit_lifecycle(&ev);
                                if let Some(ref cb) = self.event_cb {
                                    cb(NodeEvent::NodeFailed { node_id: nid.clone() });
                                }
                            }
                            EventType::Timeout => {
                                // Already emitted above; this arm is hit only when retries exhausted.
                            }
                        }

                        // scheduler.handle_event has already placed ready nodes
                        // into the scheduler's ready_queue. Drain them here.
                        for target in self.scheduler.dequeue_all() {
                            let _ = self.event_tx.send(EngineEvent::NodeReady { node_id: target }).await;
                        }
                    } else {
                        if let Some(ns) = self.scheduler.state_mut().states.get_mut(&node_id) {
                            ns.status = crate::graph::scheduler::NodeStatus::Failed;
                        }
                    }
                }

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

    /// Access the data router for testing / diagnostics.
    #[must_use]
    pub fn data_router(&self) -> &DataRouter {
        &self.data_router
    }
}

/// Result of a completed workflow run.
#[derive(Debug, Clone)]
pub struct WorkflowResult {
    /// Snapshot of engine state at completion (node statuses, elapsed time, etc.).
    pub snapshot: EngineSnapshot,
}

/// Runtime errors that can occur during workflow execution.
#[derive(Debug)]
pub enum RuntimeError {
    /// A node could not be spawned for execution.
    SpawnError(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
    use crate::graph::NodeStatus;
    use crate::model::WorkflowDef;

    /// Cross-platform echo command for tests.
    /// On Windows, echo is a cmd.exe built-in, not a standalone executable.
    /// On Unix, echo is a standalone binary at /bin/echo.
    fn echo_cmd(text: &str) -> String {
        if cfg!(windows) {
            format!("cmd.exe /c echo {}", text)
        } else {
            format!("echo {}", text)
        }
    }

    /// A simple inline workflow definition for testing: A → B (chain).
    fn chain_workflow() -> WorkflowDef {
        use crate::model::predecessor::{EventType, SchedulingEdgeDef, TriggerExpr};
        use crate::model::provider::ProviderDef;

        WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: "echo".into(),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: "echo".into(),
                    }],
                    process_timeout_secs: 10,
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
        let engine = Engine::new(def, config, None);
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
        let engine = Engine::new(def, config, None);
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
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "X".into(),
                    providers: vec![],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
            ],
            edges: vec![],
            dataflows: vec![],
        };
        let config = EngineConfig::default();
        let engine = Engine::new(def, config, None);
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
        let mut engine = Engine::new(def, config, None).expect("empty workflow");
        let result = engine.run().await;
        assert!(result.is_ok(), "empty workflow should converge immediately");
    }

    #[tokio::test]
    async fn test_engine_config_defaults_used() {
        let config = EngineConfig::new(Some(2), 7200, 5);
        assert_eq!(config.effective_max_concurrency(), 2);
        assert_eq!(config.node_timeout(), Duration::from_secs(7200));
    }

    #[tokio::test]
    async fn test_single_node_completes() {
        // Single node A (echo hello), no edges — should converge after A completes.
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![crate::model::workflow::NodeDef {
                id: "A".into(),
                providers: vec![ProviderDef::Subprocess {
                    command: echo_cmd("hello"),
                }],
                process_timeout_secs: 10,
                returns: vec![],
                max_retries: None,
            }],
            edges: vec![],
            dataflows: vec![],
        };
        let config = EngineConfig::new(None, 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_single_node_completes timed out after 30s"),
        };
        assert!(result.is_ok(), "single node should complete: {:?}", result);

        let a_idx = engine.scheduler().graph().node_index("A").expect("A exists");
        let state = engine.scheduler().state();
        assert_eq!(
            state.states[&a_idx].status,
            NodeStatus::Completed,
            "A should be Completed"
        );
        assert!(
            engine.scheduler().is_converged(),
            "graph should converge after single node completes"
        );
    }

    #[tokio::test]
    async fn test_chain_execution_a_to_b() {
        // A(echo chain_a) → B(echo chain_b), verify chain execution and convergence.
        use crate::model::predecessor::{EventType, SchedulingEdgeDef, TriggerExpr};
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("chain_a"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("chain_b"),
                    }],
                    process_timeout_secs: 10,
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
        };
        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_chain_execution_a_to_b timed out after 30s"),
        };
        assert!(
            result.is_ok(),
            "chain should complete: {:?}",
            result
        );

        let a_idx = engine.scheduler().graph().node_index("A").expect("A exists");
        let b_idx = engine.scheduler().graph().node_index("B").expect("B exists");
        let state = engine.scheduler().state();
        assert_eq!(
            state.states[&a_idx].status,
            NodeStatus::Completed,
            "A should be Completed"
        );
        assert_eq!(
            state.states[&b_idx].status,
            NodeStatus::Completed,
            "B should be Completed"
        );
        assert!(
            engine.scheduler().is_converged(),
            "chain graph should converge"
        );
    }

    #[tokio::test]
    async fn test_exit_reason_filter_triggers_downstream() {
        // Minimal test: A (exit_reason=ok via line protocol) → B (exit_reason="ok")
        // A produces __nexus_exit_reason: ok → B should trigger and complete.
        use crate::model::predecessor::{EventType, SchedulingEdgeDef, TriggerExpr};
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("__nexus_exit_reason: ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("b_done"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
            ],
            edges: vec![SchedulingEdgeDef {
                from: "A".into(),
                to: "B".into(),
                trigger: TriggerExpr::All,
                event: EventType::Complete,
                exit_reason: Some("ok".into()),
                threshold: 1,
            }],
            dataflows: vec![],
        };

        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_exit_reason_filter_triggers_downstream timed out after 30s"),
        };
        assert!(result.is_ok(), "workflow should converge: {:?}", result);

        let a_idx = engine.scheduler().graph().node_index("A").expect("A exists");
        let b_idx = engine.scheduler().graph().node_index("B").expect("B exists");
        let state = engine.scheduler().state();
        assert_eq!(
            state.states[&a_idx].status,
            NodeStatus::Completed,
            "A should be Completed"
        );
        assert_eq!(
            state.states[&b_idx].status,
            NodeStatus::Completed,
            "B should be triggered by exit_reason 'ok'"
        );
        assert!(
            engine.scheduler().is_converged(),
            "graph should converge"
        );
    }

    #[tokio::test]
    async fn test_exit_reason_routes_ok_to_b() {
        // A → B (exit_reason="ok"). A produces exit_reason "ok" → B should trigger.
        use crate::model::predecessor::{EventType, SchedulingEdgeDef, TriggerExpr};
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("__nexus_exit_reason: ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("b_done"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
            ],
            edges: vec![SchedulingEdgeDef {
                from: "A".into(),
                to: "B".into(),
                trigger: TriggerExpr::All,
                event: EventType::Complete,
                exit_reason: Some("ok".into()),
                threshold: 1,
            }],
            dataflows: vec![],
        };

        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_exit_reason_routes_ok_to_b timed out after 30s"),
        };
        assert!(result.is_ok(), "workflow should converge: {:?}", result);

        let a_idx = engine.scheduler().graph().node_index("A").expect("A exists");
        let b_idx = engine.scheduler().graph().node_index("B").expect("B exists");
        let state = engine.scheduler().state();
        assert_eq!(state.states[&a_idx].status, NodeStatus::Completed, "A should be Completed");
        assert_eq!(
            state.states[&b_idx].status,
            NodeStatus::Completed,
            "B should be triggered (exit_reason 'ok' matches)"
        );
        assert!(engine.scheduler().is_converged(), "graph should converge");
    }

    #[tokio::test]
    async fn test_exit_reason_routes_review_to_c() {
        // A → C (exit_reason="review"). A produces exit_reason "review" → C should trigger.
        use crate::model::predecessor::{EventType, SchedulingEdgeDef, TriggerExpr};
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("__nexus_exit_reason: review"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("c_done"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
            ],
            edges: vec![SchedulingEdgeDef {
                from: "A".into(),
                to: "C".into(),
                trigger: TriggerExpr::All,
                event: EventType::Complete,
                exit_reason: Some("review".into()),
                threshold: 1,
            }],
            dataflows: vec![],
        };

        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_exit_reason_routes_review_to_c timed out after 30s"),
        };
        assert!(result.is_ok(), "workflow should converge: {:?}", result);

        let a_idx = engine.scheduler().graph().node_index("A").expect("A exists");
        let c_idx = engine.scheduler().graph().node_index("C").expect("C exists");
        let state = engine.scheduler().state();
        assert_eq!(state.states[&a_idx].status, NodeStatus::Completed, "A should be Completed");
        assert_eq!(
            state.states[&c_idx].status,
            NodeStatus::Completed,
            "C should be triggered (exit_reason 'review' matches)"
        );
        assert!(engine.scheduler().is_converged(), "graph should converge");
    }

    #[tokio::test]
    async fn test_exit_reason_branch_routing_in_engine() {
        // Full branch: A → B (exit_reason="ok"), A → C (exit_reason="review").
        // A produces exit_reason "ok" → B runs. C stays Pending since its exit_reason
        // doesn't match — graph does NOT converge. We query partial state after a short
        // timeout to verify B completed and C remained Pending.
        use crate::model::predecessor::{EventType, SchedulingEdgeDef, TriggerExpr};
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("__nexus_exit_reason: ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("b_route"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("c_route"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
            ],
            edges: vec![
                SchedulingEdgeDef {
                    from: "A".into(),
                    to: "B".into(),
                    trigger: TriggerExpr::Any,
                    event: EventType::Complete,
                    exit_reason: Some("ok".into()),
                    threshold: 1,
                },
                SchedulingEdgeDef {
                    from: "A".into(),
                    to: "C".into(),
                    trigger: TriggerExpr::Any,
                    event: EventType::Complete,
                    exit_reason: Some("review".into()),
                    threshold: 1,
                },
            ],
            dataflows: vec![],
        };

        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");

        // Let A→B run (should take < 1s). C stays Pending → graph won't converge.
        // The 5s timeout elapses, proving the engine is stuck waiting.
        let result = tokio::time::timeout(Duration::from_secs(5), engine.run()).await;
        assert!(
            result.is_err(),
            "graph should NOT converge when C stays Pending (exit_reason 'ok' != 'review')"
        );

        // Check partial state: B completed, C stayed Pending.
        let state = engine.scheduler().state();
        let b_idx = engine.scheduler().graph().node_index("B").expect("B exists");
        let c_idx = engine.scheduler().graph().node_index("C").expect("C exists");

        assert_eq!(
            state.states[&b_idx].status,
            NodeStatus::Completed,
            "B should be triggered (exit_reason 'ok' matches)"
        );
        assert_eq!(
            state.states[&c_idx].status,
            NodeStatus::Pending,
            "C should NOT be triggered (exit_reason 'ok' doesn't match 'review')"
        );
    }

    #[tokio::test]
    async fn test_fan_in_all_one_fails_no_trigger() {
        // A → C (All, Complete), B → C (All, Complete)
        // A fails (exit 1, Failed event) → A's edge expects Complete, so it
        // does NOT fire. B completes (Complete event) → B's edge fires,
        // decrementing fan_in_pending[C] from 2 to 1. Since pending > 0,
        // C is NOT enqueued. C stays Pending forever → engine does not converge.
        //
        // The engine event loop hangs (no new events → waiting on event_rx).
        // The test timeout catches this: engine.run() should NOT complete
        // within the timeout.
        use crate::model::predecessor::{EventType, SchedulingEdgeDef, TriggerExpr};
        use crate::model::provider::ProviderDef;

        let a_command = if cfg!(windows) {
            "cmd.exe /c exit 1".to_string()
        } else {
            "sh -c \"exit 1\"".to_string()
        };
        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: a_command,
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("done"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
            ],
            edges: vec![
                SchedulingEdgeDef {
                    from: "A".into(),
                    to: "C".into(),
                    trigger: TriggerExpr::All,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 1,
                },
                SchedulingEdgeDef {
                    from: "B".into(),
                    to: "C".into(),
                    trigger: TriggerExpr::All,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 1,
                },
            ],
            dataflows: vec![],
        };
        let config = EngineConfig::new(Some(2), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");

        // With correct All semantics, engine.run() should NOT converge
        // (C remains Pending because A's edge never fires). The 5s timeout
        // should elapse, proving the engine is stuck waiting.
        let result = tokio::time::timeout(Duration::from_secs(5), engine.run()).await;

        assert!(
            result.is_err(),
            "engine should NOT converge when A fails and C requires both A and B (All semantics)"
        );
    }

    #[tokio::test]
    async fn test_fan_in_all_both_complete_triggers_downstream() {
        // A → C (All, Complete), B → C (All, Complete)
        // Both A and B complete (exit 0) → both edges fire → fan_in_pending[C]
        // drops to 0 → C is enqueued → C executes → graph converges.
        use crate::model::predecessor::{EventType, SchedulingEdgeDef, TriggerExpr};
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("a_ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("b_ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("c_done"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
            ],
            edges: vec![
                SchedulingEdgeDef {
                    from: "A".into(),
                    to: "C".into(),
                    trigger: TriggerExpr::All,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 1,
                },
                SchedulingEdgeDef {
                    from: "B".into(),
                    to: "C".into(),
                    trigger: TriggerExpr::All,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 1,
                },
            ],
            dataflows: vec![],
        };
        let config = EngineConfig::new(Some(2), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_fan_in_all_both_complete_triggers_downstream timed out after 30s"),
        };
        assert!(result.is_ok(), "both complete → workflow converges");

        let state = engine.scheduler().state();
        let a_idx = engine.scheduler().graph().node_index("A").expect("A exists");
        let b_idx = engine.scheduler().graph().node_index("B").expect("B exists");
        let c_idx = engine.scheduler().graph().node_index("C").expect("C exists");

        assert_eq!(state.states[&a_idx].status, NodeStatus::Completed, "A completed");
        assert_eq!(state.states[&b_idx].status, NodeStatus::Completed, "B completed");
        assert_eq!(
            state.states[&c_idx].status,
            NodeStatus::Completed,
            "C should execute when both A and B complete (All semantics)"
        );
        assert!(engine.scheduler().is_converged(), "graph should converge");
    }

    #[tokio::test]
    async fn test_fan_out() {
        // A → B, A → C, verify both B and C execute.
        use crate::model::predecessor::{EventType, SchedulingEdgeDef, TriggerExpr};
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("fan_out_a"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("fan_out_b"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("fan_out_c"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
            ],
            edges: vec![
                SchedulingEdgeDef {
                    from: "A".into(),
                    to: "B".into(),
                    trigger: TriggerExpr::All,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 1,
                },
                SchedulingEdgeDef {
                    from: "A".into(),
                    to: "C".into(),
                    trigger: TriggerExpr::All,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 1,
                },
            ],
            dataflows: vec![],
        };
        let config = EngineConfig::new(Some(2), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_fan_out timed out after 30s"),
        };
        assert!(
            result.is_ok(),
            "fan-out should complete: {:?}",
            result
        );

        let state = engine.scheduler().state();
        let a_idx = engine.scheduler().graph().node_index("A").expect("A");
        let b_idx = engine.scheduler().graph().node_index("B").expect("B");
        let c_idx = engine.scheduler().graph().node_index("C").expect("C");
        assert_eq!(state.states[&a_idx].status, NodeStatus::Completed);
        assert_eq!(state.states[&b_idx].status, NodeStatus::Completed);
        assert_eq!(state.states[&c_idx].status, NodeStatus::Completed);
        assert!(
            engine.scheduler().is_converged(),
            "fan-out graph should converge"
        );
    }

    #[tokio::test]
    async fn test_dataflow_skip_level() {
        // Scheduling: A → B → C
        // Dataflow:  A → C (skip-level: data passes over B directly to C)
        // Verify: C's inputs contain A's output with the correct value.
        use crate::model::predecessor::{DataFlowDef, EventType, SchedulingEdgeDef, TriggerExpr};
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("data_from_a"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("data_from_b"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("c_received"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
            ],
            edges: vec![
                SchedulingEdgeDef {
                    from: "A".into(),
                    to: "B".into(),
                    trigger: TriggerExpr::All,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 1,
                },
                SchedulingEdgeDef {
                    from: "B".into(),
                    to: "C".into(),
                    trigger: TriggerExpr::All,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 1,
                },
            ],
            dataflows: vec![DataFlowDef {
                from: "A".into(),
                to: "C".into(),
                alias: None,
            }],
        };

        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_dataflow_skip_level timed out after 30s"),
        };
        assert!(result.is_ok(), "skip-level dataflow should converge");

        // All three nodes must have completed.
        let a_idx = engine.scheduler().graph().node_index("A").unwrap();
        let b_idx = engine.scheduler().graph().node_index("B").unwrap();
        let c_idx = engine.scheduler().graph().node_index("C").unwrap();
        let state = engine.scheduler().state();
        assert_eq!(state.states[&a_idx].status, NodeStatus::Completed);
        assert_eq!(state.states[&b_idx].status, NodeStatus::Completed);
        assert_eq!(state.states[&c_idx].status, NodeStatus::Completed);

        // Verify C's inputs contain A's output (skip-level dataflow).
        let inputs = engine.data_router().build_input(c_idx);
        assert!(inputs.contains_key("A"), "C's inputs should contain key 'A'");
        assert!(
            inputs.get("A").map(|s| s.trim()).unwrap_or("").contains("data_from_a"),
            "C should receive A's output through skip-level dataflow, got: {:?}",
            inputs.get("A")
        );
    }

    #[tokio::test]
    async fn test_dataflow_reverse_direction() {
        // Scheduling: A → B (A executes first, then B)
        // Dataflow:  B → A (data flows opposite to scheduling direction)
        // This verifies dataflows can declare any direction independent of
        // scheduling edges without causing errors or panics.  A executes first
        // and at that point B has no output yet (empty string).  After B runs,
        // the DataRouter stores B's output so it is visible post-run.  The key
        // insight: the engine does not crash, and inputs are routed correctly.
        use crate::model::predecessor::{DataFlowDef, EventType, SchedulingEdgeDef, TriggerExpr};
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("output_from_a"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("output_from_b"),
                    }],
                    process_timeout_secs: 10,
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
            dataflows: vec![DataFlowDef {
                from: "B".into(),
                to: "A".into(),
                alias: None,
            }],
        };

        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_dataflow_reverse_direction timed out after 30s"),
        };
        assert!(result.is_ok(), "reverse-direction dataflow should not cause errors");

        let a_idx = engine.scheduler().graph().node_index("A").unwrap();
        let b_idx = engine.scheduler().graph().node_index("B").unwrap();
        let state = engine.scheduler().state();
        assert_eq!(state.states[&a_idx].status, NodeStatus::Completed);
        assert_eq!(state.states[&b_idx].status, NodeStatus::Completed);

        // The DataRouter maps B→A, so A's input slot for B exists.
        // After engine.run(), B's output is stored; we verify the router
        // correctly maps the reversed direction without errors.
        let a_inputs = engine.data_router().build_input(a_idx);
        assert!(
            a_inputs.contains_key("B"),
            "A's inputs should contain key 'B' from the dataflow declaration"
        );
        // At runtime A executed first (B was empty), but post-run B's output is stored.
        // The important thing is the dataflow declaration is accepted and routed.
        assert_eq!(
            a_inputs.get("B").map(|s| s.trim()),
            Some("output_from_b"),
            "Post-run, B's output should be visible in the data router"
        );
    }

    #[tokio::test]
    async fn test_dataflow_without_scheduling_edge() {
        // No scheduling edges: A and B are both entry nodes, run in parallel.
        // Dataflow: A → B (data dependency without scheduling dependency)
        // B runs concurrently with A, so A's output may or may not be ready.
        // Verify the engine does not crash and B's inputs contain the 'A' key.
        use crate::model::predecessor::DataFlowDef;
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("data_from_a"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("data_from_b"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
            ],
            edges: vec![],
            dataflows: vec![DataFlowDef {
                from: "A".into(),
                to: "B".into(),
                alias: None,
            }],
        };

        let config = EngineConfig::new(Some(2), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_dataflow_without_scheduling_edge timed out after 30s"),
        };
        assert!(result.is_ok(), "dataflow without scheduling edge should converge");

        let a_idx = engine.scheduler().graph().node_index("A").unwrap();
        let b_idx = engine.scheduler().graph().node_index("B").unwrap();
        let state = engine.scheduler().state();
        assert_eq!(state.states[&a_idx].status, NodeStatus::Completed);
        assert_eq!(state.states[&b_idx].status, NodeStatus::Completed);

        // B's inputs should contain key 'A' (the dataflow is registered even
        // if A's output may have arrived as empty due to concurrent execution).
        let b_inputs = engine.data_router().build_input(b_idx);
        assert!(
            b_inputs.contains_key("A"),
            "B's inputs should contain key 'A' from the dataflow declaration"
        );
    }

    #[tokio::test]
    async fn test_dataflow_multi_source_aggregation() {
        // Scheduling: A, B, C are entry nodes (parallel). D waits on all three.
        // Dataflow: A→D, B→D, C→D (three sources aggregate into D).
        // Verify D's inputs contain all three upstream outputs.
        use crate::model::predecessor::{DataFlowDef, EventType, SchedulingEdgeDef, TriggerExpr};
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("output_a"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("output_b"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("output_c"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
                crate::model::workflow::NodeDef {
                    id: "D".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: echo_cmd("d_received"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                },
            ],
            edges: vec![
                SchedulingEdgeDef {
                    from: "A".into(),
                    to: "D".into(),
                    trigger: TriggerExpr::All,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 1,
                },
                SchedulingEdgeDef {
                    from: "B".into(),
                    to: "D".into(),
                    trigger: TriggerExpr::All,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 1,
                },
                SchedulingEdgeDef {
                    from: "C".into(),
                    to: "D".into(),
                    trigger: TriggerExpr::All,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 1,
                },
            ],
            dataflows: vec![
                DataFlowDef {
                    from: "A".into(),
                    to: "D".into(),
                    alias: None,
                },
                DataFlowDef {
                    from: "B".into(),
                    to: "D".into(),
                    alias: None,
                },
                DataFlowDef {
                    from: "C".into(),
                    to: "D".into(),
                    alias: None,
                },
            ],
        };

        let config = EngineConfig::new(Some(3), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_dataflow_multi_source_aggregation timed out after 30s"),
        };
        assert!(result.is_ok(), "multi-source dataflow should converge");

        let d_idx = engine.scheduler().graph().node_index("D").unwrap();

        // D must contain all three source keys.
        let d_inputs = engine.data_router().build_input(d_idx);
        assert_eq!(
            d_inputs.len(),
            3,
            "D should receive inputs from all three sources (A, B, C)"
        );
        assert!(
            d_inputs.contains_key("A"),
            "D's inputs should contain key 'A'"
        );
        assert!(
            d_inputs.contains_key("B"),
            "D's inputs should contain key 'B'"
        );
        assert!(
            d_inputs.contains_key("C"),
            "D's inputs should contain key 'C'"
        );

        // Verify the actual output content was routed correctly.
        assert!(
            d_inputs.get("A").map(|s| s.trim()).unwrap_or("").contains("output_a"),
            "D should receive A's output"
        );
        assert!(
            d_inputs.get("B").map(|s| s.trim()).unwrap_or("").contains("output_b"),
            "D should receive B's output"
        );
        assert!(
            d_inputs.get("C").map(|s| s.trim()).unwrap_or("").contains("output_c"),
            "D should receive C's output"
        );
    }
}
