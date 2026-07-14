# Nexus Workflow JSON Generator

Generate valid Nexus workflow JSON definitions. Every workflow is a DAG of nodes connected by edges. Nodes = subprocesses or LLM calls. Edges = scheduling triggers + data routing.

## Quick Templates

### Single LLM Node
```json
{"nodes":[{"id":"ask","providers":[{"type":"llm","command":"claude -p \"{{prompt}}\" --output-format json --verbose","prompt":"Your prompt here. Output: {\"route\":\"ok\",\"content\":\"...\"}","routes":["ok"]}],"process_timeout_secs":120}]}
```

### Sequential Chain (A→B→C)
```json
{"nodes":[
  {"id":"a","providers":[{"type":"shell","command":"scripts/node_ok.bat"}],"process_timeout_secs":10},
  {"id":"b","providers":[{"type":"shell","command":"scripts/node_ok.bat"}],"process_timeout_secs":10},
  {"id":"c","providers":[{"type":"shell","command":"scripts/node_ok.bat"}],"process_timeout_secs":10}
],"edges":[
  {"from":"a","to":"b","trigger":"any","event":"complete"},
  {"from":"b","to":"c","trigger":"any","event":"complete"}
]}
```

### Fan-out / Fan-in
```json
{"nodes":[
  {"id":"src","providers":[{"type":"shell","command":"scripts/node_ok.bat"}],"process_timeout_secs":10},
  {"id":"w1","providers":[{"type":"shell","command":"scripts/node_ok.bat"}],"process_timeout_secs":10},
  {"id":"w2","providers":[{"type":"shell","command":"scripts/node_ok.bat"}],"process_timeout_secs":10},
  {"id":"merge","providers":[{"type":"shell","command":"scripts/node_ok.bat"}],"process_timeout_secs":10}
],"edges":[
  {"from":"src","to":"w1","trigger":"any","event":"complete"},
  {"from":"src","to":"w2","trigger":"any","event":"complete"},
  {"from":"w1","to":"merge","trigger":"all","event":"complete"},
  {"from":"w2","to":"merge","trigger":"all","event":"complete"}
],"dataflows":[
  {"from":"w1","to":"merge"},{"from":"w2","to":"merge"}
]}
```

### Directed Cycle (review ⇄ fix, N rounds)

**Always use this pattern for repeated LLM operations (review→fix, validate→correct, etc.).**

```json
{"nodes":[
  {"id":"start","providers":[{"type":"shell","command":"scripts/node_ok.bat"}],"process_timeout_secs":10},
  {"id":"review","providers":[{"type":"llm","command":"claude -p \"{{prompt}}\" --output-format json --verbose","prompt":"Review. Round {{metadata.run_count}}/3. Input: {{inputs.start}}. Previous fix: {{inputs.fix}}. Issues found → route='needs_fix'. All good → route='done'. Output: {\"route\":\"needs_fix|done\",\"content\":\"## Review\\n...\"}","routes":["needs_fix","done"]}],"process_timeout_secs":180,"route_policy":{"type":"max_runs","max":3,"then_route":"done"}},
  {"id":"fix","providers":[{"type":"llm","command":"claude -p \"{{prompt}}\" --output-format json --verbose","prompt":"Fix issues. Review: {{inputs.review}}. Original: {{inputs.start}}. Output: {\"route\":\"fixed\",\"content\":\"## Fix Round {{metadata.run_count}}\\n...\"}","routes":["fixed"]}],"process_timeout_secs":180},
  {"id":"report","providers":[{"type":"shell","command":"scripts/node_ok.bat"}],"process_timeout_secs":10}
],"edges":[
  {"from":"start","to":"review","trigger":"any","event":"complete"},
  {"from":"review","to":"fix","trigger":"any","event":"complete","exit_reason":"needs_fix"},
  {"from":"fix","to":"review","trigger":"any","event":"complete"},
  {"from":"review","to":"report","trigger":"any","event":"complete","exit_reason":"done"}
],"dataflows":[
  {"from":"start","to":"review"},{"from":"start","to":"fix"},
  {"from":"fix","to":"review"},{"from":"review","to":"fix"},
  {"from":"review","to":"report"},{"from":"fix","to":"report"}
]}
```

