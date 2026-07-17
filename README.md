# 🕸️ Nexus

> **Declarative workflow orchestration — DAG as JSON, engine executes.** LLM agents, HTTP calls, shell scripts, human input — any process that speaks JSON on stdin/stdout is a node. Zero Python required for the engine.

[![Rust](https://img.shields.io/badge/language-Rust-orange?style=flat-square)](https://www.rust-lang.org/)
[![Tests](https://img.shields.io/badge/tests-309%20passed-green?style=flat-square)]()
[![Platforms](https://img.shields.io/badge/platform-Windows%20%7C%20Linux-blue?style=flat-square)]()

---

## Nexus vs LangGraph

| | LangGraph | Nexus |
|---|-----------|-------|
| **Workflow definition** | Python code (StateGraph) | Pure JSON — no coding |
| **Node implementation** | Python functions | Any process (any language, any runtime) |
| **Node protocol** | Function calls (in-process) | stdin/stdout JSON (cross-process, cross-language) |
| **Routing** | Conditional edges in code | `exit_reason` string matching in JSON |
| **Data flow** | Shared state object | Independent `dataflows` graph — can route opposite to execution order |
| **Cycle termination** | `interrupt()` + external resume | `route_policy` (N rounds or N seconds), stateless — no checkpoint |
| **Human-in-the-loop** | Forced `interrupt()` at node | LLM autonomously calls `ask_human` tool when uncertain |
| **Execution model** | In-process Python | Subprocess orchestration — heterogeneous nodes in one DAG |
| **Deployment** | Python runtime required | Single binary (~4MB), no runtime dependency |

LangGraph is a **graph-as-code** library for Python agents. Nexus is a **graph-as-data** engine — define in JSON, execute anywhere, nodes can be anything.

## When to Use

| Scenario | Why Nexus |
|----------|-----------|
| **LLM agent pipelines** | Multi-stage review→fix→verify loops with auto-termination |
| **Self-supervised workflows** | Design review, code audit — LLM reviews, fixes, re-reviews autonomously |
| **Microservice orchestration** | HTTP provider — call any API without writing scripts |
| **CI/CD custom pipelines** | Shell + subprocess nodes, conditional branches, retry logic |
| **Approval workflows** | Human-in-the-loop via `ask_human` — LLM asks, human answers, continues |
| **Cross-language orchestration** | Rust engine orchestrates Python, Node.js, Go, shell — any process that outputs JSON |
| **Data ETL** | Fan-out parallel processing, fan-in aggregation with `trigger: "all"` |

## Quickstart

```bash
cd release
./bin/nexus-cli run examples/http-test.json --verbose   # CLI
./bin/nexus-dashboard                                   # Dashboard → http://127.0.0.1:48080
```

## Install

Pre-compiled binaries (Windows/Linux) in `release/bin/`. No runtime dependencies.

```bash
git clone https://github.com/Asher0501/Nexus.git
cd Nexus/engine && cargo build --release
```

## 5-Minute Example: Code Review Loop

```json
{
  "nodes": [
    {"id":"seed","providers":[{"type":"shell","command":"echo fn divide(a,b){a/b}"}],"process_timeout_secs":10},
    {"id":"review","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","system_prompt":"You are a code reviewer.","prompt":"Review:\n{{datarouter.seed.content}}\n{{datarouter.fixed_code.content}}\n\nOutput ONLY: {\"route\":\"approved|needs_fix\",\"content\":\"findings\"}","routes":["approved","needs_fix"],"max_tokens":4096}],"process_timeout_secs":600,"route_policy":{"type":"max_runs","max":3,"then_route":"approved"}},
    {"id":"fix","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","prompt":"Fix ALL issues found below. Output corrected code.\n\n{{datarouter.review.content}}","routes":["done"],"max_tokens":4096}],"process_timeout_secs":600}
  ],
  "edges":[
    {"from":"seed","to":"review","trigger":"any","event":"complete"},
    {"from":"review","to":"fix","trigger":"any","event":"complete","exit_reason":"needs_fix"},
    {"from":"fix","to":"review","trigger":"any","event":"complete"},
    {"from":"review","to":"report","trigger":"any","event":"complete","exit_reason":"approved"}
  ],
  "dataflows":[{"from":"seed","to":"review"},{"from":"review","to":"fix"},{"from":"fix","to":"review","alias":"fixed_code"}]
}
```

Review→fix→review loop. `route_policy.max_runs=3` guarantees it stops.

## Design

Nexus is built in three layers: a **theory** (how the scheduler reasons about edges), an **architecture** (how the pieces fit together), and a set of **capabilities** (what the user can do).

### Theory: h_e + g_e — Stateless Edges

Every edge is decomposed into two orthogonal **pure functions**:

| Function | Role | Behavior |
|----------|------|----------|
| **h_e** | Branch matching | Checks event type, exit_reason filter, threshold counter. Stateless — re-evaluated independently for every event. |
| **g_e** | Strategy aggregation | `any` → fire immediately. `all` → wait for every upstream with `trigger:"all"` to fire, then reset. |

No `triggered` flags. No state machines. **Value**: cycles require zero special handling — a node re-entering through a cycle triggers h_e independently each time. The scheduler is shorter, simpler, and easier to reason about.

### Architecture: Three Layers

**Scheduling / dataflow separation.** `edges` control execution order; `dataflows` control data routing. They are independent graphs — data can flow opposite to execution, skip levels, or use aliases. Maximum routing flexibility without coupling to the execution topology.

**Provider abstraction.** Every node, regardless of type, follows the same protocol: stdin receives context JSON, stdout outputs `{route, content}`. Five provider types under this single contract:

| Provider | Example |
|----------|---------|
| `subprocess` | `"command": "python script.py"` |
| `shell` | `"command": "grep error log.txt \| wc -l"` |
| `http` | `"url": "https://api.example.com", "method": "POST"` |
| `llm` | `"command": "claude -p \"{{prompt}}\""` |
| `llm_sdk` | `"model": "claude-sonnet-5-20251001"` |

**Value**: heterogeneous nodes in one DAG. An HTTP health check triggers an LLM analysis, whose output a shell script processes. No glue code. Any language, any runtime.

**Route policies.** Guaranteed loop termination without checkpoints. `max_runs` exits after N rounds; `max_duration` exits after N cumulative seconds. The engine overrides the node's route — the node doesn't even need to know it's in a loop.

### Capabilities

Built on the theory and architecture above:

- **LLM-native tools**: `read_file`, `write_file`, `execute_command`. LLM decides what to use and when.
- **Human-in-the-loop**: `ask_human` is a tool the LLM calls when uncertain. In-memory HTTP pool — zero polling, zero files. Fan-out works: multiple LLMs ask simultaneously, questions queue up.
- **Branch routing**: `{"route":"approved"}` → edges with `exit_reason:"approved"` fire.
- **Fan-out / fan-in**: parallel nodes with `trigger:"all"` aggregation.
- **10-point static validation**: deadlocks, unreachable nodes, missing dataflows caught before execution.

## Key Features

- **Branch routing** — `{"route":"approved"}` → edges with `exit_reason:"approved"` fire
- **Fan-out / fan-in** — parallel nodes, `trigger:"all"` waits for completion
- **Timeout + retry** — per-node timeout, retry on timeout/spawn errors only
- **10-point validation** — deadlocks, unreachable nodes, missing dataflows caught before execution
- **Live DAG visualization** — Dashboard with WebSocket real-time status + click-to-interact
- **Stop button** — cancel running workflows, pending nodes marked Skipped
- **MCP integration** — stdio JSON-RPC server for Claude Code
- **Cross-platform** — Windows + Linux binaries, engine is a single ~4MB executable

## Documentation

| Document | Content |
|----------|---------|
| [Workflow Reference](release/WORKFLOW_REFERENCE.md) | Complete schema, scheduling semantics, node protocol |
| [Quickstart](release/QUICKSTART.md) | 5-minute tutorial with examples |
| [Skill Reference](release/NEXUS_WORKFLOW_SKILL.md) | Claude Code workflow generation guide |

## Build

```bash
cd engine
cargo build --release
# → engine/target/release/nexus-cli, nexus-dashboard, nexus-mcp-server
```
