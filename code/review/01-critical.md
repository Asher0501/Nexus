# 🔴 Critical Issues

> Must fix before production. These affect correctness or could cause silent data loss.

---

## C1: `NodeCounters.execution_count` — dead code / semantic mismatch

**File**: `nexus-engine/src/graph/scheduler.rs:72-78`

> **处理方案**: ✅ 已修复。`NodeCounters` 改为 `{complete, failed, timeout}: u64` per-event-type 计数器，`execution_count` 和 `event_count` 已删除。Option A 方案，匹配 ARCHITECTURE.md §4.6 文档。

```rust
#[derive(Debug, Clone, Default)]
pub struct NodeCounters {
    /// Number of times the node has been executed.
    pub execution_count: u64,  // ← NEVER INCREMENTED — always 0
    /// Number of events emitted by this node.
    pub event_count: u64,      // ← The only field that's actually modified (line 202)
}
```

### What's wrong

`execution_count` is declared but **never written to** anywhere in the entire codebase. A grep for `execution_count` returns only:
- Its declaration in `scheduler.rs:75`
- Default initialization in `Scheduler::new()` (scheduler.rs:126) — sets it to 0

Meanwhile, `event_count` is incremented in `handle_event()` (scheduler.rs:202) but is a **unified** counter — it doesn't distinguish between Complete, Failed, and Timeout events.

### Why it matters

1. **Silent correctness issue**: If any future code reads `execution_count` expecting a real value, it gets `0` always. This is a trap.
2. **Documentation mismatch**: `ARCHITECTURE.md §4.6` describes per-event-type counters `{ complete, failed, timeout }: u64`. Code has `{ execution_count, event_count }: u64` — a different data model.
3. **Monitoring blind spot**: No way to query "how many times did node X fail?" or "did node X timeout?" from the scheduler state.

### Fix options

**Option A (match architecture docs):** Replace `NodeCounters` with per-event-type counters:

```rust
#[derive(Debug, Clone, Default)]
pub struct NodeCounters {
    pub complete: u64,
    pub failed: u64,
    pub timeout: u64,
}
```

Increment the appropriate field in `handle_event()` based on `event` parameter.

**Option B (simpler — remove dead code):** Remove `execution_count`, keep `event_count`:

```rust
#[derive(Debug, Clone, Default)]
pub struct NodeCounters {
    pub event_count: u64,
}
```

Then update `ARCHITECTURE.md §4.6` to match the unified counter model.

### Impact assessment

- **Likelihood**: Low (no existing code reads `execution_count`), but adding code that does is a natural mistake.
- **Consequence if triggered**: Silent monitoring gap or incorrect retry logic assumptions.
- **Effort to fix**: ~15 minutes for either option.

### Acceptance criteria

- [ ] `NodeCounters` is either per-event-type (matching architecture docs) or unified with no dead fields
- [ ] ARCHITECTURE.md §4.6 is updated to match the chosen model
- [ ] Tests verify counters are correctly updated in handle_event
- [ ] No `execution_count` or other dead fields remain in the codebase
