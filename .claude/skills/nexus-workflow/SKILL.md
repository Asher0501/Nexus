# Nexus Workflow JSON Generator

Generate valid Nexus workflow JSON. Express ALL logic in the JSON — never modify the engine or `llm_node.py`.

## Architecture Principle

```
JSON Workflow  ──defines──→  Engine (DAG scheduler)  ──spawns──→  Nodes (execution)
                                                                    │
                                              ┌─────────────────────┤
                                              │ type: "llm"         │ type: "shell"
                                              │ → llm_node.py       │ → cmd /c <command>
                                              │   (pure wrapper)    │   (.bat / echo / python)
                                              └─────────────────────┘
```

**Engine does**: DAG scheduling, edge matching (h_e+g_e), data routing, spawn/retry/timeout, convergence watchdog.
**Engine does NOT**: care about what nodes do internally. Nodes are black boxes that follow the stdout JSON protocol.

**llm_node.py does**: receive `--cmd`, read stdin context, spawn CLI, forward output to engine log, parse route+content, write to stdout.
**llm_node.py does NOT**: read files, modify prompts, write files. Pure glue.

**Shell nodes do**: anything — echo JSON, run .bat scripts, call Python scripts for file I/O.

## Node Output Protocol (NON-NEGOTIABLE)

Every node MUST output exactly one JSON object as the last line of stdout:

```json
{"route":"<string>","content":"<string>"}
```

- `route`: non-empty = used for `exit_reason` edge matching. Empty = only matches `exit_reason: null` edges.
- `content`: arbitrary text, passed to downstream nodes via dataflows.
- Exit code 0 = `complete` event. Non-zero = `failed`. Timeout = `timeout`.
- Invalid JSON or missing `route` → node fails, retried up to 3 times.

## Templates

### Single LLM Node
```json
{"nodes":[{"id":"ask","providers":[{"type":"llm","command":"claude --output-format stream-json --verbose --include-partial-messages --dangerously-skip-permissions","prompt":"Your prompt. Output ONLY: {\"route\":\"ok\",\"content\":\"...\"}","routes":["ok"]}],"process_timeout_secs":120}]}
```

### Chain (A→B→C)
```json
{"edges":[
  {"from":"a","to":"b","trigger":"any","event":"complete"},
  {"from":"b","to":"c","trigger":"any","event":"complete"}
]}
```

### Fan-out / Fan-in
`trigger: "all"` on merge node's incoming edges waits for all upstream to complete.
Both edges AND dataflows must be declared independently.

### Conditional Branch
`exit_reason` on edge matches the node's output `route` field exactly. Null exit_reason matches any route.

### Directed Cycle (general pattern for repeated operations)

```json
{
  "nodes": [
    {
      "id": "A",
      "providers": [{
        "type": "llm",
        "command": "claude --output-format stream-json --verbose --include-partial-messages --dangerously-skip-permissions",
        "prompt": "Task (Round {{metadata.run_count}}/N). Input: {{inputs.seed}}. B output: {{inputs.B}}. Continue → route='again'. Done → route='stop'. Output ONLY JSON.",
        "routes": ["again", "stop"]
      }],
      "route_policy": { "type": "max_runs", "max": N, "then_route": "stop" }
    },
    {
      "id": "B",
      "providers": [{
        "type": "llm",
        "command": "claude --output-format stream-json --verbose --include-partial-messages --dangerously-skip-permissions",
        "prompt": "Process iteration. A output: {{inputs.A}}. Output ONLY JSON with route='done'.",
        "routes": ["done"]
      }]
    }
  ],
  "edges": [
    { "from": "A", "to": "B", "event": "complete", "exit_reason": "again" },
    { "from": "B", "to": "A", "event": "complete" },
    { "from": "A", "to": "C", "event": "complete", "exit_reason": "stop" }
  ]
}
```

**Directed cycle — general rule**:
- 所有节点一律平等：输出 route，引擎匹配边。
- 一个节点是否参与环路决策，仅取决于它有没有出边指向环内。
- 终止保证：至少一个环内节点带 `route_policy`（N 轮后强制改 route）或 `threshold`（N 次事件后触发退出边）。
- Validator 要求所有节点可达出口（无出边的节点）。环需要一条退出边 → 出口信号节点。

