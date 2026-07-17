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

## Core Design

### h_e + g_e — Stateless Edge Decomposition

Every edge is two orthogonal **pure functions** — no `triggered` flags, no state machines:

- **h_e**: event type match + exit_reason filter + threshold counter (stateless, re-evaluated each event)
- **g_e**: strategy aggregation — `any` (fire immediately) or `all` (wait for all upstreams)

**Value**: cycles are naturally supported without special handling. A node can re-enter itself through a cycle and h_e re-evaluates independently each time. Shorter, simpler scheduler code. Easier to reason about.

### Scheduling / Dataflow Separation

`edges` control execution order. `dataflows` control data routing. **They are independent graphs.** You can route data opposite to execution direction, skip levels, or use aliases.

**Value**: Maximum routing flexibility. A downstream node can consume data from any upstream node regardless of the execution topology.

### Provider Abstraction

5 provider types under a single JSON protocol — stdin context in, stdout `{route, content}` out:

| Provider | Example |
|----------|---------|
| `subprocess` | `"command": "python script.py"` |
| `shell` | `"command": "grep error log.txt \| wc -l"` |
| `http` | `"url": "https://api.example.com", "method": "POST"` |
| `llm` | `"command": "claude -p \"{{prompt}}\""` |
| `llm_sdk` | `"model": "claude-sonnet-5-20251001"` |

**Value**: Heterogeneous nodes in one DAG. An HTTP health check can trigger an LLM analysis, whose output a shell script processes. No glue code.

### LLM-Native Tool Architecture

`llm_sdk` nodes have access to `read_file`, `write_file`, `execute_command`, and `ask_human`. LLM autonomously decides which tool to use and when.

**Value**: The LLM drives the workflow, not the other way around. It decides it needs clarification → calls `ask_human`. It decides it needs to read a file → calls `read_file`. No preset interaction points.

### Human-in-the-Loop as a Tool

`ask_human` is a tool the LLM calls, not a forced checkpoint. When uncertain, the LLM asks. Dashboard shows the question in the node panel. CLI reads from terminal. In-memory HTTP pool — zero polling, zero files.

**Value**: Human judgment enters at the LLM's discretion, not at pre-coded checkpoints. Works with fan-out — multiple LLMs ask simultaneously, questions queue up, answered one at a time.

### Route Policies for Loop Termination

`route_policy.max_runs` (N rounds) or `route_policy.max_duration` (N seconds) forces the engine to override the node's route, exiting the loop.

**Value**: Loops are safe by construction. No infinite execution. No manual checkpoint/restore.

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
