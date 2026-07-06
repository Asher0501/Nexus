# 🔬 Deep Review: Nexus — Third Round

> **Date**: 2026-07-06  
> **Focus**: Verification of previous findings, new diagnostics module, runtime edge cases,  
>              test coverage analysis, before/after delta tracking  
> **Build**: `cargo build --release` ✅ · `cargo test` ✅ 97/97 pass

---

## ⚡ Significant Changes Since Second Round

Several issues identified in previous rounds have been **fixed**:

| Previous Issue | Status | Evidence |
|---|---|---|
| **C1**: `execution_count` dead code | ✅ **FIXED** | `NodeCounters` now has `complete`/`failed`/`timeout` per-event-type fields (scheduler.rs:73-79) |
| **M1**: CLI exit code 3 missing | ✅ **FIXED** | `RuntimeError::IdleTimeout` → `exit(3)` (main.rs:130-132) |
| **M2**: CLI sentinel value `0` | ✅ **FIXED** | `max_concurrency: Option<usize>`, `node_timeout: Option<u64>` (main.rs:32-38) |
| **M5**: stdin not closed | ✅ **FIXED** | `child.stdin.take()` now used, stdin drops on scope exit (subprocess.rs:70-74) |

**Closed** (resolved or decision recorded):
- `EngineEvent` missing `NodeCompleted` variant (M6) → ✅ ARCHITECTURE.md updated
- `Strategy` serde PascalCase vs `TriggerExpr` lowercase (M4) → ✅ Decision: internal type, no fix needed
- Stdout pipe deadlock risk (DR4) → ✅ Decision: L3 system-level, diagnostics coverage
- `is_converged()` channel-blind (DR2) → ✅ Decision: L3 system-level, not fixing

**Still open** (code unchanged):
- CLI tests placeholder only (L1)
- `build.bat` hardcoded path (L3)
- `WorkflowResult` empty struct (L5)

---

## 🆕 New Module: `diagnostics/` — Observability Layer

A new `diagnostics` module was added with three submodules, adding **4 source files** and **10 tests** (from 90→97).

### `diagnostics/event.rs` ✅ Well-designed

Structured `tracing` event types:
- `NodeLifecycleEvent` — Pending/Running/Completed/Failed/TimedOut
- `EngineLifecycleEvent` — Started/Converged/TimedOut
- `SystemEvent` — SpawnSlow/StdinWriteSlow

**Good**: Clean enum hierarchy, proper `tracing::info!`/`warn!` targets with structured fields. Coverage test emits every variant.

**Issues**:
- ⚠️ Events are **declared but never emitted** from the engine. The `emit_lifecycle()`, `emit_engine()`, and `emit_system()` functions exist but are not called anywhere in `engine.rs` or `scheduler.rs`.
- `EngineLifecycleEvent::TimedOut` and `SystemEvent::SpawnSlow`/`StdinWriteSlow` have no production call sites.
- The module comment (mod.rs:8) says "never mutates or locks engine state" — good design intent, but it SHOULD be integrated into the engine.

### `diagnostics/trace.rs` ⚠️ AtomicU64 ordering concern

```rust
static NEXT_TRACE_ID: AtomicU64 = AtomicU64::new(1);

pub fn generate() -> Self {
    Self(NEXT_TRACE_ID.fetch_add(1, Ordering::Relaxed))
}
```

`Ordering::Relaxed` is **acceptable** for a monotonically-increasing ID (no other thread depends on seeing the update). For a simple counter, `SeqCst` would be unnecessarily strict. ✅ Correct.

**Issue**: Trace IDs are generated but **never passed** to any `emit_*` function. The event modules don't accept a `TraceId` parameter, so trace IDs exist in isolation — they can never be correlated with actual events.

### `diagnostics/snapshot.rs` ✅ Clean design

- `EngineSnapshot::capture()` takes `&Scheduler` (read-only), `running_count`, and `started_at`
- Proper `Display` impl with sorted node IDs
- Tests verify content and output format

**Minor issue**: `snapshot.rs:132` uses `ns.counters.complete`, `ns.counters.failed`, `ns.counters.timeout` — this matches the **new** `NodeCounters` definition. ✅ Correct.

---

## 🔴 DR1 Re-assessment: Global timeout → Node timeout

**Previous finding DR1** said `tokio::select!` re-creates `sleep()` every loop iteration causing reset. **Re-assessing after code changes**:

The engine no longer has a global timeout. It now has a **node idle timeout** (`tokio::time::sleep(node_timeout)` in each loop iteration). This means:

- Each loop iteration creates a new `sleep(node_timeout)` future
- If events arrive faster than `node_timeout`, the timeout never fires (by design)
- If no events arrive for `node_timeout` duration, the engine reports `NodeTimeout`

**New analysis**: This is actually a **different semantic concern now**:

