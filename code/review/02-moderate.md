# 🟡 Moderate Issues

> Should fix. These affect correctness, UX, or developer experience in non-trivial ways.

---

## M1: CLI exit code 3 (Timeout) not implemented

**Files**: `nexus-cli/src/main.rs:130-139`, `docs/architecture/ARCHITECTURE.md §8`

> **处理方案**: ✅ 已修复。CLI 现在区分 `RuntimeError::NodeTimeout` → exit(3) 和其他运行时错误 → exit(2)。

### What's wrong

ARCHITECTURE.md §8 specifies three exit codes:

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Validation/read error |
| 2 | Runtime error |
| **3** | **Timeout** ← not implemented |

The CLI `main.rs` runtime result handler:
```rust
match engine.run().await {
    Ok(_result) => {
        println!("Workflow completed successfully.");
        std::process::exit(0);
    }
    Err(e) => {
        eprintln!("Runtime error: {:?}", e);  // ← Both timeout and spawn error go here
        std::process::exit(2);                 // ← Always exit code 2
    }
}
```

`RuntimeError::GlobalTimeout` exits with code 2 instead of 3.

### Fix

```rust
Err(RuntimeError::GlobalTimeout) => {
    eprintln!("Workflow timed out.");
    std::process::exit(3);
}
Err(RuntimeError::SpawnError(msg)) => {
    eprintln!("Runtime error: {}", msg);
    std::process::exit(2);
}
```

---

## M2: CLI sentinel value `0` for concurrency/timeout options

**File**: `nexus-cli/src/main.rs:32-38`

> **处理方案**: ✅ 已修复。`max_concurrency` 和 `node_timeout` 已改为 `Option<usize>`/`Option<u64>`，不再用 `0` 做 sentinel。

### What's wrong

```rust
#[arg(long, default_value_t = 0)]
max_concurrency: usize,
#[arg(long, default_value_t = 0)]
global_timeout: u64,
```

Using `0` as a sentinel for "not set" means the user cannot explicitly pass `--max-concurrency 0` even though `0` is a valid integer. The downstream conversion:

```rust
let max_conc = if max_concurrency > 0 { Some(max_concurrency) } else { None };
let global_to = if global_timeout > 0 { global_timeout } else { 3600 };
```

### Fix

```rust
/// Maximum number of concurrent nodes (default: CPU count).
#[arg(long)]
max_concurrency: Option<usize>,

/// Global timeout in seconds (default: 3600).
#[arg(long)]
global_timeout: Option<u64>,
```

Then use directly:
```rust
let config = EngineConfig::new(max_concurrency, global_timeout.unwrap_or(3600), 3);
```

---

## M3: `NodeResult::None` → `Completed(String::new())` empty string placeholder

**File**: `nexus-engine/src/graph/scheduler.rs:180-185`

> **处理方案**: ✅ 已修复。`NodeResult::Completed(String)` → `NodeResult::Completed`（unit variant），不再有空字符串占位。节点产出真实来源为 `DataRouter`。

### What's wrong

```rust
EventType::Complete => {
    ns.status = NodeStatus::Completed;
    if matches!(ns.result, NodeResult::None) {
        ns.result = NodeResult::Completed(String::new());  // ← empty string placeholder
    }
}
```

When `handle_event` processes a `Complete` event, it sets the node result to `Completed(String::new())` with an empty string. But the actual node output is stored in `DataRouter`, not in `Scheduler`. The empty string has no meaning and creates inconsistency with `Failed(String)` and `TimedOut` which carry real payloads.

### Fix

Make `NodeResult::Completed` a unit variant (no payload):

```rust
pub enum NodeResult {
    None,
    Completed,       // was Completed(String)
    Failed(String),
    TimedOut,
}
```

Or remove `result` from `NodeState` entirely since `Scheduler` doesn't consume it — `DataRouter` is the source of truth for node outputs.

---

## M4: `Strategy` serde PascalCase vs `TriggerExpr` lowercase

