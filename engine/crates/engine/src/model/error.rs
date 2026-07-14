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

    /// A node references `{{inputs.X}}` in its prompt or command but no
    /// dataflow `from: X, to: this_node` exists.
    ReferencedInputWithoutDataflow {
        /// The node that references the input.
        node_id: String,
        /// The referenced input source node ID.
        source_id: String,
    },

    /// Graph invariant violation during construction.
    BuildInvariant {
        /// Human-readable description of which invariant failed.
        description: String,
    },

    /// A node has process_timeout_secs set to 0 (would timeout immediately).
    ZeroTimeout {
        /// The node's ID.
        node_id: String,
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
                    "unreachable node '{}': not reachable from any entry node",
                    node_id
                )
            }
            Self::ExitNotReachable { node_id } => {
                write!(f, "exit not reachable from node '{}'", node_id)
            }
            Self::CycleWithoutEntry => {
                write!(f, "cycle without entry: deadlock detected")
            }
            Self::EmptyGraph => {
                write!(f, "empty graph: no nodes defined")
            }
            Self::NoValidProvider { node_id } => {
                write!(f, "node '{}' has no valid provider", node_id)
            }
            Self::DuplicateNodeId { node_id } => {
                write!(f, "duplicate node ID: '{}'", node_id)
            }
            Self::InvalidPredecessor {
                node_id,
                predecessor_id,
            } => {
                write!(
                    f,
                    "node '{}' references non-existent predecessor '{}'",
                    node_id, predecessor_id
                )
            }
            Self::InputSourceNotFound { node_id, source_id } => {
                write!(
                    f,
                    "node '{}' references non-existent input source '{}'",
                    node_id, source_id
                )
            }
            Self::InputSourceUnreachable { node_id, source_id } => {
                write!(
                    f,
                    "input source '{}' for node '{}' is not reachable from any entry",
                    source_id, node_id
                )
            }
            Self::BuildInvariant { description } => {
                write!(f, "build invariant failure: {description}")
            }
            Self::ZeroTimeout { node_id } => {
                write!(f, "node '{}' has process_timeout_secs = 0 (would timeout immediately)", node_id)
            }
            Self::ReferencedInputWithoutDataflow { node_id, source_id } => {
                write!(
                    f,
                    "node '{}' uses {{{{inputs.{}}}}} but no dataflow from '{}' to '{}' exists",
                    node_id, source_id, source_id, node_id,
                )
            }
        }
    }
}

impl std::error::Error for ValidationError {}