1. `node_timeout` defaults to **3600 seconds** — a 1-hour node idle timeout is essentially "never fires" for most workflows
2. The `tokio::select!` structure has a subtle issue: if the event loop processes an event, the `sleep()` from the **previous** iteration is dropped and a new one is created for the next iteration. This is correct for an **idle** timeout.
3. **But**: A truly long-running node (e.g., a 30-minute subprocess) holds `running_count > 0` while waiting. The event loop blocks on `recv()`, and the 3600-second idle timer **also advances** during this time. If the node takes 3601 seconds, the idle timeout fires even though the engine is working.

**Recommendation**: The idle timeout in the event loop should be **separate** from any node's `process_timeout_secs`. Consider removing the select-based timeout entirely and relying on per-node timeouts (already handled in `SubprocessExecutor::run()` with `tokio::time::timeout`).

---

## 🟡 DR2 Re-assessment: `running_count` / convergence

**Original concern**: `handle_event()` sends `NodeReady` via channel, but `is_converged()` only checks `ready_queue` — not the channel.

**Re-analysis**: The actual code path is:

```
1. handle_event() → scheduler.handle_event() → returns ready_nodes = [B, C]
2. tx.send(NodeReady(B)), tx.send(NodeReady(C))  → channel
3. running_count -= 1
4. Loop: check convergence → recv → process → ...
```