**Files**: `nexus-engine/src/graph/edge.rs:86-87`, `nexus-engine/src/model/predecessor.rs:10-17`

> **处理方案**: `Strategy` 是内部运行时类型（`EdgeDef` 无 `Serialize`/`Deserialize` derive），不暴露给用户 JSON。当前无影响，标记为低优先级。若未来需要将 `Strategy` 暴露给用户，再加 `#[serde(rename_all = "lowercase")]`。

### What's wrong

```rust
// edge.rs — Strategy serializes as "All" / "Any" (PascalCase — derived, no rename)
pub enum Strategy { All, Any }

// predecessor.rs — TriggerExpr serializes as "all" / "any" (lowercase — explicit rename)
pub enum TriggerExpr {
    #[serde(rename = "all")] All,
    #[serde(rename = "any")] Any,
}
```

Two enums with isomorphic variants serialize differently. If `Strategy` is ever exposed in user-facing JSON (e.g., MCP describe_schema or future workflow definition), users will see `"All"` in one context and `"all"` in another.

### Fix

Add `#[serde(rename_all = "lowercase")]` or explicit renames to `Strategy`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Strategy {
    All,
    Any,
}
```

---

## M5: stdin not explicitly closed after writing context JSON

**File**: `nexus-engine/src/nodeshell/subprocess.rs:70-75`

> **处理方案**: ✅ 已修复。改用 `child.stdin.take()` 获取 `ChildStdin` 所有权，写完 stdin 后作用域结束自动 drop（等价于 `fclose`），子进程立即收到 EOF。

### What's wrong

```rust
if let Some(stdin) = child.stdin.as_mut() {
    let input = serde_json::to_string(&ctx).unwrap_or_default();
    let _ = stdin.write_all(input.as_bytes()).await;
    let _ = stdin;  // ← This is a no-op — does NOT close/drop stdin
}
```

`let _ = stdin;` binds `stdin` (a `&mut ChildStdin`) to the `_` pattern, which is a deliberate no-op. The actual `stdin` drop happens when `cmd` goes out of scope at the end of `run()`. This works but is fragile — if the child process expects stdin EOF before producing output, the timing depends on drop order.

### Fix

```rust
if let Some(stdin) = child.stdin.as_mut() {
    let input = serde_json::to_string(&ctx).unwrap_or_default();
    let _ = stdin.write_all(input.as_bytes()).await;
    // Explicitly close stdin to signal EOF to child process.
    // We have to consume the ChildStdin, which means accessing it
    // through the Child struct directly.
}
drop(child.stdin.take());  // ← This actually drops stdin
```

Or use the `shutdown()` method on `tokio::process::ChildStdin`.

---

## M6: `EngineEvent` missing `NodeCompleted` variant

**File**: `nexus-engine/src/runtime/engine.rs:14-20`

> **处理方案**: 已按 Option 2 修复——更新 ARCHITECTURE.md §5.1/§5.2，去掉 `NodeCompleted` 事件定义，伪代码同步为实际实现。`NodeCompleted` 的处理在 `handle_event()` 中同步内联完成，不经过事件队列，功能正确，不需要增加 `NodeCompleted` 事件。

### What's wrong

ARCHITECTURE.md §5.1 defines:
```rust
enum EngineEvent {
    NodeReady { node_id: NodeIndex },
    NodeCompleted { node_id, output, timed_out, exit_code, exit_reason },
}
```

But the actual code has only:
```rust
enum EngineEvent {
    NodeReady { node_id: NodeIndex },
}
```

`NodeCompleted` events are handled synchronously within `handle_event()` — the Executor runs and processes completion inline, without sending a message through the channel. This works but makes the architecture different from what's documented.

### Fix

Either:
1. Send `NodeCompleted` through the channel and split `handle_event()` into two handlers, OR
2. Update ARCHITECTURE.md §5.1 to match the actual implementation.

Option 2 is recommended — the current architecture is simpler and correct.
