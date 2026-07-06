use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// ANSI escape: cursor up N lines.
const CSI_UP: &[u8] = b"\x1b[A";
/// ANSI escape: erase entire line.
const CSI_ERASE: &[u8] = b"\x1b[K";
/// ANSI escape: carriage return.
const CR: &[u8] = b"\r";
/// ANSI escape: foreground bright blue.
const CSI_BLUE: &[u8] = b"\x1b[94m";
/// ANSI escape: foreground bright green.
const CSI_GREEN: &[u8] = b"\x1b[92m";
/// ANSI escape: foreground bright red.
const CSI_RED: &[u8] = b"\x1b[91m";
/// ANSI escape: reset.
const CSI_RESET: &[u8] = b"\x1b[0m";
/// Newline.
const NL: &[u8] = b"\n";

/// Tracks the live status of each concurrency slot and renders a fixed-height
/// status board to stderr using ANSI escape sequences.
///
/// Layout (with max_concurrency=5):
///
///   [0] ▶ config: running cmd /c echo hello
///   [1] ✓ review: completed
///   [2] idle
///   [3] idle
///   [4] idle
///
/// Each event redraws the board in place using `\r` and cursor-up escapes.
pub struct StatusBoard {
    /// Number of concurrency slots (lines to display).
    _max_concurrency: usize,
    /// Per-slot state.
    slots: Vec<Mutex<SlotState>>,
    /// Total slots ever shown (for cursor-up calculation).
    drawn_lines: AtomicUsize,
}

#[derive(Clone)]
struct SlotState {
    node_id: String,
    status: SlotStatus,
    detail: String,
}

#[derive(Clone, PartialEq)]
enum SlotStatus {
    Idle,
    Running,
    Completed,
    Failed,
}

impl StatusBoard {
    /// Create a new status board with the given number of slots.
    #[must_use]
    pub fn new(max_concurrency: usize) -> Self {
        let slots = (0..max_concurrency)
            .map(|_| {
                Mutex::new(SlotState {
                    node_id: String::new(),
                    status: SlotStatus::Idle,
                    detail: String::new(),
                })
            })
            .collect();

        Self {
            _max_concurrency: max_concurrency,
            slots,
            drawn_lines: AtomicUsize::new(0),
        }
    }

    /// Assign a running node to the first idle slot.
    pub fn start(&self, node_id: &str, _command: &str) {
        let nid = node_id.to_string();

        for slot in &self.slots {
            let mut state = slot.lock().unwrap();
            if state.status == SlotStatus::Idle {
                state.node_id = nid;
                state.status = SlotStatus::Running;
                state.detail = String::new();
                return;
            }
        }
    }

    /// Update the detail text for a running node (streaming chunk).
    pub fn chunk(&self, node_id: &str, text: &str) {
        for slot in &self.slots {
            let mut state = slot.lock().unwrap();
            if state.node_id == node_id && state.status == SlotStatus::Running {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    state.detail = trimmed.to_string();
                    self.render();
                }
                return;
            }
        }
    }

    /// Mark a running node as completed.
    pub fn complete(&self, node_id: &str) {
        for slot in &self.slots {
            let mut state = slot.lock().unwrap();
            if state.node_id == node_id {
                state.status = SlotStatus::Completed;
                self.render();
                return;
            }
        }
    }

    /// Mark a running node as failed.
    pub fn fail(&self, node_id: &str) {
        for slot in &self.slots {
            let mut state = slot.lock().unwrap();
            if state.node_id == node_id {
                state.status = SlotStatus::Failed;
                self.render();
                return;
            }
        }
    }

    /// Render the entire board to stderr.
    pub fn render(&self) {
        let drawn = self.drawn_lines.load(Ordering::Relaxed);
        let mut out: Vec<u8> = Vec::new();

        // Move cursor up to overwrite previous render.
        if drawn > 0 {
            for _ in 0..drawn {
                out.extend_from_slice(CSI_UP);
            }
        }

        let mut lines_this_frame = 0;
        for (i, slot) in self.slots.iter().enumerate() {
            let state = slot.lock().unwrap();
            match state.status {
                SlotStatus::Idle => {
                    out.extend_from_slice(CR);
                    out.extend_from_slice(CSI_ERASE);
                    write!(out, "[{i}] idle").ok();
                    out.extend_from_slice(NL);
                }
                SlotStatus::Running => {
                    out.extend_from_slice(CR);
                    out.extend_from_slice(CSI_ERASE);
                    write!(out, "[{i}] ").ok();
                    out.extend_from_slice(CSI_BLUE);
                    out.extend_from_slice(b">");
                    out.extend_from_slice(CSI_RESET);
                    write!(out, " {}: {}", state.node_id, state.detail).ok();
                    out.extend_from_slice(NL);
                }
                SlotStatus::Completed => {
                    out.extend_from_slice(CR);
                    out.extend_from_slice(CSI_ERASE);
                    write!(out, "[{i}] ").ok();
                    out.extend_from_slice(CSI_GREEN);
                    out.extend_from_slice(b"v");
                    out.extend_from_slice(CSI_RESET);
                    write!(out, " {}: completed", state.node_id).ok();
                    out.extend_from_slice(NL);
                }
                SlotStatus::Failed => {
                    out.extend_from_slice(CR);
                    out.extend_from_slice(CSI_ERASE);
                    write!(out, "[{i}] ").ok();
                    out.extend_from_slice(CSI_RED);
                    out.extend_from_slice(b"x");
                    out.extend_from_slice(CSI_RESET);
                    write!(out, " {}: failed", state.node_id).ok();
                    out.extend_from_slice(NL);
                }
            }
            lines_this_frame += 1;
        }

        self.drawn_lines.store(lines_this_frame, Ordering::Relaxed);
        let _ = std::io::stderr().write_all(&out);
        let _ = std::io::stderr().flush();
    }

    /// Clear the board from the terminal (for final cleanup).
    pub fn clear(&self) {
        let drawn = self.drawn_lines.load(Ordering::Relaxed);
        if drawn > 0 {
            let mut out: Vec<u8> = Vec::new();
            for _ in 0..drawn {
                out.extend_from_slice(CSI_UP);
            }
            for _ in 0..drawn {
                out.extend_from_slice(CR);
                out.extend_from_slice(CSI_ERASE);
                out.extend_from_slice(NL);
            }
            let _ = std::io::stderr().write_all(&out);
            let _ = std::io::stderr().flush();
        }
    }

    /// Truncate a command string for display.
    fn truncate_command(cmd: &str) -> String {
        // Only show the binary name for long commands.
        if cmd.len() > 40 {
            if let Some(first_space) = cmd.find(' ') {
                let binary = &cmd[..first_space];
                // Extract the last path component as the binary name.
                let name = binary.rsplit(&['/', '\\'][..]).next().unwrap_or(binary);
                return format!("{name} ...");
            }
        }
        cmd.to_string()
    }
}
