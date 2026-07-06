# 🟢 Documentation Corrections for Outdated Review Findings

> These are findings in `docs/review/` (ARCHITECTURE_REVIEW.md and DEEP_REVIEW.md)
> that have been resolved in code but are still documented as open issues.
> Updating these prevents confusion for new contributors reading the review docs.

---

## R1: DEEP_REVIEW.md — Problem C (`timeout_secs` rename) should be closed

**File**: `docs/review/DEEP_REVIEW.md:190-210`

**Current text**: Marked as **"未解决"** (unsolved)

**Reality**: Both `NodeData` and `NodeParams` use `process_timeout_secs` in the actual code (`graph_def.rs:25,36`). The field was renamed correctly. Also the JSON examples in ARCHITECTURE.md all use `process_timeout_secs`.

**Fix**: Change status from "未解决" to "✅ 已解决" — the code already has the correct field names.

---

## R2: DEEP_REVIEW.md — Problem D (InputSource validation) now implemented

**File**: `docs/review/DEEP_REVIEW.md:213-249`

**Current text**: Marked as **"部分解决"** — Issue 005 lists InputSourceNotFound but not verified implemented.

**Reality**: Both `InputSourceNotFound` and `InputSourceUnreachable` are fully implemented:
- `error.rs:49-63`: Both error variants defined with `Display` impl
- `validator.rs:148-189`: Both checks implemented in `validate()`
- `validator.rs:500-547`: Tests exist for both

**Fix**: Change status from "部分解决" to "✅ 已解决". And update ARCHITECTURE.md §3.3 validation table to include these two items.

---

## R3: DEEP_REVIEW.md — Problem B (retry_count / event_count overlap) partially outdated

**File**: `docs/review/DEEP_REVIEW.md:125-189`

**Current text**: Raises concern about Failed/Timeout being double-counted — counted once in retry and once in handle_event.

**Reality**: The analysis correctly notes they don't overlap (retry handles Failed/Timeout before handle_event). The suggestion to document this is still valid. But the specific code references may be outdated as the implementation has evolved.

---

## R4: ARCHITECTURE_REVIEW.md — Issue #5 (extensions type erasure) partially resolved

**File**: `docs/review/ARCHITECTURE_REVIEW.md:397-438`

**Current text**: Recommends changing `extensions` to `HashMap<String, String>` (from `HashMap<String, serde_json::Value>`).

**Reality**: The code already uses `HashMap<String, String>` in `types.rs:12`:
```rust
pub extensions: HashMap<String, String>,
```

**Fix**: Add a note that Option A (full stringification) was adopted.

---

## R5: ARCHITECTURE_REVIEW.md — Issue #9 (Cargo.toml lint/profile) now fully configured

**File**: `docs/review/ARCHITECTURE_REVIEW.md:593-662`

**Current text**: Recommends adding `[workspace.lints]`, `[profile.release]`, removing `num_cpus`.

**Reality**: All of these are already in place:
- `Cargo.toml:16-28`: Full `[workspace.lints.rust]` and `[workspace.lints.clippy]` with `deny(unsafe_code)`, `deny(missing_docs)`, `deny(panic)`, etc.
- `Cargo.toml:30-33`: `[profile.release]` with `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip = "symbols"`
- No `num_cpus` dependency exists (CLI uses `std::thread::available_parallelism()` in `config.rs:63`)

**Fix**: Mark Issue #9 as fully resolved.

---

## R6: ARCHITECTURE_REVIEW.md — Issue #10 (impl dyn NodeShell) resolved with enum dispatch

**File**: `docs/review/ARCHITECTURE_REVIEW.md:667-751`

**Current text**: Recommends `enum NodeExecutor` as option A.

**Reality**: Exactly this pattern is implemented in `nodeshell/mod.rs:21-45`:
```rust
pub enum NodeExecutor {
    Subprocess(SubprocessExecutor),
    Http(()),
}
impl NodeExecutor {
    pub async fn run(&self, ctx: NodeContext, timeout: Duration) -> Result<NodeOutcome, SpawnError> {
        match self {
            NodeExecutor::Subprocess(exe) => exe.run(ctx, timeout).await,
            NodeExecutor::Http(_) => Err(SpawnError { message: "HTTP executor not implemented" }),
        }
    }
}
```

**Fix**: Mark Issue #10 as fully resolved.

---

## R7: ARCHITECTURE_REVIEW.md — Issue #3 (Edge trait state separation) resolved

**File**: `docs/review/ARCHITECTURE_REVIEW.md:221-313`

**Current text**: Recommends moving edge mutable state out of `Edge` trait into `EdgeState` struct managed by Scheduler.

**Reality**: This is exactly what's implemented:
- `edge.rs:33-47`: `EdgeDef` — pure data, no trait, no mutable state
- `edge.rs:54-61`: `EdgeState` — separate struct with `triggered`, `event_count`, `received`
- `scheduler.rs:214-216`: Scheduler manages `EdgeState` array alongside `EdgeDef` array

**Fix**: Mark Issue #3 as fully resolved.

---

## R8: ARCHITECTURE_REVIEW.md — Issue #4 (BuildResult invariants) resolved as `GraphDef`

**File**: `docs/review/ARCHITECTURE_REVIEW.md:317-391`

**Current text**: Recommends `BuildResult` → typed `GraphDef` with invariants.

**Reality**: `GraphDef::from_components()` exists with 5 invariant checks (`graph_def.rs:81-170`).

**Fix**: Mark Issue #4 as fully resolved.
