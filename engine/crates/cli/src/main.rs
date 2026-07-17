//! Nexus CLI — Command-line interface for the Nexus workflow engine.
//!
//! Usage: nexus run <workflow.json> [OPTIONS]
//!
//! This binary needs unsafe code for Windows console API (`SetConsoleMode`).
#![cfg_attr(windows, allow(unsafe_code))]

// dev-dependency anchor for unused_crate_dependencies lint.
#[cfg(test)]
#[doc(hidden)]
pub use tempfile as _tempfile_anchor; // tempfile used in tests

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
                if winapi::um::consoleapi::GetConsoleMode(handle, &raw mut mode) != 0 {
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
                    eprintln!("Error parsing workflow JSON: {e}");
                    std::process::exit(1);
                }
            };

            // Validate
            if let Err(errors) = validate(&def) {
                for err in &errors {
                    eprintln!("Validation error: {err}");
                }
                std::process::exit(1);
            }

            // Advisory warnings — non-blocking, but flag common pitfalls.
            let warnings = nexus_engine::graph::validate_warnings(&def);
            for w in &warnings {
                eprintln!("Warning: {w}");
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

            // ── HITL setup ──
            let hitl_pending: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
            // Print header
            let _ = writeln!(std::io::stderr());
            for id in def.nodes.iter().map(|n| &n.id) {
                let _ = writeln!(std::io::stderr(), "[.] {id}: Pending");
            }
            let _ = writeln!(std::io::stderr(), "{}", "─".repeat(54));

            // Create event callback: lifecycle events only, chunks suppressed except HITL
            let hq2 = hitl_pending.clone();
            let event_cb: nexus_engine::runtime::NodeEventCb = Arc::new(move |event| {
                match event {
                    NodeEvent::Lifecycle(lifecycle) => {
                        let (symbol, nid, msg) = match &lifecycle {
                            NodeLifecycleEvent::Running { node_id, .. } => (">", node_id.clone(), "Running".to_string()),
                            NodeLifecycleEvent::Completed { node_id, output_size } => ("v", node_id.clone(), format!("Completed ({output_size} bytes)")),
                            NodeLifecycleEvent::Failed { node_id, exit_reason, .. } => ("x", node_id.clone(), format!("Failed ({exit_reason})")),
                            NodeLifecycleEvent::TimedOut { node_id, timeout_secs } => ("x", node_id.clone(), format!("TimedOut ({timeout_secs}s)")),
                            _ => return,
                        };
                        let _ = writeln!(std::io::stderr(), "[{symbol}] {nid}: {msg}");
                    },
                    NodeEvent::NodeChunk { ref node_id, ref text } => {
                        let t = text.trim();
                        if t.starts_with("[HUMAN_QUESTION]") {
                            if let Ok(p) = serde_json::from_str::<serde_json::Value>(&text[16..]) {
                                let qid = p["id"].as_str().unwrap_or("").to_string();
                                if let Ok(mut g) = hq2.lock() { g.push((qid, node_id.clone())); }
                            }
                        } else if t.starts_with("[HUMAN_ANSWERED]") || t.starts_with("[HUMAN_TIMEOUT]") {
                            // queue handled by stdin thread
                        }
                        // Non-HITL chunks: suppressed from terminal (go to log file)
                    }
                    // Legacy flat variants — some code paths emit these
                    NodeEvent::NodeRunning { ref node_id, .. } => {
                        let _ = writeln!(std::io::stderr(), "[>] {node_id}: Running");
                    }
                    NodeEvent::NodeCompleted { ref node_id } => {
                        let _ = writeln!(std::io::stderr(), "[v] {node_id}: Completed");
                    }
                    NodeEvent::NodeFailed { ref node_id } => {
                        let _ = writeln!(std::io::stderr(), "[x] {node_id}: Failed");
                    }
                    NodeEvent::NodeTimedOut { ref node_id } => {
                        let _ = writeln!(std::io::stderr(), "[x] {node_id}: TimedOut");
                    }
                    _ => {}
                }
            });

            // ── HITL: stdin reader thread → HTTP pool ──
            let hq = hitl_pending.clone();
            let pp = std::env::var("NEXUS_HUMAN_PORT").unwrap_or_else(|_| "19876".into()).parse::<u16>().unwrap_or(19876);
            std::thread::spawn(move || loop {
                // Wait for questions, then display them one at a time
                let qid = loop {
                    if let Ok(mut g) = hq.lock() {
                        if !g.is_empty() {
                            break Some(g.remove(0));
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(200));
                };
                let Some((qid, node_id)) = qid else { continue };

                // Fetch question details from pool and display
                if let Ok(mut s) = std::net::TcpStream::connect(format!("127.0.0.1:{pp}")) {
                    use std::io::{Read, Write as _};
                    let req = format!("GET /pending HTTP/1.1\r\nHost: 127.0.0.1:{pp}\r\nConnection: close\r\n\r\n");
                    let _ = s.write_all(req.as_bytes());
                    let _ = s.shutdown(std::net::Shutdown::Write);
                    let mut resp = Vec::new();
                    let _ = s.read_to_end(&mut resp);
                    let body = String::from_utf8_lossy(&resp);
                    if let Some(js) = body.find("\r\n\r\n") {
                        if let Ok(pending) = serde_json::from_str::<serde_json::Value>(&body[js+4..]) {
                            if let Some(qs) = pending["questions"].as_array() {
                                for q in qs {
                                    if q["qid"].as_str() == Some(&qid) {
                                        let _ = writeln!(std::io::stderr());
                                        let _ = writeln!(std::io::stderr(), "╔══════════════════════════════════════════════╗");
                                        let _ = writeln!(std::io::stderr(), "║  {node_id} needs your input");
                                        if let Some(c) = q["context"].as_str() { if !c.is_empty() { let _ = writeln!(std::io::stderr(), "║  Context: {c}"); } }
                                        let _ = writeln!(std::io::stderr(), "║  Q: {}", q["question"].as_str().unwrap_or(""));
                                        if let Some(opts) = q["options"].as_array() {
                                            for (i, o) in opts.iter().enumerate() {
                                                let _ = writeln!(std::io::stderr(), "║    [{i}] {o}", i=i, o=o.as_str().unwrap_or(""));
                                            }
                                        }
                                        let _ = writeln!(std::io::stderr(), "╚══════════════════════════════════════════════╝");
                                        let _ = writeln!(std::io::stderr(), "→ Type answer and press Enter: ");
                                        let _ = std::io::stderr().flush();
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }

                // Read answer from stdin
                let mut buf = String::new();
                if std::io::stdin().read_line(&mut buf).is_err() { break; }
                let ans = buf.trim().to_string();
                if ans.is_empty() { continue; }

                // Post answer to pool
                let body = format!("{{\"answer\":\"{}\"}}", ans.replace('"', "\\\""));
                let req = format!("POST /a/{qid} HTTP/1.1\r\nHost: 127.0.0.1:{pp}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                if let Ok(mut s) = std::net::TcpStream::connect(format!("127.0.0.1:{pp}")) {
                    use std::io::{Read, Write as _};
                    let _ = s.write_all(req.as_bytes());
                    let _ = s.shutdown(std::net::Shutdown::Write);
                    let _ = writeln!(std::io::stderr(), "  Answer sent!");
                    let mut r = Vec::new();
                    let _ = s.read_to_end(&mut r);
                }
            });

            // Create and run engine
            let mut engine = match Engine::new(def, config, Some(event_cb)) {
                Ok(e) => e,
                Err(errors) => {
                    for err in &errors {
                        eprintln!("Build error: {err}");
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
                    eprintln!("Runtime error: {e:?}");
                    std::process::exit(2);
                }
            }
        }
    }
}
