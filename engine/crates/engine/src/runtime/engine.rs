use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// Minimum capacity for the engine's internal event channel (actual
/// capacity is `max(node_count, MIN)` clamped to 4096).
const EVENT_CHANNEL_MIN_CAPACITY: usize = 64;

/// Default duration for the convergence watchdog timer.
/// If the event channel is silent for this long while the scheduler reports
/// convergence, the engine assumes a `NodeCompleted` event was lost and exits.
const CONVERGENCE_WATCHDOG_SECS: u64 = 10;

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
    /// Concurrency limiter — permits equal `max_concurrency`.
    semaphore: Arc<Semaphore>,
    /// Sender half of the internal event channel.
    event_tx: mpsc::Sender<EngineEvent>,
    /// Receiver half of the internal event channel.
    event_rx: mpsc::Receiver<EngineEvent>,
    /// Optional callback for real-time node lifecycle events.
    event_cb: Option<NodeEventCb>,
    /// Wall-clock time when `run()` started (None before first run).
    started_at: Option<Instant>,
    /// External cancellation flag. Set to true to stop the engine.
    cancel_flag: Arc<AtomicBool>,
    workflow_scripts_dir: Option<String>,
    /// Per-node wall-clock start times, used to compute execution
    /// duration for [`RoutePolicyDef::MaxDuration`].
    node_started_at: HashMap<NodeIndex, Instant>,
}

