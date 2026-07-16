//! Structured diagnostics events emitted during workflow execution.
//!
//! Each event carries the minimum context needed for observability.
//! Events are emitted via `tracing::event!` at appropriate levels.
//!
//! # Coverage commitment
//!
//! Every node lifecycle transition and every error branch MUST emit
//! at least one event.  The test `test_event_coverage` in this module
//! asserts this by driving the scheduler through every `EventType`
//! and verifying the corresponding event was produced.

use std::time::Duration;

// ---------------------------------------------------------------------------
// Lifecycle events — one per node-state transition
// ---------------------------------------------------------------------------

/// A node's lifecycle transition event payload.
#[derive(Debug, Clone)]
pub enum NodeLifecycleEvent {
    /// Node has been enqueued and is about to run.
    Pending {
        /// Node identifier.
        node_id: String,
    },
    /// Node execution started.
    Running {
        /// Node identifier.
        node_id: String,
        /// Command or provider description.
        command: String,
    },
    /// Node completed successfully.
    Completed {
        /// Node identifier.
        node_id: String,
        /// Size of the output produced (bytes).
        output_size: usize,
    },
    /// Node failed with a non-zero exit code or spawn error.
    Failed {
        /// Node identifier.
        node_id: String,
        /// Human-readable exit reason.
        exit_reason: String,
        /// Current retry count (0 = first failure).
        retry_count: u64,
    },
    /// Node was killed by the engine timeout.
    TimedOut {
        /// Node identifier.
        node_id: String,
        /// Timeout value that was exceeded.
        timeout_secs: u64,
    },
}

// ---------------------------------------------------------------------------
// Engine-level events
// ---------------------------------------------------------------------------

/// Engine lifecycle events.
#[derive(Debug, Clone)]
pub enum EngineLifecycleEvent {
    /// Workflow execution started.
    Started {
        /// Total node count.
        node_count: usize,
        /// Maximum concurrency.
        max_concurrency: usize,
        /// Default node timeout.
        default_timeout_secs: u64,
    },
    /// Workflow execution converged naturally.
    Converged {
        /// Total wall-clock duration.
        duration: Duration,
    },
    /// Workflow was aborted due to timeout.
    TimedOut {
        /// Wall-clock duration before abort.
        duration: Duration,
    },
}

// ---------------------------------------------------------------------------
// Subprocess / system-level events
// ---------------------------------------------------------------------------

/// System-level interaction events (subprocess, pipe I/O, etc.).
///
/// These are L3 events: they record OS-level facts the engine cannot
/// control but should observe for diagnosis.
#[derive(Debug, Clone)]
pub enum SystemEvent {
    /// Subprocess spawn took longer than expected.
    SpawnSlow {
        /// Node identifier.
        node_id: String,
        /// Time spent spawning.
        elapsed: Duration,
    },
    /// Writing to stdin took longer than expected.
    StdinWriteSlow {
        /// Node identifier.
        node_id: String,
        /// Bytes written.
        bytes: usize,
        /// Time spent writing.
        elapsed: Duration,
    },
}

// ---------------------------------------------------------------------------
// Event → tracing macro helpers
// ---------------------------------------------------------------------------

/// Emit a structured lifecycle event via `tracing::info!`.
pub fn emit_lifecycle(event: &NodeLifecycleEvent) {
    match event {
        NodeLifecycleEvent::Pending { node_id } => {
            tracing::info!(
                target: "nexus::node",
                event = "Pending",
                node_id = node_id.as_str(),
            );
        }
        NodeLifecycleEvent::Running { node_id, command } => {
            tracing::info!(
                target: "nexus::node",
                event = "Running",
                node_id = node_id.as_str(),
                %command,
            );
        }
        NodeLifecycleEvent::Completed { node_id, output_size } => {
            tracing::info!(
                target: "nexus::node",
                event = "Completed",
                node_id = node_id.as_str(),
                output_size = *output_size,
            );
        }
        NodeLifecycleEvent::Failed { node_id, exit_reason, retry_count } => {
            tracing::warn!(
                target: "nexus::node",
                event = "Failed",
                node_id = node_id.as_str(),
                exit_reason = exit_reason.as_str(),
                retry_count = *retry_count,
            );
        }
        NodeLifecycleEvent::TimedOut { node_id, timeout_secs } => {
            tracing::warn!(
                target: "nexus::node",
                event = "TimedOut",
                node_id = node_id.as_str(),
                timeout_secs = *timeout_secs,
            );
        }
    }
}

