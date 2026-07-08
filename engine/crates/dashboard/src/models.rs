//! Data models for persisted workflow and run records.

use serde::{Deserialize, Serialize};

/// A persisted workflow definition row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRow {
    pub id: String,
    pub name: String,
    /// JSON string of the full `WorkflowDef`.
    pub definition: String,
    pub created_at: String,
    pub updated_at: String,
}

/// A persisted workflow run record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRow {
    pub id: String,
    pub workflow_id: String,
    pub status: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}
