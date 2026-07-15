# Nexus Workflow JSON Generator / Nexus 工作流 JSON 生成器

Generate valid Nexus workflow JSON. Express ALL logic in the JSON — never modify the engine or `llm_node.py`.
所有逻辑表达在 JSON 中——禁止修改引擎或 `llm_node.py`。

## Reference Docs / 参考文档

- `release/WORKFLOW_REFERENCE.md` — complete schema, scheduling semantics, edge cases / 完整 Schema、调度语义、边界情况
- `release/QUICKSTART.md` — 5-min quickstart + c2/c3/c4 demo workflows / 5 分钟入门 + 示范用例
- `release/README.md` — API reference, system requirements / API 参考、系统要求

## Architecture / 架构

```
JSON Workflow → Engine (DAG scheduler) → Nodes (execution)
                  │                       ├─ type: "llm" → llm_node.py (pure glue / 纯胶水)
                  │                       └─ type: "shell" → cmd /c <command>
```

**Engine / 引擎**：DAG scheduling, edge matching (h_e+g_e), data routing, retry/timeout, convergence.
**Engine does NOT / 引擎不管**：what nodes do internally. Nodes are black boxes following stdout JSON protocol.

**llm_node.py**：receive `--cmd` → read stdin context → spawn CLI → forward output to log → multi-strategy extract route+content → if route missing/invalid → correction retry → write stdout.

## Node Output Protocol / 节点输出协议 (NON-NEGOTIABLE / 不可协商)

Every node MUST output exactly one JSON on stdout:
```json
{"route":"<string>","content":"<string>"}
```
- `route` — non-empty → used for `exit_reason` edge matching. Empty → only matches `exit_reason: null`.
- `content` — arbitrary text, passed downstream via dataflows.
- Exit 0 = `complete`. Non-zero = `failed`. Killed = `timeout`.

## Provider Types / Provider 类型

| Type / 类型 | When / 场景 | Notes / 备注 |
|------|----------|---------|
| `"llm"` | Claude/opencode/nga calls | Command is CLI flags only. Prompt via stdin. `{{datarouter.X.content}}` / `{{metadata.*}}` auto-replaced. |
| `"shell"` | Scripts, echo, .bat | Wrapped in `cmd /c` (Win) or `sh -c` (Unix). |
| `"subprocess"` | Direct spawn | Avoid. Use `"shell"` instead. |

## LLM Command Template / LLM 命令模板

```json
"command": "claude --output-format stream-json --verbose --include-partial-messages --dangerously-skip-permissions"
```
- Prompt goes via stdin (not `-p`) → no command-line length limit / prompt 走 stdin 无长度限制
- `--dangerously-skip-permissions` → allow file Read/Write without interaction

## Templates / 模板

### Engine System Variables / 引擎系统变量

The engine renders these in prompt and command templates **before** sending to the node:

| Variable | Source | Example |
|----------|--------|---------|
| `{{metadata.run_count}}` | `NodeMetadata.run_count` | `1`, `2`, `3` — current execution round (1-based) |
| `{{metadata.timed_out}}` | `NodeMetadata.timed_out` | `true` / `false` — whether previous run timed out |
| `{{datarouter.<alias>.route}}` | `DataRouter.outputs[].route` | `complete`, `dispatch`, `again` — upstream node's route |
| `{{datarouter.<alias>.content}}` | `DataRouter.outputs[].content` | Upstream node's full content text |

- `<alias>` = source node ID (or `alias` field from dataflow).
- Validator rejects unknown fields: `metadata.foo` / `datarouter.X.foo` / any other `{{prefix.key}}` → error.
- At runtime, unsupported fields emit a `warn` log and pass through unchanged.

### Single LLM / 单节点
```json
{"nodes":[{"id":"ask","providers":[{"type":"llm","command":"claude --output-format stream-json --verbose --include-partial-messages --dangerously-skip-permissions","prompt":"Output ONLY: {\"route\":\"ok\",\"content\":\"...\"}","routes":["ok"]}],"process_timeout_secs":120}]}
```

### Chain / 链式 (A→B→C)
```json
"edges":[
  {"from":"a","to":"b","trigger":"any","event":"complete"},
  {"from":"b","to":"c","trigger":"any","event":"complete"}
]
```

### Fan-out / Fan-in / 扇出扇入
`trigger: "all"` on merge node waits for all upstream. Edges and dataflows are independent — both must be declared.

### Conditional Branch / 条件分支
`exit_reason` on edge exactly matches node output `route`. Null = matches any route.

### Directed Cycle / 有向环 (repeated operations / 重复操作)

```json
{
  "nodes": [
    {"id":"A","providers":[{"type":"llm","command":"claude --output-format stream-json --verbose --include-partial-messages --dangerously-skip-permissions","prompt":"Task (Round {{metadata.run_count}}/N). Continue → route='again'. Done → route='stop'. Output ONLY JSON.","routes":["again","stop"]}],"route_policy":{"type":"max_runs","max":N,"then_route":"stop"}},
    {"id":"B","providers":[{"type":"llm","command":"claude --output-format stream-json --verbose --include-partial-messages --dangerously-skip-permissions","prompt":"Process iteration. Output ONLY JSON with route='done'.","routes":["done"]}]}
  ],
  "edges": [
    {"from":"A","to":"B","event":"complete","exit_reason":"again"},
    {"from":"B","to":"A","event":"complete"},
    {"from":"A","to":"C","event":"complete","exit_reason":"stop"}
  ]
}
```

