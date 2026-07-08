//! Trace ID propagation for workflow execution diagnostics.
//!
//! A [`TraceId`] is assigned at workflow start and propagates through
//! every event emitted during execution, enabling correlation of
//! distributed log entries.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TRACE_ID: AtomicU64 = AtomicU64::new(1);

/// A monotonically increasing identifier scoped to a single workflow run.
///
/// Trace IDs start at 1 and wrap on overflow (2⁶⁴ runs before that matters).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TraceId(u64);

impl TraceId {
    /// Generate the next globally unique trace ID.
    #[must_use]
    pub fn generate() -> Self {
        Self(NEXT_TRACE_ID.fetch_add(1, Ordering::Relaxed))
    }
}

impl fmt::Display for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "trace-{:016x}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_id_generates_unique_ids() {
        let a = TraceId::generate();
        let b = TraceId::generate();
        assert_ne!(a, b);
    }

    #[test]
    fn test_trace_id_display() {
        let id = TraceId(42);
        let s = id.to_string();
        assert!(s.starts_with("trace-"));
        assert_eq!(s.len(), 22); // "trace-" + 16 hex chars
    }
}
