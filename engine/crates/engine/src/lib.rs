//! Nexus Engine — A directed-graph-driven plugin orchestration engine.
//!
//! This crate provides the core engine library for parsing workflow definitions,
//! validating them, building execution graphs, and running them.
//!
//! # Architecture
//! The engine is organized into modules:
//! - `model`: Core types for workflow definitions, configuration, and errors
//! - `graph`: Graph builder, validator, edge model, and scheduler
//! - `runtime`: Event loop and engine execution
//! - `nodeshell`: Node execution adapters
//! - `diagnostics`: Observability events, snapshots, and trace IDs

#![deny(unsafe_code)]
#![deny(missing_docs)]
// dev-dependencies used in #[cfg(test)] modules — anchor to satisfy unused_crate_dependencies lint.
#[cfg(test)]
#[doc(hidden)]
pub use assert_json_diff as _assert_json_diff_dev;
#[cfg(test)]
#[doc(hidden)]
pub use tempfile as _tempfile_dev;

// Future-phase dependency anchors — these crates will be used in Phases 5-7.
// The `unused_crate_dependencies = "deny"` lint requires an explicit anchor.
#[doc(hidden)]
pub use serde_json as _serde_json_anchor;
#[doc(hidden)]
pub use thiserror as _thiserror_anchor;
#[doc(hidden)]
pub use tokio as _tokio_anchor;
#[doc(hidden)]
pub use tracing as _tracing_anchor;

/// Node execution adapters (subprocess, HTTP, etc.).
pub mod nodeshell;

/// Runtime engine for executing workflows.
pub mod runtime;

pub mod graph;
pub mod model;

/// Diagnostics for workflow execution observability.
pub mod diagnostics;
