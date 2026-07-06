//! Nexus CLI — Command-line interface for the Nexus workflow engine.
//!
//! Usage: nexus run <workflow.json> [OPTIONS]

use std::fs;
use std::path::PathBuf;

use clap::Parser;
use nexus_engine::diagnostics::snapshot::EngineSnapshot;
use nexus_engine::graph::validate;
use nexus_engine::model::{EngineConfig, WorkflowDef};
use nexus_engine::runtime::{Engine, RuntimeError};
#[cfg(test)]
#[doc(hidden)]
pub use tempfile as _tempfile_dev;

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
    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            path,
            max_concurrency,
            node_timeout,
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
                3,
            );

            // Create and run engine
            let mut engine = match Engine::new(def, config) {
                Ok(e) => e,
                Err(errors) => {
                    for err in &errors {
                        eprintln!("Build error: {}", err);
                    }
                    std::process::exit(1);
                }
            };

            let started_at = std::time::Instant::now();
            let result = engine.run().await;

            // Dump state snapshot if requested.
            if dump_state {
                let snapshot = EngineSnapshot::capture(
                    engine.scheduler(),
                    engine.running_count(),
                    started_at,
                );
                eprintln!("{snapshot}");
            }

            match result {
                Ok(_result) => {
                    println!("Workflow completed successfully.");
                    std::process::exit(0);
                }
                Err(RuntimeError::IdleTimeout) => {
                    eprintln!("Workflow stalled: event loop idle timeout.");
                    std::process::exit(3);
                }
                Err(e) => {
                    eprintln!("Runtime error: {:?}", e);
                    std::process::exit(2);
                }
            }
        }
    }
}
