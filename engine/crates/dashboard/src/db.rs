//! SQLite persistence layer for workflows and runs.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, Result};

use crate::models::{RunRow, WorkflowRow};

/// Thread-safe SQLite store for dashboards.
///
/// Uses `Mutex<Connection>` since rusqlite's `Connection` is not `Send`.
/// All public methods are blocking — callers should wrap in `tokio::task::spawn_blocking`
/// if called from async contexts.
///
/// Implements `Clone` via `Arc` for use as axum shared state.
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    /// Open (or create) the SQLite database at `path`.
    ///
    /// When `path` is `None`, uses `{data_dir}/nexus/dashboard.db` on supported platforms,
    /// or `.nexus/dashboard.db` relative to the current directory as fallback.
    pub fn new(path: Option<PathBuf>) -> Result<Self> {
        let db_path = path.unwrap_or_else(default_db_path);
        let conn = Connection::open(&db_path)?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS workflows (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                definition TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS runs (
                id TEXT PRIMARY KEY,
                workflow_id TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                started_at TEXT,
                finished_at TEXT,
                snapshot TEXT,
                FOREIGN KEY (workflow_id) REFERENCES workflows(id)
            );
            ",
        )?;

        tracing::info!("[Dashboard.DB] opened: {:?}", db_path);
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    // ── Workflows ──────────────────────────────────────────

    /// List all workflows ordered by most-recently-updated first.
    pub fn list_workflows(&self) -> Result<Vec<WorkflowRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, definition, created_at, updated_at FROM workflows ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(WorkflowRow {
                id: row.get(0)?,
                name: row.get(1)?,
                definition: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Get a single workflow by `id`.
    pub fn get_workflow(&self, id: &str) -> Result<Option<WorkflowRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, definition, created_at, updated_at FROM workflows WHERE id = ?",
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(WorkflowRow {
                id: row.get(0)?,
                name: row.get(1)?,
                definition: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?;
        rows.next().transpose()
    }

    /// Insert a new workflow.
    pub fn create_workflow(&self, id: &str, name: &str, definition: &str) -> Result<()> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        conn.execute(
            "INSERT INTO workflows (id, name, definition) VALUES (?1, ?2, ?3)",
            params![id, name, definition],
        )?;
        Ok(())
    }

    /// Update an existing workflow's name and definition.
    pub fn update_workflow(&self, id: &str, name: &str, definition: &str) -> Result<()> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        conn.execute(
            "UPDATE workflows SET name = ?1, definition = ?2, updated_at = datetime('now') WHERE id = ?3",
            params![name, definition, id],
        )?;
        Ok(())
    }

    /// Delete a workflow by `id` and all of its associated runs.
    pub fn delete_workflow(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        conn.execute("DELETE FROM runs WHERE workflow_id = ?", params![id])?;
        conn.execute("DELETE FROM workflows WHERE id = ?", params![id])?;
        Ok(())
    }

    // ── Runs ───────────────────────────────────────────────

    /// Insert a new run record with status `'running'`.
    pub fn create_run(&self, id: &str, workflow_id: &str) -> Result<()> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        conn.execute(
            "INSERT INTO runs (id, workflow_id, status, started_at) VALUES (?1, ?2, 'running', datetime('now'))",
            params![id, workflow_id],
        )?;
        Ok(())
    }

    /// Mark a run as finished with the given `status` and optional snapshot JSON.
    pub fn finish_run(&self, id: &str, status: &str, snapshot: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        conn.execute(
            "UPDATE runs SET status = ?1, finished_at = datetime('now'), snapshot = ?2 WHERE id = ?3",
            params![status, snapshot, id],
        )?;
        Ok(())
    }

    /// List all runs ordered by most-recently-started first.
    pub fn list_runs(&self) -> Result<Vec<RunRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, workflow_id, status, started_at, finished_at FROM runs ORDER BY started_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RunRow {
                id: row.get(0)?,
                workflow_id: row.get(1)?,
                status: row.get(2)?,
                started_at: row.get(3)?,
                finished_at: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// List runs for a specific workflow.
    pub fn list_runs_for_workflow(&self, workflow_id: &str) -> Result<Vec<RunRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, workflow_id, status, started_at, finished_at FROM runs WHERE workflow_id = ? ORDER BY started_at DESC",
        )?;
        let rows = stmt.query_map(params![workflow_id], |row| {
            Ok(RunRow {
                id: row.get(0)?,
                workflow_id: row.get(1)?,
                status: row.get(2)?,
                started_at: row.get(3)?,
                finished_at: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Get a single run by `id`.
    pub fn get_run(&self, id: &str) -> Result<Option<RunRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, workflow_id, status, started_at, finished_at FROM runs WHERE id = ?",
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(RunRow {
                id: row.get(0)?,
                workflow_id: row.get(1)?,
                status: row.get(2)?,
                started_at: row.get(3)?,
                finished_at: row.get(4)?,
            })
        })?;
        rows.next().transpose()
    }
}

/// Determine default database path.
///
/// On Unix (Linux/macOS): `$XDG_DATA_HOME/nexus/dashboard.db` or `~/.local/share/nexus/dashboard.db`.
/// On Windows: `%APPDATA%/nexus/dashboard.db`.
/// Falls back to `./.nexus/dashboard.db` when home directory cannot be determined.
fn default_db_path() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("APPDATA").ok().map(PathBuf::from).or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".local").join("share"))
            })
        })
        .unwrap_or_else(|| PathBuf::from(".nexus"));

    let db_dir = base.join("nexus");
    std::fs::create_dir_all(&db_dir).ok();
    db_dir.join("dashboard.db")
}
