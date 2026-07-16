use std::fmt;

/// Errors returned during workflow validation.
///
/// These are detected before runtime, during the validation phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// No entry node found (all nodes have predecessors).
    NoEntryNode,

    /// A node is not reachable from any entry node.
    UnreachableNode {
        /// The unreachable node's ID.
        node_id: String,
    },

    /// A node cannot reach any exit node (node with no outgoing edges).
    ExitNotReachable {
        /// The node's ID.
        node_id: String,
    },

    /// A cycle exists that has no entry point (deadlock).
    CycleWithoutEntry,

    /// The workflow definition has zero nodes.
    EmptyGraph,

    /// A node has no valid provider configured.
    NoValidProvider {
        /// The node's ID.
        node_id: String,
    },

    /// Duplicate node ID found.
    DuplicateNodeId {
        /// The duplicated ID.
        node_id: String,
    },

    /// A predecessor references a node that does not exist.
    InvalidPredecessor {
        /// The node that has the invalid predecessor.
        node_id: String,
        /// The referenced (non-existent) predecessor node ID.
        predecessor_id: String,
    },

    /// An input references a node that does not exist.
    InputSourceNotFound {
        /// The node that declared the input.
        node_id: String,
        /// The referenced (non-existent) input source node ID.
        source_id: String,
    },

    /// An input source is not reachable from any entry node.
    InputSourceUnreachable {
        /// The node that declared the input.
        node_id: String,
        /// The input source node ID that is unreachable.
        source_id: String,
    },

    /// A node references `{{datarouter.X.*}}` but no dataflow
    /// `from: X, to: this_node` exists.
    DatarouterRefWithoutDataflow {
        /// The node that references the datarouter field.
        node_id: String,
        /// The referenced source node ID.
        source_id: String,
    },

    /// A node uses `{{metadata.<FIELD>}}` with an unrecognized field name.
    UnknownMetadataField {
        /// The node that uses the unknown metadata field.
        node_id: String,
        /// The unrecognized field name.
        field: String,
    },

    /// A node uses `{{datarouter.<SRC>.<FIELD>}}` with an unrecognized field.
    UnknownDatarouterField {
        /// The node that uses the unknown field.
        node_id: String,
        /// The upstream source node.
        source_id: String,
        /// The unrecognized field name.
        field: String,
    },

    /// A template placeholder `{{...}}` uses an unrecognized prefix
    /// (not `inputs`, `metadata`, `datarouter`, or `prompt`).
    UnrecognizedTemplate {
        /// The node that uses the unrecognized template.
        node_id: String,
        /// The full template content inside `{{...}}`.
        template: String,
    },

    /// Graph invariant violation during construction.
    BuildInvariant {
        /// Human-readable description of which invariant failed.
        description: String,
    },

    /// A node has `process_timeout_secs` set to 0 (would timeout immediately).
    ZeroTimeout {
        /// The node's ID.
        node_id: String,
    },

    /// An `llm` node's `scripts_dir` does not contain `llm_node.py`.
    LlmNodeMissingWrapper {
        /// The node's ID.
        node_id: String,
        /// The resolved `scripts_dir` that was checked.
        scripts_dir: String,
    },

    /// An `llm` node uses `-p "{{prompt}}"` in its command, which will
    /// break on Windows when the rendered prompt contains newlines.
    LlmPromptInCommandLine {
        /// The node's ID.
        node_id: String,
    },

    /// An `llm` node uses `--output-format json` instead of
    /// `stream-json` — no real-time chunks in raw log.
    LlmNoStreamingOutput {
        /// The node's ID.
        node_id: String,
    },

    /// A `scripts_dir` path may resolve incorrectly at runtime due to
    /// redundant path segments (e.g. `release/release/`).
    SuspiciousScriptsDir {
        /// The node's ID (or workflow ID for workflow-level `scripts_dir`).
        node_id: String,
        /// The `scripts_dir` value configured.
        scripts_dir: String,
    },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoEntryNode => {
                write!(f, "no entry node: all nodes have predecessors")
            }
            Self::UnreachableNode { node_id } => {
                write!(
                    f,
                    "unreachable node '{node_id}': not reachable from any entry node"
                )
            }
            Self::ExitNotReachable { node_id } => {
                write!(f, "exit not reachable from node '{node_id}'")
            }
            Self::CycleWithoutEntry => {
                write!(f, "cycle without entry: deadlock detected")
            }
            Self::EmptyGraph => {
                write!(f, "empty graph: no nodes defined")
            }
            Self::NoValidProvider { node_id } => {
                write!(f, "node '{node_id}' has no valid provider")
            }
            Self::DuplicateNodeId { node_id } => {
                write!(f, "duplicate node ID: '{node_id}'")
            }
            Self::InvalidPredecessor {
                node_id,
                predecessor_id,
            } => {
                write!(
                    f,
                    "node '{node_id}' references non-existent predecessor '{predecessor_id}'"
                )
            }
            Self::InputSourceNotFound { node_id, source_id } => {
                write!(
                    f,
                    "node '{node_id}' references non-existent input source '{source_id}'"
                )
            }
            Self::InputSourceUnreachable { node_id, source_id } => {
                write!(
                    f,
                    "input source '{source_id}' for node '{node_id}' is not reachable from any entry"
                )
            }
            Self::BuildInvariant { description } => {
                write!(f, "build invariant failure: {description}")
            }
            Self::ZeroTimeout { node_id } => {
                write!(f, "node '{node_id}' has process_timeout_secs = 0 (would timeout immediately)")
            }
            Self::DatarouterRefWithoutDataflow { node_id, source_id } => {
                write!(
                    f,
                    "node '{node_id}' uses {{{{datarouter.{source_id}.xxx}}}} but no dataflow from '{source_id}' to '{node_id}' exists",
                )
            }
            Self::UnknownMetadataField { node_id, field } => {
                write!(
                    f,
                    "node '{node_id}' uses {{{{metadata.{field}}}}} — unknown metadata field (valid: run_count, timed_out)",
                )
            }
            Self::UnknownDatarouterField { node_id, source_id, field } => {
                write!(
                    f,
                    "node '{node_id}' uses {{{{datarouter.{source_id}.{field}}}}} — unknown field (valid: route, content)",
                )
            }
            Self::UnrecognizedTemplate { node_id, template } => {
                write!(
                    f,
                    "node '{node_id}' has unrecognized template placeholder '{template}' — engine supports metadata.*, datarouter.*.*",
                )
            }
            Self::LlmNodeMissingWrapper {
                node_id,
                scripts_dir,
            } => {
                write!(
                    f,
                    "node '{node_id}' (type=llm) requires llm_node.py in scripts_dir '{scripts_dir}', but the file was not found. \
                    Set node-level scripts_dir to the directory containing llm_node.py",
                )
            }
            Self::LlmPromptInCommandLine { node_id } => {
                write!(
                    f,
                    "node '{node_id}' (type=llm) uses -p \"{{{{prompt}}}}\" in its command — this will break on Windows \
                    when the prompt contains newlines. Remove -p \"{{{{prompt}}}}\" from the command; \
                    llm_node.py will pass the prompt via stdin instead",
                )
            }
            Self::LlmNoStreamingOutput { node_id } => {
                write!(
                    f,
                    "node '{node_id}' (type=llm) uses --output-format json (no streaming). \
                    Use --output-format stream-json for real-time chunks in the raw log",
                )
            }
            Self::SuspiciousScriptsDir {
                node_id,
                scripts_dir,
            } => {
                write!(
                    f,
                    "node '{node_id}' has suspicious scripts_dir '{scripts_dir}' — it may resolve to a non-existent path \
                    at runtime due to redundant segments or incorrect CWD assumptions",
                )
            }
        }
    }
}

