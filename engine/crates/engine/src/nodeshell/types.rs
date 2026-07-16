use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Execution metadata passed to a node alongside its input data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMetadata {
    /// How many times this node has been executed so far (1-based).
    pub run_count: u64,
    /// Whether the previous execution of this node timed out.
    pub timed_out: bool,
}

/// Input context passed to a node from the engine.
#[derive(Debug, Clone, Serialize)]
pub struct NodeContext {
    /// Map of upstream node IDs to their output content strings
    /// (legacy: retained for non-template use; template rendering uses `upstream`).
    pub inputs: HashMap<String, String>,
    /// Extension parameters for the node (unused in v1, reserved).
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub extensions: HashMap<String, String>,
    /// Execution metadata.
    pub metadata: NodeMetadata,
    /// Map of upstream alias → full output (route + content),
    /// for `{{datarouter.<alias>.route}}` and `{{datarouter.<alias>.content}}`.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub upstream: HashMap<String, NodeOutput>,
}

impl Default for NodeContext {
    fn default() -> Self {
        Self {
            inputs: HashMap::new(),
            extensions: HashMap::new(),
            metadata: NodeMetadata {
                run_count: 1,
                timed_out: false,
            },
            upstream: HashMap::new(),
        }
    }
}

impl NodeContext {
    /// Remove lone surrogates (U+D800–U+DFFF) from all string fields.
    ///
    /// These invalid Unicode code points can leak in via Windows console API
    /// transcoding or `serde_json` escape sequences.  They are never
    /// intentional — they are always encoding artefacts — and they break
    /// UTF-8 encoding downstream (Python `json.dumps`, Anthropic SDK, etc.).
    pub fn sanitize_surrogates(&mut self) {
        for (_, v) in &mut self.inputs {
            strip_surrogates(v);
        }
        for (_, v) in &mut self.extensions {
            strip_surrogates(v);
        }
        for (_, v) in &mut self.upstream {
            strip_surrogates(&mut v.route);
            strip_surrogates(&mut v.content);
        }
    }
}

/// Strip lone surrogate code points from a string in place.
fn strip_surrogates(s: &mut String) {
    s.retain(|c| !(0xD800..=0xDFFF).contains(&(c as u32)));
}

/// An output chunk emitted by a node during execution (streaming).
///
/// This is distinct from the final [`NodeOutput`], which is the complete
/// structured result. A node may emit multiple chunks as it runs.
#[derive(Debug, Clone)]
pub struct NodeChunk {
    /// The output text line.
    pub text: String,
}

/// A structured output from a node, carrying both the routing key
/// and the content payload. Emitted as JSON on stdout so the engine
/// can match edges by route and forward content via the `DataRouter`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeOutput {
    /// Logical route key for edge matching.
    pub route: String,
    /// The content payload.
    #[serde(default)]
    pub content: String,
}

/// Exit code sentinel values for the engine.
///
/// - `0`: node exited normally (exit 0)
/// - non-zero positive: node self-reported error (exit N)
/// - `-1`: wait failed or child produced no exit code
/// - `-9`: killed by engine due to timeout
pub mod exit_codes {
    /// Process exited normally with code 0.
    #[allow(dead_code)]
    pub const SUCCESS: i32 = 0;
    /// `wait()` failed or no exit code available.
    pub const WAIT_FAILED: i32 = -1;
    /// Killed by engine due to timeout.
    pub const TIMEOUT: i32 = -9;
}

/// The outcome of executing a node.
#[derive(Debug, Clone)]
pub struct NodeOutcome {
    /// Structured output produced by the node.
    pub output: NodeOutput,
    /// Process exit code:
    ///  0  = success
    ///  -1 = wait failed / no exit code
    ///  -9 = killed by timeout
    ///  N  = node self-reported error
    pub exit_code: i32,
    /// Exit reason extracted from stdout header.
    pub exit_reason: Option<String>,
}

impl NodeOutcome {
    /// Whether the node was killed due to timeout (`exit_code` == -9).
    #[must_use]
    pub const fn timed_out(&self) -> bool {
        self.exit_code == exit_codes::TIMEOUT
    }
}

/// Error returned when a node cannot be spawned.
#[derive(Debug, Clone)]
pub struct SpawnError {
    /// Human-readable error description.
    pub message: String,
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "spawn error: {}", self.message)
    }
}

impl std::error::Error for SpawnError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a string containing a lone surrogate at the given code point.
    /// Lone surrogates have valid UTF-8 encodings (they're just not valid
    /// Unicode *scalar values*), so they can exist in Rust Strings via
    /// byte manipulation or serde_json parsing.
    fn surrogate_str(code: u16) -> String {
        assert!((0xD800..=0xDFFF).contains(&code));
        // UTF-8 encoding of a 3-byte sequence: 0xED 0x?? 0x??
        let c = code as u32;
        let b = [
            0xEDu8,
            (0x80 | ((c >> 6) & 0x3F)) as u8,
            (0x80 | (c & 0x3F)) as u8,
        ];
        String::from_utf8(b.to_vec()).unwrap_or_default()
    }

    #[test]
    fn strip_surrogates_removes_lone_surrogate() {
        let mut s = format!("hello{}world", surrogate_str(0xD800));
        strip_surrogates(&mut s);
        assert_eq!(s, "helloworld");
    }

    #[test]
    fn strip_surrogates_preserves_valid_unicode() {
        // U+00E9 = é, U+4E2D = 中 — both valid, must be kept
        let mut s = String::from("caf\u{00E9}\u{4E2D}");
        let expected = s.clone();
        strip_surrogates(&mut s);
        assert_eq!(s, expected);
    }

    #[test]
    fn strip_surrogates_handles_multiple_surrogates() {
        let mut s = format!("a{}b{}c", surrogate_str(0xDC00), surrogate_str(0xDFFF));
        strip_surrogates(&mut s);
        assert_eq!(s, "abc");
    }

    #[test]
    fn sanitize_node_context_cleans_all_fields() {
        let mut ctx = NodeContext {
            inputs: std::collections::HashMap::from([(
                "k".into(),
                format!("v{}al", surrogate_str(0xD800)),
            )]),
            extensions: std::collections::HashMap::from([(
                "e".into(),
                format!("x{}y", surrogate_str(0xDFFF)),
            )]),
            metadata: NodeMetadata { run_count: 1, timed_out: false },
            upstream: std::collections::HashMap::from([(
                "u".into(),
                NodeOutput {
                    route: format!("r{}", surrogate_str(0xDC00)),
                    content: format!("c{}", surrogate_str(0xD800)),
                },
            )]),
        };
        ctx.sanitize_surrogates();
        assert_eq!(ctx.inputs["k"], "val");
        assert_eq!(ctx.extensions["e"], "xy");
        assert_eq!(ctx.upstream["u"].route, "r");
        assert_eq!(ctx.upstream["u"].content, "c");
    }
}
