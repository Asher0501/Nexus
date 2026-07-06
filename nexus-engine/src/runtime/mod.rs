//! Runtime engine for executing workflows.
//!
//! The runtime module contains:
//! - [`Engine`]: The top-level workflow execution engine
//! - [`EngineConfig`]: Runtime configuration (re-exported from model)
//!
//! The engine implements the main event loop that processes `NodeCompleted`
//! events and schedules `NodeReady` nodes for execution.

mod engine;

pub use engine::{Engine, RuntimeError};
