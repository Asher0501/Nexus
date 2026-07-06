# 🟡 Documentation-to-Code Synchronization Gaps

> These are discrepancies between ARCHITECTURE.md / NODE_PROTOCOL.md and the actual Rust source.
> Some are harmless (docs describe planned design), others could confuse new contributors.

---

## D1: ARCHITECTURE.md §9 — dependency table lists non-existent crates

**File**: `docs/architecture/ARCHITECTURE.md:738-740`

```markdown
| regex | exit_patterns 正则匹配 |
| reqwest 0.12 | HTTP 客户端（HttpExecutor） |
| async-trait 0.1 | 异步 trait 支持 |
```

All three crates are **not present** in any `Cargo.toml`:
- `regex` — not needed (exit_reason matching uses string comparison, not regex)
- `reqwest` — not used (HttpExecutor is a placeholder with `()` type — subprocess.rs only)
- `async-trait` — not used (enum dispatch `NodeExecutor` replaces `impl dyn NodeShell`)

**Fix**: Remove these rows from the dependency table, or add a note: "planned for HttpExecutor implementation, not yet required."

---

## D2: ARCHITECTURE.md §5.1 — `EngineEvent` documented with `NodeCompleted` variant

**File**: `docs/architecture/ARCHITECTURE.md:358-368`

```rust
enum EngineEvent {
    NodeReady { node_id: NodeIndex },
    NodeCompleted {
        node_id: NodeIndex,
        output: String,
        timed_out: bool,
        exit_code: i32,
        exit_reason: Option<String>,
    },
}
```

Actual code (`engine.rs:14-20`) has only `NodeReady`. `NodeCompleted` events are processed synchronously within `handle_event()` — the executor runs and the completion handling happens inline, not via channel.

**Fix**: Either:
1. Add `NodeCompleted` variant and split event handling (makes architecture match docs but adds complexity)
2. **Update the doc to match current implementation** (recommended — simpler, already correct)

---

## D3: ARCHITECTURE.md §5.2 — event loop pseudocode is synchronous / outdated

**File**: `docs/architecture/ARCHITECTURE.md:373-420`

The pseudocode shows:
- A synchronous loop with `send(NodeReady(node))` as direct function calls
- Inline retry logic in the event loop body
- No `tokio::select!` with global timeout
- No `SpawnError` handling path
- `DataRouter.build_input()` signature uses `node_id: &str` but actual code uses `requested_inputs: &[String]`

**Fix**: Update the pseudocode to match the actual `engine.rs:run()` implementation:
- `tokio::select!` loop with channel events
- `handle_event()` dispatch with spawn error handling
- Retry via `Scheduler::retry_node()` method

---

## D4: ARCHITECTURE.md §3.2 — `NodeData.return_values` mentioned but non-existent

**File**: `docs/architecture/ARCHITECTURE.md:213-219`

```rust
pub struct NodeData {
    pub id: String,
    pub providers: Vec<ProviderDef>,
    pub process_timeout_secs: u64,
    pub max_concurrency: usize,
    pub requested_inputs: Vec<String>,
}
```

Correct for code. But the appendix (line 787-789) says:
> "DESIGN_PHILOSOPHY.md §3: '如果节点不需要分支，returns 配置为空' — NodeDef 中没有 returns: Vec<String> 字段"

This appendix note is **outdated** — `NodeDef.returns` exists in `workflow.rs:38`:
```rust
pub returns: Vec<String>,
```

And `NodeData` correctly does **not** carry `returns` because the scheduler uses `EdgeDef.exit_reason` for matching, not node-level returns. The design decision is correct — the review note in the appendix should be updated.

---

## D5: ARCHITECTURE.md §4.6 — `NodeCounters` documented differently from code

**File**: `docs/architecture/ARCHITECTURE.md:340-345`

```rust
pub struct NodeCounters {
    pub complete: u64,
    pub failed: u64,
    pub timeout: u64,
}
```

Actual code (`scheduler.rs:72-78`):
```rust
pub struct NodeCounters {
    pub execution_count: u64,  // dead code
    pub event_count: u64,      // unified counter
}
```

**Two gaps**:
1. Documentation says per-event-type counters; code has unified counter + dead field
2. The dead field `execution_count` is not documented anywhere

**Fix**: Resolve the critical issue first (C1), then update §4.6 to match the chosen implementation.

---

## D6: ARCHITECTURE.md §5.2 — `counters[node][event]++` doesn't match code

**File**: `docs/architecture/ARCHITECTURE.md:395`

```
counters[node][event]++
```

The pseudocode implies a 2D array indexed by event type. Actual code (`scheduler.rs:201-203`):
```rust
if let Some(counter) = self.state.counters.get_mut(&node) {
    counter.event_count += 1;
}
```

The code uses a unified `event_count`, not per-event-type counters. Along with D5, this section needs synchronization.

---

## D7: NODE_PROTOCOL.md — `NodeContext.inputs` documented as string-to-string but code matches

**File**: `docs/architecture/NODE_PROTOCOL.md:44-47`

This is actually **correctly implemented**. Documentation says:
> `inputs: object` — `key = 来源节点 ID`，`value = 该节点输出的纯文本`

Code (`types.rs:7-13`):
```rust
pub struct NodeContext {
    pub inputs: HashMap<String, String>,
    pub extensions: HashMap<String, String>,
}
```

✅ Matches. No issue here — included for completeness.

---

## D8: ARCHITECTURE.md §3.3 — Validator table missing `InvalidPredecessor`

**File**: `docs/architecture/ARCHITECTURE.md:234-243`

The Validator check table lists 7 items but is missing `InvalidPredecessor` (line 93 of validator test table shows it's implemented in code). The `DEEP_REVIEW.md` Problem D noted `InputSourceNotFound` and `InputSourceUnreachable` were missing — they're now in the code (`error.rs:49-63`), but the table in ARCHITECTURE.md §3.3 hasn't been updated to include them.
