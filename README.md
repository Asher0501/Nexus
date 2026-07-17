# 🕸️ Nexus

> **Directed-graph workflow orchestration engine.** Define a DAG, the engine executes it. LLM agents, HTTP calls, shell scripts — any process that speaks JSON on stdin/stdout is a node.

[![Rust](https://img.shields.io/badge/language-Rust-orange?style=flat-square)](https://www.rust-lang.org/)
[![Tests](https://img.shields.io/badge/tests-309%20passed-green?style=flat-square)]()

---

## Quickstart

```bash
# CLI — run a workflow
cd release
./bin/nexus-cli run examples/http-test.json --verbose

# Dashboard — REST API + live DAG visualization
./bin/nexus-dashboard
# → http://127.0.0.1:48080

# MCP Server — JSON-RPC stdio
echo '{"jsonrpc":"2.0","method":"validate_workflow","params":{"workflow_json":"{\"nodes\":[{\"id\":\"hello\",\"providers\":[{\"type\":\"subprocess\",\"command\":\"echo hi\"}],\"process_timeout_secs\":10}]}"},"id":1}' | ./bin/nexus-mcp-server
```

## Install

Pre-compiled binaries for Windows and Linux in `release/bin/`. No dependencies.

```bash
git clone https://github.com/Asher0501/Nexus.git
cd Nexus/engine
cargo build --release
```

## Hello World

```json
{"nodes":[{"id":"hello","providers":[{"type":"subprocess","command":"echo {\"route\":\"ok\",\"content\":\"Hello Nexus\"}"}],"process_timeout_secs":10}]}
```

## Code Review Loop

```json
{
  "nodes": [
    {"id":"seed","providers":[{"type":"shell","command":"echo fn divide(a,b){a/b}"}],"process_timeout_secs":10},
    {"id":"review","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","prompt":"Review: {{datarouter.seed.content}}. Output {\"route\":\"approved|needs_fix\",\"content\":\"...\"}","routes":["approved","needs_fix"],"max_tokens":4096}],"process_timeout_secs":600,"route_policy":{"type":"max_runs","max":3,"then_route":"approved"}},
    {"id":"fix","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","prompt":"Fix: {{datarouter.review.content}}","routes":["done"],"max_tokens":4096}],"process_timeout_secs":600}
  ],
  "edges": [
    {"from":"seed","to":"review","trigger":"any","event":"complete"},
    {"from":"review","to":"fix","trigger":"any","event":"complete","exit_reason":"needs_fix"},
    {"from":"fix","to":"review","trigger":"any","event":"complete"},
    {"from":"review","to":"report","trigger":"any","event":"complete","exit_reason":"approved"}
  ],
  "dataflows":[{"from":"seed","to":"review"},{"from":"review","to":"fix"},{"from":"fix","to":"review","alias":"fixed_code"}]
}
```

## Core Concepts

**Nodes** are processes that speak JSON on stdin/stdout. **Edges** define execution order. **Dataflows** define data routing. They are independent — you can route data in any direction regardless of execution order.

**Providers** determine HOW a node executes:

| Provider | Description |
|----------|-------------|
| `subprocess` | Spawn a process directly |
| `shell` | Spawn via shell (pipes, redirects) |
| `http` | Make an HTTP request |
| `llm` | Spawn an LLM CLI (claude, opencode, etc.) |
| `llm_sdk` | Call Anthropic API directly (with tools: read_file, write_file, execute_command, ask_human) |

**Route policies** guarantee loop termination: `max_runs` (N rounds) or `max_duration` (N seconds).

**Human-in-the-loop**: `llm_sdk` nodes can call `ask_human` when uncertain. Dashboard shows questions in node panels. CLI reads answers from terminal.

**ToolBridge**: `llm_sdk` automatically discovers tools from MCP servers configured in `~/.nexus/mcp.json`.

## Key Features

- **h_e + g_e decomposition** — edges are stateless pure functions; cycles are naturally supported
- **Branch routing** — nodes output `{"route":"approved","content":"..."}`, edges match `exit_reason`
- **Fan-out / fan-in** — parallel execution with `trigger: "all"` aggregation
- **Timeout + retry** — per-node timeout with configurable retry (timeout/spawn errors only)
- **10-point validation** — static DAG analysis catches deadlocks, unreachable nodes, missing dataflows
- **Live DAG visualization** — Dashboard with WebSocket real-time node status + click-to-interact HITL
- **MCP integration** — stdio JSON-RPC server, run workflows from Claude Code

## Documentation

| Document | Content |
|----------|---------|
| [Workflow Reference](release/WORKFLOW_REFERENCE.md) | Complete schema, scheduling semantics, node protocol |
| [Quickstart](release/QUICKSTART.md) | 5-minute tutorial |
| [Skill Reference](release/NEXUS_WORKFLOW_SKILL.md) | Claude Code workflow generation guide |

## Build

```bash
cd engine
cargo build --release
# Binaries in engine/target/release/
# Copy to release/bin/ with release/scripts/ to form a distribution
```
