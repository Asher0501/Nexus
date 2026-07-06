//! Nexus CLI — Command-line interface for the Nexus workflow engine.
//!
//! Usage: nexus run <workflow.json> [OPTIONS]
//!
//! This binary needs unsafe code for Windows console API (SetConsoleMode).
#![cfg_attr(windows, allow(unsafe_code))]

use std::fmt::{self, Write};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use nexus_engine::diagnostics::snapshot::EngineSnapshot;
use nexus_engine::graph::validate;
use nexus_engine::model::{EngineConfig, WorkflowDef};
use nexus_engine::runtime::{Engine, NodeEvent, RuntimeError};
use tracing::field::Field;
use tracing::Event;
use tracing_subscriber::fmt::format::Writer as FmtWriter;
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::fmt::FormatEvent;
use tracing_subscriber::fmt::FormatFields;
use tracing_subscriber::registry::LookupSpan;

struct ChunkFormatter;

impl<S, N> FormatEvent<S, N> for ChunkFormatter
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: FmtWriter<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let meta = event.metadata();
        write!(writer, "{} {:5} {}:", "TIMESTAMP", meta.level(), meta.target())?;

        if meta.target() == "nexus::node::chunk" {
            let mut node_id = String::new();
            let mut chunk_text = String::new();
            event.record(&mut |field: &Field, value: &dyn fmt::Debug| {
                if field.name() == "node_id" { node_id = format!("{value:?}"); }
                else if field.name() == "text" { chunk_text = format!("{value:?}"); }
            });
            let node_id = node_id.trim_matches('"');
            let chunk_text = chunk_text.trim_matches('"');
            let summary = summarize_chunk(chunk_text);
            write!(writer, " node_id={node_id} text={summary}")?;
        } else {
            write!(writer, " ")?;
            event.record(&mut |field: &Field, value: &dyn fmt::Debug| {
                let _ = write!(writer, " {}={value:?}", field.name());
            });
        }
        writeln!(writer)
    }
}

fn summarize_chunk(text: &str) -> String {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(text) {
        let kind = val.get("type").and_then(|t| t.as_str()).unwrap_or("data");
        match kind {
            "step_start" => "step_start".to_string(),
            "step_finish" => {
                let mut s = "step_finish".to_string();
                if let Some(reason) = val.get("part").and_then(|p| p.get("reason")).and_then(|r| r.as_str()) {
                    let _ = write!(s, " ({reason})");
                }
                if let Some(tokens) = val.get("part").and_then(|p| p.get("tokens")).and_then(|t| t.get("total")).and_then(|t| t.as_u64()) {
                    let _ = write!(s, " {tokens} tokens");
                }
                s
            }
            "text" => val
                .get("part").and_then(|p| p.get("text")).and_then(|t| t.as_str())
                .map(|t| { let max = 200; if t.len() > max { format!("text: {}...", &t[..max]) } else { format!("text: {t}") } })
                .unwrap_or_else(|| "text".to_string()),
            "tool_use" => {
                let tool = val.get("part").and_then(|p| p.get("tool")).and_then(|t| t.as_str()).unwrap_or("unknown");
                let status = val.get("part").and_then(|p| p.get("state")).and_then(|s| s.get("status")).and_then(|s| s.as_str()).unwrap_or("");
                if status.is_empty() { format!("tool: {tool}") } else { format!("tool: {tool} ({status})") }
            }
            other => { let raw = format!("event: {other}"); let max = 120; if raw.len() > max { format!("{}...", &raw[..max]) } else { raw } }
        }
    } else {
        let max = 200; let t = text.trim();
        if t.len() > max { format!("{}...", &t[..max]) } else { t.to_string() }
    }
}

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
                .event_format(ChunkFormatter)
                .with_env_filter(level)
                .with_writer(make_writer)
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

            // Create the event callback that logs events to stderr in real time.
            let event_cb: nexus_engine::runtime::NodeEventCb = Arc::new(move |event| {
                use std::io::Write;
                match event {
                    NodeEvent::NodeRunning { node_id, .. } => {
                        let _ = writeln!(std::io::stderr(), "[>] {node_id}: running...");
                    }
                    NodeEvent::NodeCompleted { node_id } => {
                        let _ = writeln!(std::io::stderr(), "[v] {node_id}: completed");
                    }
                    NodeEvent::NodeFailed { node_id } => {
                        let _ = writeln!(std::io::stderr(), "[x] {node_id}: failed");
                    }
                    NodeEvent::NodeTimedOut { node_id } => {
                        let _ = writeln!(std::io::stderr(), "[x] {node_id}: timed out");
                    }
                    NodeEvent::NodeChunk { node_id, text } => {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() && trimmed.len() < 120 {
                            let _ = writeln!(std::io::stderr(), "[ ] {node_id}: {trimmed}");
                        }
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