### Conditional Branch (exit_reason routing)
```json
{"nodes":[
  {"id":"check","providers":[{"type":"shell","command":"scripts/node_approved.bat"}],"process_timeout_secs":10},
  {"id":"pass","providers":[{"type":"shell","command":"scripts/node_ok.bat"}],"process_timeout_secs":10},
  {"id":"fail","providers":[{"type":"shell","command":"scripts/node_ok.bat"}],"process_timeout_secs":10}
],"edges":[
  {"from":"check","to":"pass","trigger":"any","event":"complete","exit_reason":"approved"},
  {"from":"check","to":"fail","trigger":"any","event":"complete","exit_reason":"rejected"}
]}
```

## Critical Rules

### Cycles
- **Use `route_policy` for cycle exit**: `{"type":"max_runs","max":N,"then_route":"done"}` on the review/entry node. After N runs, the engine overrides the node's route to force exit.
- **Cycle edges**: forward edge has `exit_reason` to enter the loop. Return edge (fix→review) has NO `exit_reason` (always fires on complete). Exit edge has `exit_reason` matching the `then_route` value.
- **`{{metadata.run_count}}`** available in prompts to tell the LLM which round it is.

### Providers
- **`type: "llm"`**: For Claude/opencode/nga calls. `command` is the CLI invocation, `prompt` is the template. `routes` lists expected output routes. Engines shells through `llm_node.py` wrapper.
- **`type: "shell"`**: Wrapped in `cmd /c` (Win) or `sh -c` (Unix). Use `.bat` scripts or simple echo commands.
- **`type: "subprocess"`**: Direct spawn, no shell. Only use for absolute EXE paths (no `scripts/` resolution).

### Commands
- **Use `scripts/xxx.bat`** for deterministic output. Inline Python `-c "..."` BREAKS on Windows due to cmd quoting.
- **`scripts/` paths** are auto-resolved relative to the engine binary. Put scripts in `engine/scripts/`.
- **Node output protocol**: Every node MUST output `{"route":"...","content":"..."}` as the LAST line of stdout. Bat scripts: `echo {"route":"ok","content":"done"}`.

### Edges
- **`trigger: "any"`**: Fire downstream immediately on match. Default for single-source edges.
- **`trigger: "all"`**: Wait for ALL incoming `all` edges to fire before triggering downstream. Used for fan-in/merge.
- **`exit_reason`**: String-match the node's output `route`. Null = match any route.
- **`event`**: `"complete"` (exit 0), `"failed"` (exit != 0), `"timeout"` (killed).

### Dataflows
- **Independent from edges**: dataflow A→B ≠ edge A→B. Both must be declared.
- **Alias**: Use `"alias":"custom_name"` to rename the input key. Default key = source node ID.
- **Only latest output**: In cycles, a node's output is overwritten each iteration. Downstream nodes see the last value.

## MCP Integration

When calling `run_workflow` via MCP, always pass `dashboard_url` so the run appears in the monitoring UI:

```json
{
  "method": "run_workflow",
  "params": {
    "workflow_json": "...",
    "dashboard_url": "http://127.0.0.1:48080"
  }
}
```

Returns `{run_id, dashboard_url, monitor_url}`. The `monitor_url` can be opened in a browser for live DAG visualization.

## Common Pitfalls

| Pitfall | Fix |
|---------|-----|
| Cycle exits too early/never | Use `route_policy.max_runs` with correct `then_route` |
| LLM node route always empty | Prompt must explicitly ask for JSON with `route` field; `routes` list in provider |
| Fix node never starts | Check `exit_reason` matches review node's output `route` exactly |
| Data not flowing to downstream | Add `dataflows` entry; edges are scheduling-only |
| Python -c "..." fails on Win | Use `.bat` script in `scripts/` directory |
| Inline echo JSON breaks | Wrap in `.bat` file, avoid special chars `&|<>^%!` in echo text |
| `{{inputs.X}}` not replaced | Ensure `dataflows` has `from: X` to the current node |
