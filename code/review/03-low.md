# 🟢 Low-Priority Issues

> Nice to fix. Don't block anything but improve code quality and robustness.

---

## L1: CLI tests are placeholders only

**File**: `nexus-cli/tests/cli_tests.rs`

```rust
//! CLI integration tests for nexus-cli (Phase 7).
//! Currently a placeholder. Full test suite will be implemented in Phase 7.

#![allow(missing_docs)]
use clap as _;
use nexus_engine as _;
use serde_json as _;
use tempfile as _;
use tokio as _;
use tracing_subscriber as _;
```

No actual tests exist. Critical integration tests are missing:
- Workflow JSON parsing from file
- Validation-only mode (`--validate-only`)
- Error handling (file not found, invalid JSON)
- Timeout scenario
- Exit code verification

---

## L2: nexus-mcp-server lacks explicit tokio dependency

**File**: `nexus-mcp-server/Cargo.toml`

```toml
[dependencies]
nexus-engine = { path = "../nexus-engine" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

No `tokio` dep listed. `tokio` is pulled transitively through `nexus-engine`, but the MCP server never uses it directly (it uses sync stdio). This is correct behavior but fragile — if `nexus-engine` ever makes `tokio` optional, the MCP server silently breaks.

Not a real problem today since `nexus-engine` always needs `tokio`. Just noting for awareness.

---

## L3: `build.bat` hardcodes VS2022 BuildTools path

**File**: `build.bat`

```bat
@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
set PATH=C:\Users\Asher\.cargo\bin;%PATH%
cargo %*
```

Two issues:
1. Path is hardcoded to VS2022 BuildTools — won't work with VS2019, VS2023, or Community/Professional editions
2. PATH addition is machine-specific (`C:\Users\Asher\.cargo\bin`)

Suggestion: Use `vswhere` (Microsoft's tool for locating VS installations) or check common install paths. Or document that the user needs their VC environment set up first and remove the `call` line.

---

## L4: MCP `describe_schema` should reference `returns` and `max_retries`

**File**: `nexus-mcp-server/src/main.rs:125-198`

The `describe_schema` handler produces a JSON schema, but it includes all `NodeDef` fields **except** it doesn't reference the full JSON schema that `serde_json` produces. The schema is hand-written and may drift from the actual Rust types.

For long-term maintenance, either:
- Auto-generate the schema from `serde_json::to_value(WorkflowDef::schema())` using `schemars` crate, OR
- Add a test that verifies the hand-written schema matches the actual `NodeDef` struct fields

---

## L5: `WorkflowResult` is an empty struct

**File**: `nexus-engine/src/runtime/engine.rs:214-215`

```rust
pub struct WorkflowResult {}
```

Currently carries no information about the completed workflow. A more useful struct would include:
- Number of nodes completed/failed/timed out
- Total execution time
- List of node results (by ID)
- Final data router state

This is fine for v1 but should be noted for future enhancement.

---

## L6: Error messages use `eprintln!` instead of structured logging

**Files**: `nexus-cli/src/main.rs:76-78,85-88,136-137`, `nexus-mcp-server/src/main.rs:219,229,247-248`

Errors are written to stderr via `eprintln!()`. This works but doesn't follow structured logging patterns. The engine already depends on `tracing`. Consider using `tracing::error!()` in CLI and MCP for consistency.
