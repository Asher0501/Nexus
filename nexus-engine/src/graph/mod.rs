//! Graph building, validation, and scheduling types.
//!
//! This module handles the syntax layer of the engine:
//! - [`GraphDef`]: The verified, aggregated graph definition
//! - [`Builder`]: Converts `WorkflowDef` into `GraphDef`
//! - [`Validator`]: Pre-execution validation
//! - [`Scheduler`]: Runtime event handling and state management
//! - [`EdgeDef`], [`EdgeState`], [`Strategy`]: Edge types
//! - [`DataRouter`]: Upstream-to-downstream data routing

/// Edge definition and runtime state types (`EdgeDef`, `EdgeState`, `Strategy`).
pub mod edge;

/// Aggregated graph definition with invariant checks (`GraphDef`, `NodeTransfer`, `NodeData`).
pub mod graph_def;

/// Graph builder — converts [`WorkflowDef`](crate::model::WorkflowDef) into [`GraphDef`].
pub mod builder;

/// Pre-execution workflow validator.
pub mod validator;

/// Runtime data router for upstream-to-downstream data flow.
pub mod data_router;

/// Graph execution scheduler with event handling and state management.
pub mod scheduler;

pub use builder::Builder;
pub use data_router::DataRouter;
pub use edge::{EdgeDef, EdgeState, Strategy};
pub use graph_def::{GraphDef, NodeData, NodeParams, NodeTransfer};
pub use scheduler::{NodeCounters, NodeResult, NodeState, NodeStatus, RuntimeState, Scheduler};
pub use validator::validate;