**Cycle rules / 环路规则**：
- All nodes equal — output route, engine matches edges / 所有节点平等，输出 route，引擎匹配边
- At least one node has `route_policy` (force route after N runs) or `threshold` → guarantees termination / 终止保证
- Validator requires every node reachable → an exit node (node with 0 outgoing edges). Cycle needs exit edge → signal node.
- `route_policy.max_runs=N`：node runs N-1 times, skipped on Nth trigger → saves one LLM call / 跑 N-1 次，第 N 次跳过省一次调用

## Edge Rules / 边规则

| Field / 字段 | Values / 值 | Meaning / 含义 |
|-------|---------|---------|
| `trigger` | `"any"` (default), `"all"` | `"all"` = wait for all incoming all-edges (fan-in) / 等所有入边 |
| `event` | `"complete"`, `"failed"`, `"timeout"` | Event type from node exit code / 节点退出码决定 |
| `exit_reason` | string or null | Exact match on node's `route`. null = any route / 匹配任意 |
| `threshold` | integer, default 1 | Fire after N matching events. Use for self-loops / 自环用 |

## Dataflow Rules / 数据流规则

- **Edges ≠ Dataflows**：edges schedule execution，dataflows route data。Both are independent graphs.
- **`alias`**：rename input key for downstream. Default key = source node ID.
- **Latest only**：in cycles，each run overwrites previous output.

## File Output / 文件输出

1. **Prompt instruction / Prompt 指令**：`"Write your review to review.md."` — Claude uses Write tool. No extra nodes.
2. **Shell node**：`{"type":"shell","command":"python scripts/write_report.py"}` — script writes from stdin context.

## Long Prompt Strategy / 长 Prompt 策略

Prompt > ~4KB：give Claude the file path. Claude reads via Read tool. Path is short; content never hits command line.
Prompt 超 4KB 时给 Claude 文件路径，Claude 用 Read 工具自读，内容不经过命令行。

## LLM Prompt Design / Prompt 设计

- **Always specify exact output format** / 必须明确输出格式：`Output ONLY: {"route":"again|stop","content":"..."}`
- **Always include `routes` list** / 必须有 routes 列表：`"routes": ["again", "stop"]`
- **Template variables / 模板变量**：`{{datarouter.X.content}}` (upstream data), `{{datarouter.X.route}}` (upstream route), `{{metadata.run_count}}` (cycle round), `{{metadata.timed_out}}` (previous timeout)

## MCP Integration / MCP 集成

```json
{"method":"run_workflow","params":{"workflow_json":"<JSON>","dashboard_url":"http://127.0.0.1:48080"},"id":1}
```
Returns `{run_id, dashboard_url, monitor_url}`。

## Validation / 校验 (MUST DO / 必须执行)

**After generating a workflow JSON, run the validator:**

```bash
nexus-cli --validate --workflow <file.json>
```

Or via MCP:
```json
{"method":"validate_workflow","params":{"workflow_json":"<JSON>"},"id":1}
```

**Regenerate if ANY error appears.** The validator catches:
- `UnknownMetadataField` — `{{metadata.xxx}}` with unrecognized field (valid: `run_count`, `timed_out`)
- `UnknownDatarouterField` — `{{datarouter.X.xxx}}` with unrecognized field (valid: `route`, `content`)
- `DatarouterRefWithoutDataflow` — `{{datarouter.X.*}}` but no dataflow `X → this_node`
- `UnrecognizedTemplate` — any `{{prefix.key}}` not matching `metadata.*` or `datarouter.*.*`

## Common Pitfalls / 常见问题

| Symptom / 现象 | Fix / 解法 |
|---------|------|
| Cycle never exits / 环路不退出 | Add `route_policy: {max_runs: N, then_route: "stop"}` |
| Fix/worker node never starts | Match `exit_reason` exactly to LLM output `route` |
| LLM route always empty / 始终为空 | Wrapper auto-corrects: missing/invalid route triggers retry. Still, ensure prompt asks for JSON and `routes` list is declared. |
| Downstream gets no data / 下游无数据 | Add `dataflows: [{from: X, to: Y}]` |
| `{{datarouter.X.content}}` not replaced / 未替换 | Check dataflows has `from: X, to: current_node` |
| Validator: unrecognized template / 模板未识别 | Only `metadata.*` and `datarouter.*.*` are supported |
| Validator: unknown metadata/datarouter field / 未知字段 | Only `metadata.{run_count,timed_out}` and `datarouter.X.{route,content}` |
| Validator: "exit not reachable" | Add exit signal node + exit edge from cycle |
| Prompt too long, claude hangs / 卡住 | Give file path instead of inline content / 给文件路径非内联 |