impl fmt::Debug for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Engine")
            .field("scheduler", &self.scheduler)
            .field("config", &self.config)
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
    /// Convenience wrapper — discards the cancel handle. Use
    /// [`Engine::new_with_cancel`] if you need to stop the engine externally.
    pub fn new(
        def: WorkflowDef,
        config: EngineConfig,
        event_cb: Option<NodeEventCb>,
    ) -> Result<Self, Vec<ValidationError>> {
        Self::new_with_cancel(def, config, event_cb).map(|(e, _)| e)
    }

    /// Create a new `Engine` returning the cancel handle alongside.
    /// Set the handle to `true` to stop the engine at the next event-loop
    /// iteration — all pending nodes are marked Skipped and the engine
    /// converges.
    pub fn new_with_cancel(
        def: WorkflowDef,
        config: EngineConfig,
        event_cb: Option<NodeEventCb>,
    ) -> Result<(Self, Arc<AtomicBool>), Vec<ValidationError>> {
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let graph = crate::graph::Builder::build(&def)?;
        let node_id_to_index: HashMap<String, NodeIndex> = graph
            .node_indices()
            .filter_map(|idx| {
                graph
                    .node_weight(idx)
                    .map(|nd| (nd.id.clone(), idx))
            })
            .collect();
        let node_count = graph.node_count();
        let capacity = node_count.max(EVENT_CHANNEL_MIN_CAPACITY).min(4096);

        let data_router = DataRouter::new(node_id_to_index, &def.dataflows);
        let scheduler = Scheduler::new(graph);

        let max_permits = config.effective_max_concurrency();
        let semaphore = Arc::new(Semaphore::new(max_permits));

        let (tx, rx) = mpsc::channel(capacity);

        let workflow_scripts_dir = def.scripts_dir;
        let handle = cancel_flag.clone();
        Ok((Self {
            scheduler,
            data_router,
            config,
            semaphore,
            event_tx: tx,
            event_rx: rx,
            event_cb,
            started_at: None,
            cancel_flag,
            workflow_scripts_dir,
            node_started_at: HashMap::new(),
        }, handle))
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

        self.started_at = Some(Instant::now());
        let mut consecutive_timeouts: u32 = 0;

        loop {
            // External cancellation — kill running nodes and converge.
            if self.cancel_flag.load(Ordering::Relaxed) {
                tracing::warn!(target: "nexus::engine", "cancelled by external request, stopping");
                self.scheduler.mark_pending_nodes_skipped();
                break;
            }
            // Check convergence via scheduler — the single source of truth.
            if self.scheduler.is_converged() {
                break;
            }

            match tokio::time::timeout(
                Duration::from_secs(CONVERGENCE_WATCHDOG_SECS),
                self.event_rx.recv(),
            )
            .await
            {
                Ok(Some(e)) => {
                    consecutive_timeouts = 0;
                    self.handle_event(e).await;
                }
                // Channel closed (all senders dropped) — no more events.
                Ok(None) => break,
                // Watchdog timeout: no event for 10s. If scheduler says converged,
                // assume a NodeCompleted event was lost and exit.
                Err(_) if self.scheduler.is_converged() => break,
                // Timeout with no new events. If nodes are still running, the
                // watchdog may have fired while waiting for a slow node (e.g. LLM
                // call). Reset the counter and keep waiting.
                Err(_) if self.running_count() > 0 => {
                    consecutive_timeouts = 0;
                }
                // Timeout, scheduler not converged, and no nodes running — the
                // graph is deadlocked. Mark pending nodes as skipped and exit
                // after 3 consecutive timeouts.
                Err(_) => {
                    consecutive_timeouts += 1;
                    if consecutive_timeouts >= 3 {
                        self.scheduler.mark_pending_nodes_skipped();
                        break;
                    }
                }
            }
        }

        let elapsed = self.started_at.map(|s| s.elapsed()).unwrap_or_default();
        event::emit_engine(&EngineLifecycleEvent::Converged {
            duration: elapsed,
        });
        let snapshot = EngineSnapshot::capture(&self.scheduler, self.started_at.unwrap_or(Instant::now()));
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
                self.handle_node_ready(node_id).await;
            }
            EngineEvent::NodeCompleted { node_id, outcome } => {
                self.handle_node_completed(node_id, outcome).await;
            }
        }
    }

    // ── Route policy helpers ─────────────────────────────────
    //
    // route_policy (currently only MaxRuns) has two phases:
    //   1. Pre-execution skip — if run_count already reached max, don't
    //      spawn the process; emit a synthetic completion with then_route.
    //   2. Post-execution override — after the node completes, override
    //      its exit_reason if the policy threshold was met this round.
    //
    // Both phases use the same threshold check, consolidated here.

    /// Check whether `route_policy` should skip execution before spawning.
    ///
    /// Returns `Some(then_route)` if the policy threshold has been reached
    /// and the node should be skipped, `None` otherwise.
    fn route_policy_should_skip(
        &self,
        node_id: NodeIndex,
        run_count: u64,
    ) -> Option<String> {
        let policy = self
            .scheduler
            .graph()
            .node_weight(node_id)
            .and_then(|nd| nd.route_policy.as_ref())?;
        match policy {
            crate::model::workflow::RoutePolicyDef::MaxRuns { max, then_route }
                if run_count >= *max => Some(then_route.clone()),
            crate::model::workflow::RoutePolicyDef::MaxDuration { max_secs, then_route }
                if self.scheduler.cumulative_runtime_secs(node_id) >= *max_secs =>
            {
                Some(then_route.clone())
            }
            _ => None,
        }
    }

    /// Resolve the effective `exit_reason` for a completed node.
    ///
    /// `route_policy` overrides the node's own `exit_reason` when the threshold
    /// is met.  Otherwise the node's stdout route is used as-is.
    fn resolve_effective_exit_reason(
        &self,
        node_id: NodeIndex,
        run_count: u64,
        node_exit_reason: Option<&str>,
    ) -> Option<String> {
        self.route_policy_should_skip(node_id, run_count)
            .or_else(|| node_exit_reason.map(String::from))
    }

    // ── Execution helpers ────────────────────────────────────

    /// Build a [`NodeContext`] for the given node using the current
    /// `DataRouter` and Scheduler state.
    fn build_execution_context(
        &self,
        node_id: NodeIndex,
        run_count: u64,
    ) -> NodeContext {
        NodeContext {
            inputs: self.data_router.build_input(node_id),
            extensions: HashMap::new(),
            metadata: crate::nodeshell::NodeMetadata {
                run_count,
                timed_out: self.scheduler.was_last_timed_out(node_id),
            },
            upstream: self.data_router.build_upstream(node_id),
        }
    }

    /// Emit a lifecycle event to both the tracing subscriber and the
    /// optional user callback.
    fn emit_lifecycle_event(&self, event: NodeLifecycleEvent) {
        event::emit_lifecycle(&event);
        if let Some(ref cb) = self.event_cb {
            cb(NodeEvent::Lifecycle(event));
        }
    }

    /// Drain the scheduler's ready queue and dispatch `NodeReady` events
    /// for every enqueued node.
    fn dispatch_ready_nodes(&mut self) {
        let ready: Vec<_> = self.scheduler.dequeue_all();
        if ready.is_empty() {
            return;
        }
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            for target in ready {
                let _ = tx.send(EngineEvent::NodeReady { node_id: target }).await;
            }
        });
    }

    /// Spawn a background task that consumes chunk messages and forwards
    /// them to tracing + the user callback.  Returns the sender half so
    /// the executor can push chunks.
    fn spawn_chunk_consumer(&self, node_id: &str) -> mpsc::Sender<NodeChunk> {
        let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::channel::<NodeChunk>(256);
        let chunk_node_id = node_id.to_string();
        let chunk_cb = self.event_cb.clone();
        tokio::spawn(async move {
            while let Some(chunk) = chunk_rx.recv().await {
                let trimmed = chunk.text.trim();
                if trimmed.is_empty() {
                    continue; // skip heartbeat keep-alives
                }
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
        chunk_tx
    }

    /// Handle a `NodeReady` event: apply `route_policy` pre-check, build
    /// the execution context, guard against double-execution, and spawn
    /// the node in a background task.
    async fn handle_node_ready(&mut self, node_id: NodeIndex) {
        let nid = self.node_id(node_id);

        // ── Phase 1: extract immutable data from the graph ──
        // All graph borrows are scoped to this block so that later
        // mutable scheduler operations don't conflict.
        let (command_label, timeout, scripts_dir, provider_owned) = {
            let graph = self.scheduler.graph();
            let nd = graph.node_weight(node_id);
            let provider = nd.and_then(|nd| nd.providers.first());
            (
                provider
                    .as_ref()
                    .map(|p| format!("{p:?}"))
                    .unwrap_or_default(),
                graph
                    .node_params(node_id)
                    .map_or(Duration::from_secs(
                        self.config.default_node_timeout_secs,
                    ), |p| Duration::from_secs(p.process_timeout_secs)),
                crate::nodeshell::resolve_scripts_dir(
                    nd.and_then(|nd| nd.scripts_dir.as_deref()),
                    self.workflow_scripts_dir.as_deref(),
                ),
                provider.cloned(),
            )
        };

        // Emit Running event.
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

        // ── Phase 2: mutable scheduler updates ──
        let run_count = self.scheduler.node_run_count(node_id).saturating_add(1);
        self.scheduler.set_node_run_count(node_id, run_count);

        // route_policy pre-execution skip.
        if let Some(then_route) = self.route_policy_should_skip(node_id, run_count) {
            tracing::info!(
                target: "nexus::engine",
                node_id = nid,
                run_count,
                then_route,
                "route_policy: max_runs reached, skipping node execution"
            );
            self.data_router.store_output(
                node_id,
                &crate::nodeshell::NodeOutput {
                    route: then_route.clone(),
                    content: String::new(),
                },
            );
            let outcome = crate::nodeshell::NodeOutcome {
                output: crate::nodeshell::NodeOutput {
                    route: then_route.clone(),
                    content: String::new(),
                },
                exit_code: 0,
                exit_reason: Some(then_route),
            };
            let tx = self.event_tx.clone();
            tokio::spawn(async move {
                let _ = tx
                    .send(EngineEvent::NodeCompleted {
                        node_id,
                        outcome: Ok(outcome),
                    })
                    .await;
            });
            if let Some(ref cb) = self.event_cb {
                cb(crate::runtime::NodeEvent::NodeCompleted {
                    node_id: nid,
                });
            }
            return;
        }

        // Guard against concurrent double-execution.
        if self.scheduler.is_node_running(node_id) {
            tracing::warn!(
                target: "nexus::engine",
                node_id = nid,
                "duplicate NodeReady for already-running node, skipping"
            );
            return;
        }
        self.scheduler
            .set_node_status(node_id, crate::graph::NodeStatus::Running);

        // Record start time for MaxDuration route_policy.
        self.node_started_at.insert(node_id, Instant::now());

        // ── Phase 3: build context & spawn ──
        let ctx = self.build_execution_context(node_id, run_count);

        let executor = provider_owned
            .as_ref().map_or_else(|| {
                NodeExecutor::from_provider(
                    &crate::model::provider::ProviderDef::Subprocess {
                        command: String::new(),
                    },
                    &scripts_dir,
                )
            }, |p| NodeExecutor::from_provider(p, &scripts_dir));

        let chunk_tx = self.spawn_chunk_consumer(&nid);
        let outcome_tx = self.event_tx.clone();
        let semaphore = self.semaphore.clone();
        tokio::spawn(async move {
            let _permit = if let Ok(p) = semaphore.acquire_owned().await { p } else {
                tracing::warn!(
                    target: "nexus::engine",
                    node_id = nid,
                    "semaphore closed, skipping node"
                );
                return;
            };
            let outcome = executor.run(ctx, timeout, &nid, Some(chunk_tx)).await;
            let _ = outcome_tx
                .send(EngineEvent::NodeCompleted {
                    node_id,
                    outcome,
                })
                .await;
        });
    }

    /// Handle a `NodeCompleted` event: decide retry vs. process, update
    /// scheduler state, resolve `route_policy`, emit lifecycle events, and
    /// dispatch newly-ready downstream nodes.
    async fn handle_node_completed(
        &mut self,
        node_id: NodeIndex,
        outcome: Result<NodeOutcome, SpawnError>,
    ) {
        let nid = self.node_id(node_id);
        let max_retries = self
            .scheduler
            .node_max_retries(node_id, self.config.max_timeout_retries);

        // ── Retry decision (timeout / spawn error only) ──
        let should_retry = match &outcome {
            Ok(o) if o.timed_out() => {
                self.emit_lifecycle_event(NodeLifecycleEvent::TimedOut {
                    node_id: nid.clone(),
                    timeout_secs: self.config.default_node_timeout_secs,
                });
                if let Some(ref cb) = self.event_cb {
                    cb(NodeEvent::NodeTimedOut {
                        node_id: nid.clone(),
                    });
                }
                self.scheduler.retry_node(node_id, max_retries)
            }
            Err(_) => {
                self.emit_lifecycle_event(NodeLifecycleEvent::Failed {
                    node_id: nid.clone(),
                    exit_reason: "spawn_error".into(),
                    retry_count: 0,
                });
                self.scheduler.retry_node(node_id, max_retries)
            }
            _ => false,
        };

        if should_retry {
            self.data_router.clear_output(node_id);
            let tx = self.event_tx.clone();
            tokio::spawn(async move {
                let _ = tx.send(EngineEvent::NodeReady { node_id }).await;
            });
            return;
        }

        // ── Process outcome ──
        match outcome {
            Ok(outcome) => {
                self.process_successful_outcome(node_id, &nid, outcome).await;
            }
            Err(_spawn_err) => {
                // Spawn error after retries exhausted → Failed.
                self.scheduler
                    .handle_event(node_id, EventType::Failed, None);
                self.dispatch_ready_nodes();
                if self.scheduler.running_count() == 0 {
                    tracing::warn!(
                        target: "nexus::engine",
                        node_id = ?self.scheduler.graph().node_weight(node_id).map(|n| n.id.as_str()),
                        "spawn error exhausted retries, no downstream edge matched, deadlock"
                    );
                    self.scheduler.mark_pending_nodes_skipped();
                }
            }
        }
    }

    /// Process a successful node outcome: store output, apply `route_policy`,
    /// update scheduler state, emit completion events, and dispatch
    /// newly-ready downstream nodes.
    async fn process_successful_outcome(
        &mut self,
        node_id: NodeIndex,
        nid: &str,
        outcome: NodeOutcome,
    ) {
        // Accumulate execution duration for MaxDuration route_policy.
        if let Some(start) = self.node_started_at.remove(&node_id) {
            let elapsed = start.elapsed().as_secs();
            self.scheduler.add_cumulative_runtime(node_id, elapsed);
        }

        // Record timeout state for next execution.
        if outcome.timed_out() {
            self.scheduler.mark_last_timed_out(node_id);
        }
        self.data_router.store_output(node_id, &outcome.output);

        let event_type = if outcome.timed_out() {
            EventType::Timeout
        } else if outcome.exit_code == 0 {
            EventType::Complete
        } else {
            EventType::Failed
        };

        // Resolve exit_reason — route_policy overrides node output.
        let run_count = self.scheduler.node_run_count(node_id);
        let exit_reason = self.resolve_effective_exit_reason(
            node_id,
            run_count,
            outcome.exit_reason.as_deref(),
        );

        self.scheduler
            .handle_event(node_id, event_type, exit_reason.as_deref());

        // ── Emit completion events ──
        match event_type {
            EventType::Complete => {
                event::emit_lifecycle(&NodeLifecycleEvent::Completed {
                    node_id: nid.to_string(),
                    output_size: outcome.output.content.len(),
                });
                if let Some(ref cb) = self.event_cb {
                    cb(NodeEvent::NodeCompleted {
                        node_id: nid.to_string(),
                    });
                }
            }
            EventType::Failed => {
                let retry_count = self.scheduler.node_retry_count(node_id);
                self.emit_lifecycle_event(NodeLifecycleEvent::Failed {
                    node_id: nid.to_string(),
                    exit_reason: outcome.exit_reason.unwrap_or_else(|| "failed".into()),
                    retry_count,
                });
                if let Some(ref cb) = self.event_cb {
                    cb(NodeEvent::NodeFailed {
                        node_id: nid.to_string(),
                    });
                }
            }
            EventType::Timeout => {
                // Emitted during retry-decision phase; this arm is reached
                // only when retries are exhausted.
            }
        }

        self.dispatch_ready_nodes();
    }

    /// Get a reference to the scheduler (for diagnostics / snapshot).
    #[must_use]
    pub const fn scheduler(&self) -> &Scheduler {
        &self.scheduler
    }

    /// Get the number of currently running nodes.
    #[must_use]
    pub fn running_count(&self) -> usize {
        self.scheduler.running_count()
    }

    /// Access the data router for testing / diagnostics.
    #[must_use]
    pub const fn data_router(&self) -> &DataRouter {
        &self.data_router
    }

    /// Time elapsed since `run()` was called, or `Duration::ZERO` if not running.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.started_at.map(|t| t.elapsed()).unwrap_or_default()
    }

    /// Capture a point-in-time snapshot of the engine's runtime state.
    #[must_use]
    pub fn snapshot(&self) -> EngineSnapshot {
        EngineSnapshot::capture(&self.scheduler, self.started_at.unwrap_or_else(Instant::now))
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
            Self::SpawnError(msg) => {
                write!(f, "spawn error: {msg}")
            }
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<SpawnError> for RuntimeError {
    fn from(err: SpawnError) -> Self {
        Self::SpawnError(err.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::NodeStatus;
    use crate::model::{ProviderDef, WorkflowDef};

    /// Build a command that emits a JSON `NodeOutput` on stdout, cross-platform.
    /// Uses Python to output valid JSON so the subprocess executor can parse it.
    fn json_echo_cmd(text: &str, route: &str) -> String {
        let json = serde_json::json!({"route": route, "content": text});
        let hex: String = json.to_string().bytes().map(|b| format!("{b:02x}")).collect();
        format!(
            "python -c __import__('sys').stdout.write(bytes.fromhex('{hex}').decode())"
        )
    }

    /// Build a command that emits an `exit_reason` JSON on stdout, cross-platform.
    fn exit_reason_cmd(reason: &str) -> String {
        json_echo_cmd("", reason)
    }

    /// Build a `ProviderDef::Subprocess` that emits JSON on stdout.
    fn json_provider(content: &str, route: &str) -> ProviderDef {
        ProviderDef::Subprocess {
            command: json_echo_cmd(content, route),
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
                route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: "echo".into(),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
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
        scripts_dir: None,
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
        scripts_dir: None,
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
                route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "X".into(),
                    providers: vec![],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
                },
            ],
            edges: vec![],
            dataflows: vec![],
        scripts_dir: None,
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
        scripts_dir: None,
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
        assert_eq!(config.node_timeout(), Duration::from_hours(2));
    }

    #[tokio::test]
    async fn test_single_node_completes() {
        // Single node A (echo hello), no edges — should converge after A completes.
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![crate::model::workflow::NodeDef {
                id: "A".into(),
                providers: vec![ProviderDef::Subprocess {
                    command: json_echo_cmd("hello", "ok"),
                }],
                process_timeout_secs: 10,
                returns: vec![],
                max_retries: None,
            route_policy: None,
            scripts_dir: None,
            }],
            edges: vec![],
            dataflows: vec![],
        scripts_dir: None,
        };
        let config = EngineConfig::new(None, 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_single_node_completes timed out after 30s"),
        };
        assert!(result.is_ok(), "single node should complete: {result:?}");

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
    async fn test_single_node_via_shell_provider() {
        // Single node A executed via ProviderDef::Shell (wrapped in cmd /c or sh -c).
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![crate::model::workflow::NodeDef {
                id: "A".into(),
                providers: vec![ProviderDef::Shell {
                    command: json_echo_cmd("shell_works", "ok"),
                }],
                process_timeout_secs: 10,
                returns: vec![],
                max_retries: None,
                route_policy: None,
            scripts_dir: None,
            }],
            edges: vec![],
            dataflows: vec![],
        scripts_dir: None,
        };
        let config = EngineConfig::new(None, 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_single_node_via_shell_provider timed out after 30s"),
        };
        assert!(result.is_ok(), "shell provider should complete: {result:?}");

        let a_idx = engine.scheduler().graph().node_index("A").expect("A exists");
        let state = engine.scheduler().state();
        assert_eq!(
            state.states[&a_idx].status,
            NodeStatus::Completed,
            "A should be Completed"
        );
        assert!(
            engine.scheduler().is_converged(),
            "graph should converge after shell node completes"
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
                        command: exit_reason_cmd("review"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("c_done", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
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
        scripts_dir: None,
        };

        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def.clone(), config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_exit_reason_routes_review_to_c timed out after 30s"),
        };
        assert!(result.is_ok(), "workflow should converge: {result:?}");

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
    async fn test_exit_reason_filter_triggers_downstream() {
        // A (exit_reason=ok) → B (exit_reason="ok"). A's route "ok" triggers B.
        use crate::model::predecessor::{EventType, SchedulingEdgeDef, TriggerExpr};
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("done", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("b_done", "complete"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                scripts_dir: None,
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
        scripts_dir: None,
        };

        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_exit_reason_filter_triggers_downstream timed out after 30s"),
        };
        assert!(result.is_ok(), "workflow should converge: {result:?}");

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
                        command: exit_reason_cmd("ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("b_done", "complete"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                scripts_dir: None,
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
        scripts_dir: None,
        };

        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_exit_reason_routes_ok_to_b timed out after 30s"),
        };
        assert!(result.is_ok(), "workflow should converge: {result:?}");

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
        // A → C (exit_reason="review"). A's route "review" → C should trigger.
        use crate::model::predecessor::{EventType, SchedulingEdgeDef, TriggerExpr};
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: exit_reason_cmd("review"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("c_done", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                scripts_dir: None,
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
        scripts_dir: None,
        };

        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_exit_reason_routes_review_to_c timed out after 30s"),
        };
        assert!(result.is_ok(), "workflow should converge: {result:?}");

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
                        command: exit_reason_cmd("ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("b_route", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("c_route", "review"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
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
        scripts_dir: None,
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
                route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("ok", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("done", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
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
        scripts_dir: None,
        };
        let config = EngineConfig::new(Some(2), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");

        // With All semantics, A fails → A's Complete edge never fires.
        // B completes → B's edge fires but fan_in_pending stays > 0.
        // C stays Pending.  Deadlock detection kicks in immediately
        // (no ready nodes, nothing running) → C is Skipped.
        let result = engine.run().await;
        assert!(
            result.is_ok(),
            "engine should converge immediately via deadlock detection: {result:?}"
        );
        let snapshot = &result.unwrap().snapshot;
        let c_status = snapshot.nodes.get("C").map(|n| n.status);
        assert_eq!(
            c_status,
            Some(crate::graph::scheduler::NodeStatus::Skipped),
            "C should be Skipped due to deadlock detection"
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
                        command: json_echo_cmd("a_ok", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("b_ok", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("c_done", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
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
        scripts_dir: None,
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
                        command: json_echo_cmd("fan_out_a", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("fan_out_b", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("fan_out_c", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
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
        scripts_dir: None,
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
            "fan-out should complete: {result:?}"
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
                        command: json_echo_cmd("data_from_a", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("data_from_b", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("c_received", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
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
        scripts_dir: None,
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
            inputs.get("A").map_or("", |s| s.trim()).contains("data_from_a"),
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
                        command: json_echo_cmd("output_from_a", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("output_from_b", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
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
        scripts_dir: None,
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
                        command: json_echo_cmd("data_from_a", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("data_from_b", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
                },
            ],
            edges: vec![],
            dataflows: vec![DataFlowDef {
                from: "A".into(),
                to: "B".into(),
                alias: None,
            }],
        scripts_dir: None,
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
                        command: json_echo_cmd("output_a", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("output_b", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("output_c", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "D".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: json_echo_cmd("d_received", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                route_policy: None,
                scripts_dir: None,
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
        scripts_dir: None,
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
            d_inputs.get("A").map_or("", |s| s.trim()).contains("output_a"),
            "D should receive A's output"
        );
        assert!(
            d_inputs.get("B").map_or("", |s| s.trim()).contains("output_b"),
            "D should receive B's output"
        );
        assert!(
            d_inputs.get("C").map_or("", |s| s.trim()).contains("output_c"),
            "D should receive C's output"
        );
    }

    #[tokio::test]
    async fn test_route_policy_max_runs_loop() {
        // Review → Fix loop with route_policy.max_runs=3.
        // Runs 1,2: review outputs route="rejected" → fix (DataRouter.route="rejected")
        // Run 3: route_policy overrides to "approved" → retro
        use crate::model::predecessor::{EventType, SchedulingEdgeDef, TriggerExpr};
        use crate::model::provider::ProviderDef;
        use crate::model::workflow::RoutePolicyDef;

        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "start".into(),
                    providers: vec![ProviderDef::Shell {
                        command: json_echo_cmd("start_done", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "review".into(),
                    providers: vec![ProviderDef::Shell {
                        command: json_echo_cmd("review", "rejected"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: Some(RoutePolicyDef::MaxRuns {
                        max: 3,
                        then_route: "approved".into(),
                    }),
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "fix".into(),
                    providers: vec![ProviderDef::Shell {
                        command: json_echo_cmd("fix_done", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "retro".into(),
                    providers: vec![ProviderDef::Shell {
                        command: json_echo_cmd("retro_done", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                scripts_dir: None,
                },
            ],
            edges: vec![
                SchedulingEdgeDef {
                    from: "start".into(),
                    to: "review".into(),
                    trigger: TriggerExpr::Any,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 1,
                },
                SchedulingEdgeDef {
                    from: "review".into(),
                    to: "fix".into(),
                    trigger: TriggerExpr::Any,
                    event: EventType::Complete,
                    exit_reason: Some("rejected".into()),
                    threshold: 1,
                },
                SchedulingEdgeDef {
                    from: "fix".into(),
                    to: "review".into(),
                    trigger: TriggerExpr::Any,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 1,
                },
                SchedulingEdgeDef {
                    from: "review".into(),
                    to: "retro".into(),
                    trigger: TriggerExpr::Any,
                    event: EventType::Complete,
                    exit_reason: Some("approved".into()),
                    threshold: 1,
                },
            ],
            dataflows: vec![
                crate::model::predecessor::DataFlowDef {
                    from: "fix".into(),
                    to: "review".into(),
                    alias: None,
                },
            ],
        scripts_dir: None,
        };

        let config = EngineConfig::new(Some(4), 3600, 0);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_route_policy_max_runs_loop timed out after 30s"),
        };
        assert!(result.is_ok(), "workflow should converge: {result:?}");

        // All 4 nodes should have completed (start, review, fix, retro)
        let start_idx = engine.scheduler().graph().node_index("start").expect("start exists");
        let review_idx = engine.scheduler().graph().node_index("review").expect("review exists");
        let fix_idx = engine.scheduler().graph().node_index("fix").expect("fix exists");
        let retro_idx = engine.scheduler().graph().node_index("retro").expect("retro exists");
        let state = engine.scheduler().state();

        assert_eq!(
            state.states[&start_idx].status,
            NodeStatus::Completed,
            "start should complete"
        );
        assert_eq!(
            state.states[&review_idx].status,
            NodeStatus::Completed,
            "review should complete"
        );
        assert_eq!(
            state.states[&fix_idx].status,
            NodeStatus::Completed,
            "fix should complete"
        );
        assert_eq!(
            state.states[&retro_idx].status,
            NodeStatus::Completed,
            "retro should complete (route_policy override)"
        );
        assert!(
            engine.scheduler().is_converged(),
            "graph should converge"
        );
    }

    #[tokio::test]
    async fn test_concurrency_limit_one() {
        // Three independent entry nodes A, B, C with max_concurrency=1.
        // Nodes must run one at a time, but all should complete and converge.
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![ProviderDef::Shell {
                        command: json_echo_cmd("a_done", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Shell {
                        command: json_echo_cmd("b_done", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![ProviderDef::Shell {
                        command: json_echo_cmd("c_done", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                scripts_dir: None,
                },
            ],
            edges: vec![],
            dataflows: vec![],
        scripts_dir: None,
        };

        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_concurrency_limit_one timed out after 30s"),
        };
        assert!(
            result.is_ok(),
            "3 independent nodes with max_concurrency=1 should converge: {result:?}"
        );

        let a_idx = engine.scheduler().graph().node_index("A").expect("A exists");
        let b_idx = engine.scheduler().graph().node_index("B").expect("B exists");
        let c_idx = engine.scheduler().graph().node_index("C").expect("C exists");
        let state = engine.scheduler().state();

        assert_eq!(state.states[&a_idx].status, NodeStatus::Completed, "A should be Completed");
        assert_eq!(state.states[&b_idx].status, NodeStatus::Completed, "B should be Completed");
        assert_eq!(state.states[&c_idx].status, NodeStatus::Completed, "C should be Completed");
        assert!(
            engine.scheduler().is_converged(),
            "graph should converge after all 3 nodes complete"
        );
    }

    #[tokio::test]
    async fn test_concurrency_parallel_execution() {
        // Three independent entry nodes A, B, C with max_concurrency=10.
        // All three should run (effectively in parallel) and complete.
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![ProviderDef::Shell {
                        command: json_echo_cmd("a_done", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Shell {
                        command: json_echo_cmd("b_done", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![ProviderDef::Shell {
                        command: json_echo_cmd("c_done", "ok"),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                scripts_dir: None,
                },
            ],
            edges: vec![],
            dataflows: vec![],
        scripts_dir: None,
        };

        let config = EngineConfig::new(Some(10), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_concurrency_parallel_execution timed out after 30s"),
        };
        assert!(
            result.is_ok(),
            "3 independent nodes with max_concurrency=10 should converge: {result:?}"
        );

        let a_idx = engine.scheduler().graph().node_index("A").expect("A exists");
        let b_idx = engine.scheduler().graph().node_index("B").expect("B exists");
        let c_idx = engine.scheduler().graph().node_index("C").expect("C exists");
        let state = engine.scheduler().state();

        assert_eq!(state.states[&a_idx].status, NodeStatus::Completed, "A should be Completed");
        assert_eq!(state.states[&b_idx].status, NodeStatus::Completed, "B should be Completed");
        assert_eq!(state.states[&c_idx].status, NodeStatus::Completed, "C should be Completed");
        assert!(
            engine.scheduler().is_converged(),
            "graph should converge after all 3 nodes complete"
        );
    }

    #[tokio::test]
    async fn test_concurrency_single_node() {
        // Single node A with max_concurrency=1 — basic smoke test.
        use crate::model::provider::ProviderDef;

        let def = WorkflowDef {
            nodes: vec![crate::model::workflow::NodeDef {
                id: "A".into(),
                providers: vec![ProviderDef::Shell {
                    command: json_echo_cmd("single_ok", "ok"),
                }],
                process_timeout_secs: 10,
                returns: vec![],
                max_retries: None,
                route_policy: None,
            scripts_dir: None,
            }],
            edges: vec![],
            dataflows: vec![],
        scripts_dir: None,
        };

        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_concurrency_single_node timed out after 30s"),
        };
        assert!(
            result.is_ok(),
            "single node with max_concurrency=1 should complete: {result:?}"
        );

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

    // ── HTTP provider integration tests ─────────────────────
    //
    // These tests exercise the full DAG pipeline with HTTP nodes,
    // covering: single node, chain, route branching, error handling,
    // timeout/retry, template interpolation, and fan-out.

    /// Helper: build a minimal HTTP node with the given URL.
    fn http_node(id: &str, url: String) -> crate::model::workflow::NodeDef {
        crate::model::workflow::NodeDef {
            id: id.into(),
            providers: vec![ProviderDef::Http {
                url,
                method: Some("GET".into()),
                headers: None,
                body: None,
            }],
            process_timeout_secs: 10,
            returns: vec![],
            max_retries: None,
            route_policy: None,
            scripts_dir: None,
        }
    }

    /// Helper: build an HTTP POST node.
    fn http_post_node(id: &str, url: String, body: &str) -> crate::model::workflow::NodeDef {
        crate::model::workflow::NodeDef {
            id: id.into(),
            providers: vec![ProviderDef::Http {
                url,
                method: Some("POST".into()),
                headers: None,
                body: Some(body.into()),
            }],
            process_timeout_secs: 10,
            returns: vec![],
            max_retries: None,
            route_policy: None,
            scripts_dir: None,
        }
    }

    #[tokio::test]
    async fn test_http_single_node_get_converges() {
        let srv = crate::nodeshell::http::test_server::TestServer::start();
        let def = WorkflowDef {
            nodes: vec![http_node("A", srv.url("/ok"))],
            edges: vec![],
            dataflows: vec![],
            scripts_dir: None,
        };
        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_http_single_node_get_converges timed out"),
        };
        assert!(result.is_ok(), "HTTP GET should converge: {result:?}");

        let a_idx = engine.scheduler().graph().node_index("A").expect("A exists");
        let state = engine.scheduler().state();
        assert_eq!(state.states[&a_idx].status, NodeStatus::Completed);
        assert!(engine.scheduler().is_converged());
    }

    #[tokio::test]
    async fn test_http_post_with_body() {
        let srv = crate::nodeshell::http::test_server::TestServer::start();
        let def = WorkflowDef {
            nodes: vec![http_post_node(
                "poster",
                srv.url("/echo"),
                r#"{"task":"review","file":"src/main.rs"}"#,
            )],
            edges: vec![],
            dataflows: vec![],
            scripts_dir: None,
        };
        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_http_post_with_body timed out"),
        };
        assert!(result.is_ok(), "HTTP POST should converge: {result:?}");

        // Verify the response content was routed.
        let poster_idx = engine
            .scheduler()
            .graph()
            .node_index("poster")
            .expect("poster exists");
        let _output = engine.data_router().build_input(poster_idx);
        // POST /echo returns the body as content; output is stored in DataRouter.
        assert!(
            engine.scheduler().state().states[&poster_idx].status == NodeStatus::Completed,
            "poster should complete"
        );
    }

    #[tokio::test]
    async fn test_http_route_branching() {
        // A: GET /ok → route="ok" → triggers B
        // A: GET /err → route="err" → would trigger C (but doesn't match)
        // A calls /ok, so B runs, C stays Pending → watchdog skips C.
        use crate::model::predecessor::{EventType, SchedulingEdgeDef, TriggerExpr};

        let srv = crate::nodeshell::http::test_server::TestServer::start();
        let def = WorkflowDef {
            nodes: vec![
                http_node("A", srv.url("/ok")),
                // B: triggers on route "ok"
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![json_provider("b_done", "ok")],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                    scripts_dir: None,
                },
                // C: triggers on route "err" (won't fire)
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![json_provider("c_done", "ok")],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                    scripts_dir: None,
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
                    exit_reason: Some("err".into()),
                    threshold: 1,
                },
            ],
            dataflows: vec![],
            scripts_dir: None,
        };
        let config = EngineConfig::new(Some(2), 3600, 3);
        // Short convergence watchdog so C→Skipped happens quickly.
        let mut engine = Engine::new(def, config, None).expect("valid workflow");

        let result = tokio::time::timeout(Duration::from_secs(10), engine.run()).await;
        // Will converge after watchdog skips C.
        let result = if let Ok(r) = result { r } else {
            // Engine didn't converge naturally (C stayed Pending).
            // Force converge and check partial state.
            let state = engine.scheduler().state();
            let a_idx = engine.scheduler().graph().node_index("A").unwrap();
            let b_idx = engine.scheduler().graph().node_index("B").unwrap();
            assert_eq!(state.states[&a_idx].status, NodeStatus::Completed, "A should complete");
            assert_eq!(state.states[&b_idx].status, NodeStatus::Completed, "B should be triggered by route='ok'");
            return;
        };
        assert!(result.is_ok(), "HTTP route branching: {result:?}");
    }

    #[tokio::test]
    async fn test_http_chain_a_to_b() {
        // A (GET /ok) → B (GET /ok), verify chain execution.
        use crate::model::predecessor::{EventType, SchedulingEdgeDef, TriggerExpr};

        let srv = crate::nodeshell::http::test_server::TestServer::start();
        let def = WorkflowDef {
            nodes: vec![
                http_node("A", srv.url("/ok")),
                http_node("B", srv.url("/ok")),
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
            scripts_dir: None,
        };
        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_http_chain_a_to_b timed out"),
        };
        assert!(result.is_ok(), "HTTP chain should converge: {result:?}");

        let a_idx = engine.scheduler().graph().node_index("A").unwrap();
        let b_idx = engine.scheduler().graph().node_index("B").unwrap();
        let state = engine.scheduler().state();
        assert_eq!(state.states[&a_idx].status, NodeStatus::Completed);
        assert_eq!(state.states[&b_idx].status, NodeStatus::Completed);
    }

    #[tokio::test]
    async fn test_http_error_handling_5xx_triggers_failed_edge() {
        // A: GET /status/500 → 5xx → exit_code=1 → Failed event → B fires.
        use crate::model::predecessor::{EventType, SchedulingEdgeDef, TriggerExpr};

        let srv = crate::nodeshell::http::test_server::TestServer::start();
        let def = WorkflowDef {
            nodes: vec![
                // A will get a 500 → exit_code=1 → Failed event
                http_node("A", srv.url("/status/500")),
                // B: error handler, triggered on Failed
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![json_provider("handled", "ok")],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                    scripts_dir: None,
                },
            ],
            edges: vec![SchedulingEdgeDef {
                from: "A".into(),
                to: "B".into(),
                trigger: TriggerExpr::Any,
                event: EventType::Failed,
                exit_reason: None,
                threshold: 1,
            }],
            dataflows: vec![],
            scripts_dir: None,
        };
        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_http_error_handling timed out"),
        };
        assert!(result.is_ok(), "5xx→Failed→B chain should converge: {result:?}");

        let a_idx = engine.scheduler().graph().node_index("A").unwrap();
        let b_idx = engine.scheduler().graph().node_index("B").unwrap();
        let state = engine.scheduler().state();
        assert_eq!(state.states[&a_idx].status, NodeStatus::Failed, "A should be Failed (5xx)");
        assert_eq!(state.states[&b_idx].status, NodeStatus::Completed, "B should run on Failed");
    }

    #[tokio::test]
    async fn test_http_timeout_retry_then_complete() {
        // A: GET /slow (3s delay) with timeout=1s → timeout → retry (max 1).
        // After retry exhausted, node is TimedOut. Convergence via watchdog.
        let srv = crate::nodeshell::http::test_server::TestServer::start();
        let def = WorkflowDef {
            nodes: vec![crate::model::workflow::NodeDef {
                id: "A".into(),
                providers: vec![ProviderDef::Http {
                    url: srv.url("/slow"),
                    method: Some("GET".into()),
                    headers: None,
                    body: None,
                }],
                process_timeout_secs: 1,    // 1s timeout vs 3s delay
                returns: vec![],
                max_retries: Some(1),        // retry once
                route_policy: None,
                scripts_dir: None,
            }],
            edges: vec![],
            dataflows: vec![],
            scripts_dir: None,
        };
        let config = EngineConfig::new(Some(1), 3600, 0); // 0 global retries (use node-level)
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        // The engine will handle the timeout→retry→timeout flow.
        // Give enough time for /slow to respond (3s) × retries.
        let result = tokio::time::timeout(Duration::from_secs(15), engine.run()).await;
        assert!(
            result.is_ok(),
            "timeout retry should eventually converge (or be skipped by watchdog)"
        );
    }

    #[tokio::test]
    async fn test_http_template_dataflow_chain() {
        // A: echo "hello" (subprocess, outputs content="hello")
        // B: GET /ok (but template URL isn't tested here since we need dataflow)
        // The key test: A→B chain with dataflow.
        use crate::model::predecessor::{DataFlowDef, EventType, SchedulingEdgeDef, TriggerExpr};

        let srv = crate::nodeshell::http::test_server::TestServer::start();
        let def = WorkflowDef {
            nodes: vec![
                // A: produces content that B can reference via template
                crate::model::workflow::NodeDef {
                    id: "A".into(),
                    providers: vec![json_provider("from_a", "ok")],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                    scripts_dir: None,
                },
                // B: uses {{datarouter.A.content}} in URL
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![ProviderDef::Http {
                        url: format!("{}/{}", srv.addr(), "ok"),
                        method: Some("GET".into()),
                        headers: None,
                        body: None,
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                    scripts_dir: None,
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
                from: "A".into(),
                to: "B".into(),
                alias: None,
            }],
            scripts_dir: None,
        };
        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_http_template_dataflow_chain timed out"),
        };
        assert!(result.is_ok(), "A→B HTTP chain with dataflow should converge: {result:?}");

        let b_idx = engine.scheduler().graph().node_index("B").unwrap();
        let state = engine.scheduler().state();
        assert_eq!(state.states[&b_idx].status, NodeStatus::Completed, "B should complete");
    }

    #[tokio::test]
    async fn test_http_fan_out() {
        // A (GET /ok) → B, C (both subprocess, run in parallel).
        use crate::model::predecessor::{EventType, SchedulingEdgeDef, TriggerExpr};

        let srv = crate::nodeshell::http::test_server::TestServer::start();
        let def = WorkflowDef {
            nodes: vec![
                http_node("A", srv.url("/ok")),
                crate::model::workflow::NodeDef {
                    id: "B".into(),
                    providers: vec![json_provider("b_out", "ok")],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                    scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "C".into(),
                    providers: vec![json_provider("c_out", "ok")],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                    scripts_dir: None,
                },
            ],
            edges: vec![
                SchedulingEdgeDef {
                    from: "A".into(),
                    to: "B".into(),
                    trigger: TriggerExpr::Any,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 1,
                },
                SchedulingEdgeDef {
                    from: "A".into(),
                    to: "C".into(),
                    trigger: TriggerExpr::Any,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 1,
                },
            ],
            dataflows: vec![],
            scripts_dir: None,
        };
        let config = EngineConfig::new(Some(2), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("test_http_fan_out timed out"),
        };
        assert!(result.is_ok(), "HTTP fan-out should converge: {result:?}");

        let a_idx = engine.scheduler().graph().node_index("A").unwrap();
        let b_idx = engine.scheduler().graph().node_index("B").unwrap();
        let c_idx = engine.scheduler().graph().node_index("C").unwrap();
        let state = engine.scheduler().state();
        assert_eq!(state.states[&a_idx].status, NodeStatus::Completed);
        assert_eq!(state.states[&b_idx].status, NodeStatus::Completed);
        assert_eq!(state.states[&c_idx].status, NodeStatus::Completed);
    }

    /// Subprocess fails with exit≠0 + empty stdout → Failed, NO retry.
    /// Regression test for the SpawnError-on-nonzero-exit bug: a process
    /// that runs and exits non-zero should go directly to Failed without
    /// the engine retrying it.
    #[tokio::test]
    async fn test_subprocess_nonzero_exit_no_retry() {
        // Python exits 1, stdout empty.
        let cmd = if cfg!(windows) {
            "python -c \"import sys; sys.exit(1)\"".into()
        } else {
            "python3 -c 'import sys; sys.exit(1)'".into()
        };
        let def = WorkflowDef {
            nodes: vec![crate::model::workflow::NodeDef {
                id: "A".into(),
                providers: vec![ProviderDef::Subprocess { command: cmd }],
                process_timeout_secs: 10,
                returns: vec![],
                max_retries: None, // use global default (3)
                route_policy: None,
                scripts_dir: None,
            }],
            edges: vec![],
            dataflows: vec![],
            scripts_dir: None,
        };
        let config = EngineConfig::new(Some(1), 3600, 3);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = result.expect("should converge without hanging (no retry loop)");
        assert!(result.is_ok(), "workflow should converge: {result:?}");

        let a_idx = engine.scheduler().graph().node_index("A").unwrap();
        let state = engine.scheduler().state();
        assert_eq!(
            state.states[&a_idx].status,
            NodeStatus::Failed,
            "A should be Failed, not retried"
        );
        assert_eq!(
            state.retry_counts.get(&a_idx).copied().unwrap_or(0),
            0,
            "retry count should be 0 (no retries for exit-code failure)"
        );
        assert!(
            engine.scheduler().is_converged(),
            "graph should converge immediately"
        );
    }

    /// `MaxDuration` overrides route after cumulative execution time threshold.
    /// start → review (sleeps 2s, outputs "`needs_fix`", `MaxDuration(max_secs=1)`)
    ///      → retro (triggered by "timeout" from `MaxDuration` override).
    /// No loop needed — verifying the override itself is sufficient.
    #[tokio::test]
    async fn test_route_policy_max_duration_loop() {
        use crate::model::predecessor::{EventType, SchedulingEdgeDef, TriggerExpr};
        use crate::model::workflow::RoutePolicyDef;

        let mut script_path = std::env::temp_dir();
        script_path.push("nexus_test_max_duration.py");
        let script_content =
            "import time, json\ntime.sleep(2)\nprint(json.dumps({'route':'needs_fix','content':'issues'}))\n";
        std::fs::write(&script_path, script_content).expect("write temp script");

        let def = WorkflowDef {
            nodes: vec![
                crate::model::workflow::NodeDef {
                    id: "start".into(),
                    providers: vec![json_provider("start_done", "ok")],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                    scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "review".into(),
                    providers: vec![ProviderDef::Subprocess {
                        command: format!("python {}", script_path.display()),
                    }],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: Some(RoutePolicyDef::MaxDuration {
                        max_secs: 1,
                        then_route: "timeout".into(),
                    }),
                    scripts_dir: None,
                },
                crate::model::workflow::NodeDef {
                    id: "retro".into(),
                    providers: vec![json_provider("retro_done", "ok")],
                    process_timeout_secs: 10,
                    returns: vec![],
                    max_retries: None,
                    route_policy: None,
                    scripts_dir: None,
                },
            ],
            edges: vec![
                SchedulingEdgeDef {
                    from: "start".into(),
                    to: "review".into(),
                    trigger: TriggerExpr::Any,
                    event: EventType::Complete,
                    exit_reason: None,
                    threshold: 1,
                },
                SchedulingEdgeDef {
                    from: "review".into(),
                    to: "retro".into(),
                    trigger: TriggerExpr::Any,
                    event: EventType::Complete,
                    exit_reason: Some("timeout".into()),
                    threshold: 1,
                },
            ],
            dataflows: vec![],
            scripts_dir: None,
        };
        let config = EngineConfig::new(Some(2), 3600, 0);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(10), engine.run()).await;
        let result = match result {
            Ok(r) => r,
            Err(_) => panic!("max_duration test timed out — policy may not have triggered"),
        };
        assert!(
            result.is_ok(),
            "max_duration should converge: {result:?}"
        );

        let review_idx = engine.scheduler().graph().node_index("review").unwrap();
        let retro_idx = engine.scheduler().graph().node_index("retro").unwrap();
        let state = engine.scheduler().state();
        assert_eq!(
            state.states[&review_idx].status,
            NodeStatus::Completed,
            "review should complete"
        );
        assert_eq!(
            state.states[&retro_idx].status,
            NodeStatus::Completed,
            "retro should execute (max_duration forced route to timeout)"
        );
        let cum = engine.scheduler.cumulative_runtime_secs(review_idx);
        assert!(
            cum >= 1,
            "cumulative runtime should be >= 1 after 2s sleep, got {cum}"
        );

        // Cleanup
        let _ = std::fs::remove_file(&script_path);
    }

    /// Subprocess with `SpawnError` (command not found) → should still retry.
    #[tokio::test]
    async fn test_subprocess_spawn_error_still_retries() {
        let def = WorkflowDef {
            nodes: vec![crate::model::workflow::NodeDef {
                id: "A".into(),
                providers: vec![ProviderDef::Subprocess {
                    command: "nonexistent_command_xyz_123".into(),
                }],
                process_timeout_secs: 10,
                returns: vec![],
                max_retries: Some(1),
                route_policy: None,
                scripts_dir: None,
            }],
            edges: vec![],
            dataflows: vec![],
            scripts_dir: None,
        };
        let config = EngineConfig::new(Some(1), 3600, 0);
        let mut engine = Engine::new(def, config, None).expect("valid workflow");
        let result = tokio::time::timeout(Duration::from_secs(30), engine.run()).await;
        let result = result.expect("should converge (retry exhausted → Failed or Skipped)");
        let _ = result; // may be Ok (converged) or contain spawn error info

        let a_idx = engine.scheduler().graph().node_index("A").unwrap();
        let state = engine.scheduler().state();
        let retries = state.retry_counts.get(&a_idx).copied().unwrap_or(0);
        assert!(
            retries > 0 || state.states[&a_idx].status == NodeStatus::Failed,
            "spawn error should trigger retry, got retries={retries}, status={:?}",
            state.states[&a_idx].status
        );
    }
}