impl std::error::Error for ValidationError {}

// ── Validation Warning ─────────────────────────────────────────────

/// Warnings returned during workflow validation.
///
/// Unlike [`ValidationError`], warnings do NOT block execution.
/// They flag common pitfalls that may cause runtime issues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationWarning {
    /// An `llm` node uses `-p "{{prompt}}"` with a multi-line prompt
    /// that will break on Windows when the newlines hit `cmd.exe`.
    LlmPromptInCommandLine {
        /// The node's ID.
        node_id: String,
    },
    /// An `llm` node uses `--output-format json` instead of `stream-json`
    /// — no real-time chunks will appear in the raw log.
    LlmNoStreamingOutput {
        /// The node's ID.
        node_id: String,
    },
    /// A `scripts_dir` resolves to a path with redundant segments
    /// (e.g. `release/release/` when CWD is already `release/`).
    SuspiciousScriptsDir {
        /// The node or workflow identifier.
        node_id: String,
        /// The configured `scripts_dir` value.
        scripts_dir: String,
        /// The resolved path showing the redundancy.
        resolved: String,
    },
}

impl fmt::Display for ValidationWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LlmPromptInCommandLine { node_id } => {
                write!(
                    f,
                    "node '{node_id}' uses -p \"{{{{prompt}}}}\" with a multi-line prompt — \
                    this will break on Windows. Remove -p \"{{{{prompt}}}}\" from the command; \
                    llm_node.py passes the prompt via stdin instead",
                )
            }
            Self::LlmNoStreamingOutput { node_id } => {
                write!(
                    f,
                    "node '{node_id}' uses --output-format json (no streaming). \
                    Use --output-format stream-json for real-time chunks in the raw log",
                )
            }
            Self::SuspiciousScriptsDir { node_id, scripts_dir, resolved } => {
                write!(
                    f,
                    "node '{node_id}' scripts_dir '{scripts_dir}' resolves to '{resolved}' — redundant path segment detected",
                )
            }
        }
    }
}
