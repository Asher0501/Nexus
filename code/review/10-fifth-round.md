# 🔬 Deep Review: Nexus — Fifth Round

> **Date**: 2026-07-06  
> **Focus**: Root cause tracing of remaining issues, runtime verification, end-to-end  
>              execution testing, concurrency limit enforcement audit  
> **Build+Test**: 102/102 pass, cargo build --release, live E2E confirmed  
> **Bugs fixed since last round**: FR1, FR3

---

## Live Runtime Verification

All E2E tests executed successfully:

| Test | Command | Result |
|---|---|---|
| Basic workflow | `nexus-cli run test_workflow.json` | Workflow completed in 32ms |
| Validate only | `nexus-cli run test_workflow.json --validate-only` | Validation passed |
| Invalid path | `nexus-cli run nonexistent.json` | Exit code 1, clear error |
| Custom timeout | `nexus-cli run --node-timeout 7200 test_workflow.json` | default_timeout_secs=7200 |
| MCP describe_schema | `echo '{"method":"describe_schema"}' \| nexus-mcp-server` | Full JSON schema returned |
| MCP validate (valid) | `validate_workflow` with valid workflow | valid:true |
| MCP validate (invalid) | `validate_workflow` with empty nodes | valid:false + error message |
| MCP parse | `parse_workflow` | Full structured output |

Diagnostics events verified in production output:
```
event=Started  node_count=1  max_concurrency=8  default_timeout_secs=3600
event=Running  node_id="echo"  command=cmd.exe /c echo hello
event=Completed  node_id="echo"  output_size=7
event=Converged  duration_ms=32
```

---

## Changes Detected Since R4

Two issues have been fixed:

| Issue | Status | Evidence |
|---|---|---|
| FR1: channel None leads to false idle timeout | FIXED | engine.rs:120 `None => break` instead of falling to sleep |
| FR3: retry_node off-by-one | FIXED | scheduler.rs:296 `*count > max_retries` — correct semantics |

---

## Critical New Finding

### FR9: Concurrency limit max_concurrency not enforced

**Files**: engine.rs, config.rs  
**Severity**: CRITICAL

EngineConfig.effective_max_concurrency() returns the configured (or CPU-count-based) max concurrency, but it is never checked during execution. The value is only used for diagnostic logging in engine.rs:95. The engine launches every NodeReady event without any running_count >= max_concurrency check. There is no queuing mechanism for excess concurrency at the engine level.

Impact: A workflow with 64 nodes and no predecessor constraints will launch all 64 subprocesses simultaneously, regardless of --max-concurrency 4.

> **处理方案**: ✅ 已修复。Engine 新增 `Arc<Semaphore>`，`handle_event()` 在 spawn 子进程前调用 `acquire().await`。许可用尽时自动等待，无需手动管理队列。同时新增 `Executor` 概念层（ARCHITECTURE.md §5.6），将执行层职责从调度层中分离。Scheduler 只负责"哪个节点可触发"，执行层负责"哪个触发了的节点现在能跑"。

---

## Moderate Findings

### FR10: NodeResult::Completed(String::new()) placeholder still present

scheduler.rs:180-185: When handle_event processes a Complete event, it sets the node result to Completed(String::new()) with an empty string. The actual output is stored in DataRouter. Two sources of truth for node output.

> **处理方案**: ❌ 本发现为 false alarm。`NodeResult::Completed(String)` 已在问题 M3 时改为 unit variant `Completed`，不再携带空字符串。实际代码：`scheduler.rs:185` 为 `ns.result = NodeResult::Completed;`。

### FR11: MCP parse_workflow returns null for exit_reason

Cosmetic only — serde serializes Option None as null.

---

## Low Findings

### FR12: build.bat vcvars path is redundant for Rust builds

The call vcvars64.bat line is not needed by cargo's MSVC toolchain. It fails silently if VS BuildTools is not at the hardcoded path.

### FR13: scripts/build-all.ps1 is functional but uses --$config (broken syntax)

The script uses cargo build --$config -p nexus-engine where $config is "release" or "debug". The correct flag syntax is --release or (no flag for debug). Using --$config produces --release (correct) or --debug (wrong — should be no flag).

### FR14: MCP server serde_json::to_value unwrap_or_default silently returns null

In make_success/make_error (mcp-server/main.rs:41,56): if serialization fails, the client receives null instead of a proper error response.

---

## Current State: All Issues

### Fixed across 5 rounds: 11 issues
C1, M1, M2, M5, NR1, NR2, NR4, FR1, FR3, FR7, FR9

### False alarms: 2 issues
FR10 (Completed placeholder already unit variant), NR6 (describe_schema already correct)

### Remaining Open: 6 issues
- MODERATE: DR4 (stdout deadlock — L3 system level, not fixing), M4/FR5 (Strategy serde — low priority)
- LOW: L1 (CLI tests), DR6/7/8, FR11/12/13/14
