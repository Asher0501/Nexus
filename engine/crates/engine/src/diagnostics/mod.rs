//! Diagnostics module for workflow execution observability.
//!
//! This module is **read-only** with respect to engine internals:
//! it defines event types and snapshot structs derived from the
//! public API of [`Scheduler`] and [`Engine`], but never mutates
//! or locks engine state.
//!
//! # Problem-level coverage
//!
//! | Level | Scope | Mechanism |
//! |-------|-------|-----------|
//! | L1 - Engine errors | Deterministic engine logic errors | `Result` + `ValidationError` + lifecycle events |
//! | L2 - Node errors | Node business-logic failures | `EventType`/`NodeResult` + lifecycle events |
//! | L3 - System exceptions | OS/runtime unpredictability | System events + snapshot dump |
//! | L4 - Configuration/env | Startup dependency issues | Clear error messages at CLI layer |
//!
//! # Modules
//!
//! - [`event`] — Structured tracing event definitions and emit helpers
//! - [`snapshot`] — Point-in-time engine state dump
//! - [`trace`] — Trace ID generation and propagation
//!
//! [`Scheduler`]: crate::graph::scheduler::Scheduler
//! [`Engine`]: crate::runtime::Engine

pub mod event;
pub mod snapshot;
pub mod trace;
