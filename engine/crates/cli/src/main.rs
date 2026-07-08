//! Nexus CLI — Command-line interface for the Nexus workflow engine.
//!
//! Usage: nexus run <workflow.json> [OPTIONS]
//!
//! This binary needs unsafe code for Windows console API (SetConsoleMode).
#![cfg_attr(windows, allow(unsafe_code))]

// dev-dependency anchor for unused_crate_dependencies lint.
#[cfg(test)]
#[doc(hidden)]
pub use tempfile as _tempfile_anchor; // tempfile used in tests

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use nexus_engine::diagnostics::event::NodeLifecycleEvent;
use nexus_engine::graph::validate;
use nexus_engine::model::{EngineConfig, WorkflowDef};
use nexus_engine::runtime::{Engine, NodeEvent};

/// Nexus workflow engine CLI.
#[derive(Parser)]
#[command(name = "nexus", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Run a workflow from a JSON file.
    Run {
        /// Path to the workflow JSON file.
        path: PathBuf,

        /// Maximum number of concurrent nodes (default: CPU count).
        #[arg(long)]
        max_concurrency: Option<usize>,

        /// Default node timeout in seconds (default: 3600).
        /// A node's `process_timeout_secs` overrides this value.
        #[arg(long)]
        node_timeout: Option<u64>,

        /// Maximum retries for timed out or spawn-failed nodes (default: 3).
        /// Exit-code failures are NOT automatically retried.
        #[arg(long, default_value = "3")]
        max_timeout_retries: u64,

        /// Enable verbose logging.
        #[arg(long, short)]
        verbose: bool,

        /// Only validate the workflow, do not execute.
        #[arg(long)]
        validate_only: bool,

        /// Dump engine state snapshot on completion or error.
        #[arg(long)]
        dump_state: bool,
    },
}

#[tokio::main]
async fn main() {
    // Enable ANSI escape sequence processing on Windows.
    #[cfg(windows)]
    {
        #[allow(unsafe_code)]
        unsafe {
            let handle = winapi::um::processenv::GetStdHandle(
                winapi::um::winbase::STD_ERROR_HANDLE,
            );
            if handle != winapi::um::handleapi::INVALID_HANDLE_VALUE {
                let mut mode: u32 = 0;
                if winapi::um::consoleapi::GetConsoleMode(handle, &mut mode) != 0 {
                    let _ = winapi::um::consoleapi::SetConsoleMode(
                        handle,
                        mode | winapi::um::wincon::ENABLE_VIRTUAL_TERMINAL_PROCESSING,
                    );
                }
            }
        }
    }

    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            path,
            max_concurrency,
            node_timeout,
            max_timeout_retries,
            verbose,
            validate_only,
            dump_state,
        } => {
            // Initialize tracing — write to log/ directory
            let log_dir = PathBuf::from("log");
            let _ = fs::create_dir_all(&log_dir);
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let log_file = log_dir.join(format!("run-{timestamp}.log"));

            let log_file_clone = log_file.clone();
            let make_writer = std::sync::Mutex::new(
                fs::File::create(&log_file).expect("failed to create log file"),
            );

            let level = if verbose { "debug" } else { "info" };
            tracing_subscriber::fmt()
                .with_env_filter(level)
                .with_writer(make_writer)
                .with_ansi(false)
                .init();

            eprintln!("Log: {}", log_file_clone.display());

            // Read workflow file
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Error reading '{}': {}", path.display(), e);
                    std::process::exit(1);
                }
            };

            // Parse workflow
            let def: WorkflowDef = match serde_json::from_str(&content) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Error parsing workflow JSON: {}", e);
                    std::process::exit(1);
                }
            };

            // Validate
            if let Err(errors) = validate(&def) {
                for err in &errors {
                    eprintln!("Validation error: {}", err);
                }
                if validate_only {
                    std::process::exit(1);
                }
                std::process::exit(1);
            }

            if validate_only {
                println!("Workflow validation passed.");
                return;
            }

            // Create config
            let config = EngineConfig::new(
                max_concurrency,
                node_timeout.unwrap_or(3600),
                max_timeout_retries,
            );

            // Create the event callback that logs events to stderr in real time.
            let event_cb: nexus_engine::runtime::NodeEventCb = Arc::new(move |event| {
                use std::io::Write;
                match event {
                    NodeEvent::Lifecycle(lifecycle) => match lifecycle {
                        NodeLifecycleEvent::Running { node_id, .. } => {
                            let _ = writeln!(std::io::stderr(), "[>] {node_id}: running...").ok();
                        }
                        NodeLifecycleEvent::Completed { node_id, output_size } => {
                            let _ = writeln!(std::io::stderr(), "[v] {node_id}: completed ({output_size} bytes)").ok();
                        }
                        NodeLifecycleEvent::Failed { node_id, exit_reason, .. } => {
                            let _ = writeln!(std::io::stderr(), "[x] {node_id}: failed ({exit_reason})").ok();
                        }
                        NodeLifecycleEvent::TimedOut { node_id, timeout_secs } => {
                            let _ = writeln!(std::io::stderr(), "[x] {node_id}: timed out ({timeout_secs}s)").ok();
                        }
                        NodeLifecycleEvent::Pending { .. } => {} // not displayed
                    },
                    NodeEvent::NodeChunk { node_id, text } => {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() && trimmed.len() < 120 {
                            let _ = writeln!(std::io::stderr(), "[ ] {node_id}: {trimmed}").ok();
                        }
                    }
                    // Legacy flat variants — kept for backward compat; Lifecycle supersedes these.
                    NodeEvent::NodeRunning { node_id, .. } => {
                        let _ = writeln!(std::io::stderr(), "[>] {node_id}: running...").ok();
                    }
                    NodeEvent::NodeCompleted { node_id } => {
                        let _ = writeln!(std::io::stderr(), "[v] {node_id}: completed").ok();
                    }
                    NodeEvent::NodeFailed { node_id } => {
                        let _ = writeln!(std::io::stderr(), "[x] {node_id}: failed").ok();
                    }
                    NodeEvent::NodeTimedOut { node_id } => {
                        let _ = writeln!(std::io::stderr(), "[x] {node_id}: timed out").ok();
                    }
                }
            });

            // Create and run engine
            let mut engine = match Engine::new(def, config, Some(event_cb)) {
                Ok(e) => e,
                Err(errors) => {
                    for err in &errors {
                        eprintln!("Build error: {}", err);
                    }
                    std::process::exit(1);
                }
            };

            let result = engine.run().await;

            match result {
                Ok(result) => {
                    if dump_state {
                        eprintln!("{}", result.snapshot);
                    }
                    println!("Workflow completed successfully.");
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Runtime error: {:?}", e);
                    std::process::exit(2);
                }
            }
        }
    }
}