The immediate `recv()` in step 4 will pick up B and C. The issue only manifests if **all** nodes are `Completed`/`Failed`/`TimedOut` **and** the channel has pending events **only** for already-terminated nodes (which doesn't happen in practice).

**Verdict**: Theoretically possible but **extremely unlikely** with unbounded mpsc. Not a pressing bug but should be documented.

---

## 🆕 New Findings This Round

### NR1: Engine timeouts now named confusingly

`RuntimeError::NodeTimeout` is emitted when the EVENT LOOP is idle for `node_timeout` — not when a node exceeds its `process_timeout_secs`. The name implies the latter. This is a **naming bug**:

```rust
// engine.rs:103
_ = tokio::time::sleep(node_timeout) => {
    return Err(RuntimeError::NodeTimeout);  // ← "node exceeded timeout"
}
```

But the actual per-node timeout (SubprocessExecutor's `tokio::time::timeout`) returns `NodeOutcome { timed_out: true }` which becomes `EventType::Timeout`. That's handled correctly.

The `RuntimeError::NodeTimeout` should be renamed to `RuntimeError::EventLoopStall` or `RuntimeError::IdleTimeout`.

**Impact**: Low (affects error messages only), but confusing.

---

### NR2: Diagnostics events never emitted

All four `emit_*` functions exist but are **not called**:

```bash
grep -r "emit_lifecycle\|emit_engine\|emit_system" nexus-engine/src/
# → Only declarations and test calls found
```

No calls from `engine.rs`, `scheduler.rs`, or `subprocess.rs`. The diagnostics observability layer is **declared but non-functional**.

---

### NR3: No global workflow timeout

The previous `global_timeout_secs: u64` was removed and replaced with `default_node_timeout_secs: u64`. There is now **no upper bound** on total workflow execution time. A workflow with:
- 10,000 nodes × 1 second each = 10,000 seconds (~2.8 hours)
- `default_node_timeout_secs = 3600` (1 hour)

Expected: workflow times out at 1 hour (previous behavior).  
Actual: runs for 2.8 hours (each node's 1-second timeout is per-node, no global cap).

> **处理方案**: 不修，无此需求。`default_node_timeout_secs` 的语义是每个节点的默认超时（可以被节点的 `process_timeout_secs` 覆盖），不是全局总执行时间上限。当前设计的意图就是节点级别超时，全局兜底不在需求范围内。如果未来需要，可加回 `global_timeout_secs` 并用循环外 `tokio::pin!` 实现。

---

### NR4: 3 source files have zero tests

| File | Lines | Tests |
|---|---|---|
| `nodeshell/types.rs` | 35 | 0 |
| `runtime/engine.rs` | 212 | 0 |

The engine's main event loop has **zero unit tests** — the most critical runtime component is untested. The `nodeshell::NodeContext`/`NodeOutcome`/`SpawnError` types have no dedicated tests (though used indirectly through subprocess tests).

> **处理方案**: ✅ 已修复。`runtime/engine.rs` 新增 5 个测试（引擎创建、重复节点拒绝、空工作流收敛、配置默认值）。`nodeshell/types.rs` 是纯数据结构（`NodeContext`/`NodeOutcome`/`SpawnError`），通过 subprocess 测试间接覆盖，不需要独立测试。

---

### NR5: `diagnostics` module not re-exported at crate root

In `lib.rs:44`:
```rust
pub mod diagnostics;
```

The module is `pub mod` but nothing from it is re-exported (`pub use`). Consumers of `nexus-engine` need to use `nexus_engine::diagnostics::snapshot::EngineSnapshot` — a deeply nested path. For a public API intended for observability, consider adding top-level re-exports.

> **处理方案**: 不修。目前唯一的外部消费者是 CLI，CLI 直接通过完整路径 `nexus_engine::diagnostics::snapshot::EngineSnapshot` 引用，仅此一处。等 diagnostics 模块的外部使用者多起来再考虑 re-export。

---

### NR6: MCP `describe_schema` still omits `returns` and `max_retries`

In `nexus-mcp-server/src/main.rs`, the hand-written JSON schema for `NodeDef` correctly includes `"returns"` and `"max_retries"` as optional fields. ✅ This was mentioned in L4 of previous findings but is actually correct now.

> **处理方案**: 本发现本身就是 false alarm——schema 已经正确包含了 `returns` 和 `max_retries`，无需任何改动。

---

### NR7: Test coverage gaps by module

> **处理方案**: 信息性记录，无需改动。`runtime/engine.rs` 的覆盖缺口已在 NR4 修复。其他模块覆盖度合理——高复杂度模块（scheduler/validator/builder）覆盖充足，低复杂度模块（types/diagnostics event）覆盖适中。如果未来新增 engine 功能，参照现有测试模式补充即可。

| Module | Lines | Test Annotations | Coverage Assessment |
|---|---|---|---|
| `model/*` | ~220 | 20 | ✅ Comprehensive (serde, roundtrip, edge cases) |
| `graph/edge.rs` | 125 | 5 | ✅ Adequate (state, serde, strategies) |
| `graph/graph_def.rs` | 444 | 9 | ✅ Strong invariant testing |
| `graph/builder.rs` | 461 | 10 | ✅ Good (chains, fan-out/in, strategies, merge) |
| `graph/validator.rs` | 546 | 14 | ✅ Excellent (9 checks all tested) |
| `graph/scheduler.rs` | 969 | 18 | ✅ Good, but missing: convergence race, channel fill |
| `graph/data_router.rs` | 135 | 9 | ✅ Excellent (all edge cases covered) |
| `nodeshell/subprocess.rs` | 199 | 4 | ⚠️ Missing: stderr handling, large output |
| **`runtime/engine.rs`** | **212** | **0** | ❌ **No tests** |
| `diagnostics/event.rs` | 248 | 4 | ⚠️ Smoke tests only, no integration |
| `diagnostics/trace.rs` | 42 | 3 | ✅ Adequate |
| `diagnostics/snapshot.rs` | 193 | 3 | ⚠️ Basic capture/display only |

---

## Summary of All Known Issues (As of Round 3)

### 🔴 Critical (0 remaining after fixes)
- ~~C1: `execution_count` dead code~~ → **FIXED** ✅

### 🟡 Moderate (4 remaining)

| ID | Issue | Filed In | Status |
|---|---|---|---|
| NR1 | `RuntimeError::NodeTimeout` misnamed (should be `IdleTimeout`) | This round | ✅ **Fixed** — renamed to `IdleTimeout` |
| NR3 | No global workflow timeout | This round | New — no current requirement, L3 system-level |
| NR4 | `engine.rs` has zero tests | This round | New — acknowledged gap |
| D5 | ARCHITECTURE.md §4.6 `NodeCounters` docs outdated | 05-doc-code-drift.md | ✅ **Closed** — code already matches per C1 fix |

### 🟡 Moderate — Closed (resolved or decision recorded)

| ID | Issue | Filed In | Status |
|---|---|---|---|
| M6 | `EngineEvent` missing `NodeCompleted` variant | 02-moderate.md | ✅ **Closed** — ARCHITECTURE.md updated (Option 2), code correct |
| DR2 | `is_converged()` doesn't check channel | 07-deep-review.md | ✅ **Closed** — L3 system-level, not fixing, diagnostics coverage |
| DR4 | stdout pipe deadlock (read after wait) | 07-deep-review.md | ✅ **Closed** — L3 system-level, not fixing, diagnostics coverage |
| M4 | `Strategy` serde PascalCase vs `TriggerExpr` lowercase | 02-moderate.md | ✅ **Closed** — internal type, not user-facing, low priority |

### 🟢 Low (8 remaining)

| ID | Issue | Filed In | Status |
|---|---|---|---|
| L1 | CLI tests placeholder | 03-low.md | Open |
| L3 | `build.bat` hardcoded path | 03-low.md | Open |
| L5 | `WorkflowResult` empty struct | 03-low.md | Open |
| DR6 | `GraphDef.node_indices()` replaces petgraph built-in | 07-deep-review.md | Open |
| DR7 | `process_timeout_secs` missing → cryptic serde error | 07-deep-review.md | Open |
| DR8 | MCP `unwrap_or_default()` silently swallows errors | 07-deep-review.md | Open |
| NR2 | Diagnostics events never emitted | This round | New — framework in place, integration pending |
| NR5 | `diagnostics` not re-exported at crate root | This round | New |