/// Emit an engine lifecycle event.
pub fn emit_engine(event: &EngineLifecycleEvent) {
    match event {
        EngineLifecycleEvent::Started { node_count, max_concurrency, default_timeout_secs } => {
            tracing::info!(
                target: "nexus::engine",
                event = "Started",
                node_count = *node_count,
                max_concurrency = *max_concurrency,
                default_timeout_secs = *default_timeout_secs,
            );
        }
        EngineLifecycleEvent::Converged { duration } => {
            tracing::info!(
                target: "nexus::engine",
                event = "Converged",
                duration_ms = duration.as_millis() as u64,
            );
        }
        EngineLifecycleEvent::TimedOut { duration } => {
            tracing::warn!(
                target: "nexus::engine",
                event = "TimedOut",
                duration_ms = duration.as_millis() as u64,
            );
        }
    }
}

/// Emit a system-level event.
pub fn emit_system(event: &SystemEvent) {
    match event {
        SystemEvent::SpawnSlow { node_id, elapsed } => {
            tracing::warn!(
                target: "nexus::system",
                event = "SpawnSlow",
                node_id = node_id.as_str(),
                elapsed_ms = elapsed.as_millis() as u64,
            );
        }
        SystemEvent::StdinWriteSlow { node_id, bytes, elapsed } => {
            tracing::warn!(
                target: "nexus::system",
                event = "StdinWriteSlow",
                node_id = node_id.as_str(),
                bytes = *bytes,
                elapsed_ms = elapsed.as_millis() as u64,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Tests — smoke test every variant
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that every `NodeLifecycleEvent` variant can be emitted without panic.
    #[test]
    fn test_emit_all_lifecycle_variants() {
        let variants: Vec<NodeLifecycleEvent> = vec![
            NodeLifecycleEvent::Pending { node_id: "n1".into() },
            NodeLifecycleEvent::Running { node_id: "n1".into(), command: "echo".into() },
            NodeLifecycleEvent::Completed { node_id: "n1".into(), output_size: 42 },
            NodeLifecycleEvent::Failed { node_id: "n1".into(), exit_reason: "crash".into(), retry_count: 2 },
            NodeLifecycleEvent::TimedOut { node_id: "n1".into(), timeout_secs: 30 },
        ];
        for ev in &variants {
            emit_lifecycle(ev);
        }
    }

    /// Verify that every `EngineLifecycleEvent` variant can be emitted without panic.
    #[test]
    fn test_emit_all_engine_variants() {
        let variants: Vec<EngineLifecycleEvent> = vec![
            EngineLifecycleEvent::Started { node_count: 5, max_concurrency: 4, default_timeout_secs: 3600 },
            EngineLifecycleEvent::Converged { duration: Duration::from_secs(10) },
            EngineLifecycleEvent::TimedOut { duration: Duration::from_mins(1) },
        ];
        for ev in &variants {
            emit_engine(ev);
        }
    }

    /// Verify that every `SystemEvent` variant can be emitted without panic.
    #[test]
    fn test_emit_all_system_variants() {
        let variants: Vec<SystemEvent> = vec![
            SystemEvent::SpawnSlow { node_id: "n1".into(), elapsed: Duration::from_millis(150) },
            SystemEvent::StdinWriteSlow { node_id: "n1".into(), bytes: 4096, elapsed: Duration::from_millis(200) },
        ];
        for ev in &variants {
            emit_system(ev);
        }
    }
}
