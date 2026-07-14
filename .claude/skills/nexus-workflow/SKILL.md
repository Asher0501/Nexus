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

**llm_node.py does**: receive `--cmd`, read stdin context, spawn CLI (claude/opencode/nga), forward NDJSON to stderr (→ engine log), parse final output, extract route+content, write to stdout.
**llm_node.py does NOT**: write files, modify engine state, need extra args. All behavior is controlled by the prompt.

**Shell nodes do**: anything — echo JSON, run .bat scripts, call Python scripts for file I/O.
**Shell nodes do NOT**: have `{{prompt}}` templating. Use `{{inputs.X}}` for upstream data.

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
{"nodes":[{"id":"ask","providers":[{"type":"llm","command":"claude -p \"{{prompt}}\" --output-format json --verbose","prompt":"Your prompt. Output ONLY: {\"route\":\"ok\",\"content\":\"...\"}","routes":["ok"]}],"process_timeout_secs":120}]}
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

### Directed Cycle (review ⇄ fix, N rounds) — PRIMARY PATTERN

```json
{
  "nodes": [
    {
      "id": "review",
      "providers": [{
        "type": "llm",
        "command": "claude -p \"{{prompt}}\" --output-format json --verbose",
        "prompt": "Review (Round {{metadata.run_count}}/N). Input: {{inputs.start}}. Previous fix: {{inputs.fix}}. Issues → route='needs_fix'. All good → route='done'. Output ONLY JSON.",
        "routes": ["needs_fix", "done"]
      }],
      "route_policy": { "type": "max_runs", "max": N, "then_route": "done" }
    },
    {
      "id": "fix",
      "providers": [{
        "type": "llm",
        "command": "claude -p \"{{prompt}}\" --output-format json --verbose",
        "prompt": "Fix issues. Review: {{inputs.review}}. Original: {{inputs.start}}. Output ONLY JSON with route='fixed'.",
        "routes": ["fixed"]
      }]
    }
  ],
  "edges": [
    { "from": "review", "to": "fix",    "event": "complete", "exit_reason": "needs_fix" },
    { "from": "fix",    "to": "review", "event": "complete" },
    { "from": "review", "to": "done",   "event": "complete", "exit_reason": "done" }
  ]
}
```

**Cycle mechanics**:
1. Review outputs `needs_fix` → edge review→fix fires → fix runs → fix outputs `fixed` → edge fix→review fires → cycle continues
2. Review runs N times. On run N, `route_policy` overrides output route to `done` → edge review→done fires → cycle exits
3. `{{metadata.run_count}}` in prompt tells LLM which round. Values: 1, 2, ..., N.
4. Cycle edges: forward (review→fix) has `exit_reason`. Return (fix→review) has NO `exit_reason`. Exit (review→done) matches `then_route`.

## Provider Types

| Type | Use case | Command handling |
|------|----------|-----------------|
| `"llm"` | Claude/opencode/nga calls | Engine passes `--cmd` to `llm_node.py`. Prompt from `prompt` field. `{{inputs.X}}` and `{{metadata.run_count}}` auto-replaced. |
| `"shell"` | Scripts, echo, simple commands | Wrapped in `cmd /c` (Win) or `sh -c` (Unix). `{{inputs.X}}` replaced. |
| `"subprocess"` | Direct executable | No shell. Split on whitespace. Avoid — use `"shell"` instead. |

## Command Rules

- **Shell nodes**: Use `scripts/xxx.bat` for deterministic output on Windows. `.bat` files are auto-resolved relative to the engine binary.
- **LLM nodes**: `command` is the CLI invocation template. `{{prompt}}` is replaced by `llm_node.py` from stdin context (NOT by the engine). Prompt can contain `{{inputs.X}}` which the engine replaces BEFORE sending to stdin.
- **Avoid inline `python -c "..."`** — cmd.exe quoting breaks on Windows. Put Python in `scripts/xxx.py` instead.
- **Scripts location**: Put `.bat`/`.py` files in both `engine/scripts/` and `release/scripts/`.

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

Two approaches, both valid:

1. **Shell node**: `{"type":"shell","command":"python scripts/write_report.py"}` — script reads stdin context, writes `inputs.review`/`inputs.fix` to files.
2. **Prompt instruction**: Tell Claude in the prompt to write files directly (Claude Code has Write/Bash tools in `-p` mode). Example: `"Write your review to review.md using the Write tool."` No extra nodes needed.

## LLM Prompt Design

- **Always specify exact output format**: `Output ONLY: {"route":"needs_fix|done","content":"## Review\n..."}`
- **Always include `routes` list**: `"routes": ["needs_fix", "done"]`
- **Use template variables**: `{{inputs.X}}` for upstream data, `{{metadata.run_count}}` for cycle iteration
- **Tell Claude about file output**: if you want files written, include `"Write your output to X.md"` in the prompt.

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
| Cycle never exits | `route_policy` missing or wrong `then_route` | Add `route_policy: {max_runs: N, then_route: "done"}` |
| Fix node never starts | `exit_reason` mismatch | Match edge `exit_reason` exactly to LLM output `route` |
| LLM route always empty | Claude doesn't output JSON | Prompt must say "Output ONLY JSON with route field"; add `routes` list |
| No data in downstream node | Missing dataflow | Add `dataflows: [{from: X, to: Y}]` |
| Node retries 3× then fails | Stdout not valid JSON with `route` | Verify node output: must be `{"route":"...","content":"..."}` |
| Python -c fails on Windows | cmd.exe quoting | Use `.bat` script or `scripts/xxx.py` |
| `{{inputs.X}}` not replaced | No dataflow from X to current node | Check dataflows array |