```
A ──"again"──→ B ──→ A  (loop)
A ──"stop"───→ C         (exit)
```

## Provider Types

| Type | Use case | Command handling |
|------|----------|-----------------|
| `"llm"` | Claude/opencode/nga | Engine passes `--cmd` to `llm_node.py`. Prompt from `prompt` field, sent via stdin. `{{inputs.X}}` auto-replaced by engine. `{{metadata.run_count}}` auto-replaced. |
| `"shell"` | Scripts, echo, simple commands | Wrapped in `cmd /c` (Win) or `sh -c` (Unix). `{{inputs.X}}` replaced. |
| `"subprocess"` | Direct executable | No shell. Split on whitespace. Avoid — use `"shell"` instead. |

## Command Rules

- **Shell nodes**: Use `scripts/xxx.bat` for deterministic output. `.bat` files auto-resolved relative to engine binary.
- **LLM nodes**: `command` is CLI flags only (no `-p`). Prompt goes via stdin, no command-line length limit.
- **Avoid inline `python -c "..."`** — quoting breaks on Windows. Put Python in `scripts/xxx.py`.
- **Scripts location**: Put `.bat`/`.py` in both `engine/scripts/` and `release/scripts/`.

## Edge Rules

- `trigger: "any"` — immediate downstream trigger. Default.
- `trigger: "all"` — wait for ALL `all`-strategy incoming edges. Use for fan-in.
- `exit_reason` — exact string match on node's output `route`. `null` matches any route (including empty).
- `event` — `"complete"` (exit 0), `"failed"` (exit ≠ 0), `"timeout"` (killed by engine).
- `threshold` — fire after N matching events. Default 1. Use for self-loops.

## Dataflow Rules

- **Edges ≠ Dataflows**: edges schedule execution order. Dataflows route data. Both are independent graphs.
- **`alias`**: rename input key for downstream node. Default key = source node ID.
- **Latest only**: in cycles, each run overwrites previous output. Downstream sees final value.

## File Output Strategy

Two approaches:

1. **Shell node**: `{"type":"shell","command":"python scripts/write_report.py"}` — script reads stdin context, writes files.
2. **Prompt instruction**: Tell Claude to write files directly (Claude Code has Write/Bash tools in `-p` mode). Example: `"Write your review to review.md."` No extra nodes needed.

## Prompt Length Strategy

When prompt exceeds ~4KB (e.g., reviewing a large document): give Claude the file path in the prompt. Claude reads the file itself via the Read tool. File paths are short; the document content never hits the command line.

```
A(llm: "Read ../path/to/doc.md. Review...")
```

## LLM Prompt Design

- **Always specify exact output format**: `Output ONLY: {"route":"again|stop","content":"## Review\n..."}`
- **Always include `routes` list**: `"routes": ["again", "stop"]`
- **Use template variables**: `{{inputs.X}}` for upstream data, `{{metadata.run_count}}` for cycle iteration

## MCP Integration

```json
{
  "method": "run_workflow",
  "params": {
    "workflow_json": "<JSON string>",
    "dashboard_url": "http://127.0.0.1:48080"
  }
}
```

Returns `{run_id, dashboard_url, monitor_url}`.

## Common Pitfalls

| Symptom | Root cause | Fix |
|---------|-----------|-----|
| Cycle never exits | `route_policy` missing or wrong `then_route` | Add `route_policy: {max_runs: N, then_route: "stop"}` |
| Fix node never starts | `exit_reason` mismatch | Match edge `exit_reason` exactly to LLM output `route` |
| LLM route always empty | Claude doesn't output JSON | Prompt must say "Output ONLY JSON with route field"; add `routes` list |
| No data in downstream node | Missing dataflow | Add `dataflows: [{from: X, to: Y}]` |
| `{{inputs.X}}` not replaced | No dataflow from X to current node | Check dataflows array |
| Workflow rejected: "exit not reachable" | No node with 0 outgoing edges reachable | Add exit signal node + exit edge from cycle |
| Prompt too long, Claude hangs | Document in prompt > ~4KB via `-p` | Use seed node or give file path instead |
