# Nexus Workflow JSON Generator / Nexus 工作流 JSON 生成器

Generate valid Nexus workflow JSON from user intent. Full schema and semantics are in the reference; this skill covers **what to do** and **what goes wrong**.

Reference: `release/WORKFLOW_REFERENCE.md` (schema & semantics), `release/QUICKSTART.md` (5-min tutorial), `release/NEXUS_VS_LANGGRAPH.md` (LangGraph comparison).

## Pattern library / 模式库

These are **real, validated workflows** that demonstrate common topologies. When a user requests a workflow, find the closest pattern below and adapt it — rename nodes, swap providers, adjust prompts, add/remove branches. The LLM understands what is structural (edges, dataflows, route names) and what is business logic (prompts, commands, URLs).

### P1: Code review loop

Self-review loop with LLM → fix → review cycle, automatic exit after N rounds.

```json
{"nodes":[
  {"id":"seed","providers":[{"type":"shell","command":"echo fn divide(a:i32,b:i32)->i32{a/b}"}],"process_timeout_secs":10},
  {"id":"review","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","system_prompt":"You are a senior code reviewer. Find bugs, security issues, and design flaws.","prompt":"Review this code. If you find issues reply with route 'needs_fix' and explain the problem. If the code looks good reply with route 'approved'.\n\nCode to review:\n{{datarouter.seed.content}}\n{{datarouter.fixed_code.content}}","routes":["approved","needs_fix"],"max_tokens":4096}],"process_timeout_secs":600,"route_policy":{"type":"max_runs","max":3,"then_route":"approved"},"returns":["approved","needs_fix"]},
  {"id":"fix","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","system_prompt":"You are a code fixer. Apply the suggested fixes precisely.","prompt":"Fix the issues identified below. Output ONLY the corrected code.\n\nIssues: {{datarouter.review.content}}","routes":["done"],"max_tokens":4096}],"process_timeout_secs":600},
  {"id":"report","providers":[{"type":"shell","command":"echo Final review: {{datarouter.review.content}}"}],"process_timeout_secs":10}
],
"edges":[
  {"from":"seed","to":"review","trigger":"any","event":"complete"},
  {"from":"review","to":"fix","trigger":"any","event":"complete","exit_reason":"needs_fix"},
  {"from":"fix","to":"review","trigger":"any","event":"complete"},
  {"from":"review","to":"report","trigger":"any","event":"complete","exit_reason":"approved"}
],
"dataflows":[
  {"from":"seed","to":"review"},
  {"from":"review","to":"fix"},
  {"from":"fix","to":"review","alias":"fixed_code"},
  {"from":"review","to":"report"}
]}
```
Adapt by replacing the seed command, review focus, model, and max_runs.

### P2: Fan-out parallel analysis

Two LLM analyzers run in parallel, results merge into one report.

```json
{"nodes":[
  {"id":"seed","providers":[{"type":"shell","command":"echo Analyze this codebase for issues."}],"process_timeout_secs":10},
  {"id":"security_audit","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","prompt":"Audit for security vulnerabilities. Output route 'ok' with findings.\\nContext: {{datarouter.seed.content}}","routes":["ok"],"max_tokens":2048}],"process_timeout_secs":300},
  {"id":"perf_audit","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","prompt":"Audit for performance issues. Output route 'ok' with findings.\\nContext: {{datarouter.seed.content}}","routes":["ok"],"max_tokens":2048}],"process_timeout_secs":300},
  {"id":"merge","providers":[{"type":"shell","command":"python -c \"import json,sys; d=json.load(sys.stdin); i=d['inputs']; print(json.dumps({'route':'ok','content':f'SECURITY:\\n{i.get(chr(115)+chr(101)+chr(99)+chr(117)+chr(114)+chr(105)+chr(116)+chr(121)+chr(95)+chr(97)+chr(117)+chr(100)+chr(105)+chr(116),chr(63))}\\n\\nPERF:\\n{i.get(chr(112)+chr(101)+chr(114)+chr(102)+chr(95)+chr(97)+chr(117)+chr(100)+chr(105)+chr(116),chr(63))}'}))\""}],"process_timeout_secs":10}
],
"edges":[
  {"from":"seed","to":"security_audit","trigger":"any","event":"complete"},
  {"from":"seed","to":"perf_audit","trigger":"any","event":"complete"},
  {"from":"security_audit","to":"merge","trigger":"all","event":"complete"},
  {"from":"perf_audit","to":"merge","trigger":"all","event":"complete"}
],
"dataflows":[
  {"from":"seed","to":"security_audit"},
  {"from":"seed","to":"perf_audit"},
  {"from":"security_audit","to":"merge"},
  {"from":"perf_audit","to":"merge"}
]}
```
The `trigger:"all"` on merge ensures both audits complete before merging.

### P3: Conditional branch + error handling

Decision node routes to success or failure path. Failed edge captures errors.

```json
{"nodes":[
  {"id":"validator","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","prompt":"Validate the input. Return route 'pass' or 'fail' with explanation.\\nInput: {{datarouter.seed.content}}","routes":["pass","fail"],"max_tokens":1024}],"process_timeout_secs":120},
  {"id":"on_pass","providers":[{"type":"shell","command":"echo VALIDATION PASSED"}],"process_timeout_secs":10},
  {"id":"on_fail","providers":[{"type":"shell","command":"echo VALIDATION FAILED"}],"process_timeout_secs":10}
],
"edges":[
  {"from":"validator","to":"on_pass","trigger":"any","event":"complete","exit_reason":"pass"},
  {"from":"validator","to":"on_fail","trigger":"any","event":"complete","exit_reason":"fail"}
]
}
```
Add `"event":"failed"` edges to catch runtime errors separately from business-logic rejection.

### P4: HTTP orchestration chain

Chain HTTP calls, each feeding data into the next.

```json
{"nodes":[
  {"id":"fetch_user","providers":[{"type":"http","url":"https://jsonplaceholder.typicode.com/users/1","method":"GET"}],"process_timeout_secs":15},
  {"id":"create_post","providers":[{"type":"http","url":"https://jsonplaceholder.typicode.com/posts","method":"POST","body":"{\"title\":\"nexus-test\",\"body\":\"user data: {{datarouter.fetch_user.content}}\",\"userId\":1}"}],"process_timeout_secs":15},
  {"id":"verify","providers":[{"type":"shell","command":"python -c \"import json,sys; d=json.load(sys.stdin); resp=d['inputs'].get('create_post',''); ok='id' in resp; print(json.dumps({'route':'ok' if ok else 'err','content':'post created' if ok else 'failed'}))\""}],"process_timeout_secs":5}
],
"edges":[
  {"from":"fetch_user","to":"create_post","trigger":"any","event":"complete"},
  {"from":"create_post","to":"verify","trigger":"any","event":"complete"}
],
"dataflows":[
  {"from":"fetch_user","to":"create_post"},
  {"from":"create_post","to":"verify"}
]}
```

### P5: Retry loop with max_duration exit

LLM task with retry on error, exits after accumulated time exceeds threshold.

```json
{"nodes":[
  {"id":"task","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","prompt":"Complete the assigned task. Output route 'ok' on success, 'err' on failure.\\nTask: {{datarouter.feedback.content}}","routes":["ok","err"],"max_tokens":2048}],"process_timeout_secs":300,"route_policy":{"type":"max_duration","max_secs":600,"then_route":"timeout"},"returns":["ok","err"]},
  {"id":"retry_feedback","providers":[{"type":"shell","command":"echo Previous attempt failed. Try a different approach."}],"process_timeout_secs":5},
  {"id":"on_ok","providers":[{"type":"shell","command":"echo Task completed successfully"}],"process_timeout_secs":5},
  {"id":"on_timeout","providers":[{"type":"shell","command":"echo Task timed out after max duration"}],"process_timeout_secs":5}
],
"edges":[
  {"from":"task","to":"retry_feedback","trigger":"any","event":"complete","exit_reason":"err"},
  {"from":"retry_feedback","to":"task","trigger":"any","event":"complete"},
  {"from":"task","to":"on_ok","trigger":"any","event":"complete","exit_reason":"ok"},
  {"from":"task","to":"on_timeout","trigger":"any","event":"complete","exit_reason":"timeout"}
],
"dataflows":[
  {"from":"retry_feedback","to":"task","alias":"feedback"}
]}
```

### When no pattern fits

Compose from the Quick Patterns below. The LLM can mix patterns — e.g. fan-out + conditional branch + error handling in one DAG.

## Quick patterns / 快速模式

### Single node / 单节点

```json
{"nodes":[{"id":"ask","providers":[{"type":"llm_sdk","model":"...","prompt":"...",
"routes":["ok"]}],"process_timeout_secs":120}],"edges":[],"dataflows":[]}
```

### Chain A → B → C / 链式

```json
"edges":[
  {"from":"A","to":"B","trigger":"any","event":"complete"},
  {"from":"B","to":"C","trigger":"any","event":"complete"}
]
```
If B needs A's output, also add `"dataflows":[{"from":"A","to":"B"},{"from":"B","to":"C"}]`.

### Fan-out / 扇出 (A → B, C in parallel)

```json
"edges":[
  {"from":"A","to":"B","trigger":"any","event":"complete"},
  {"from":"A","to":"C","trigger":"any","event":"complete"}
]
```

### Fan-in / 扇入 (A,B → merge)

`trigger: "all"` on the merge node's incoming edges. All upstream must complete before merge fires.

### Branch / 分支 (review → approved? deploy : fix)

```json
"edges":[
  {"from":"review","to":"deploy","trigger":"any","event":"complete","exit_reason":"approved"},
  {"from":"review","to":"fix",   "trigger":"any","event":"complete","exit_reason":"rejected"}
]
```

### Cycle / 循环 (review → fix → review → ... → exit)

```json
{"id":"review","route_policy":{"type":"max_runs","max":3,"then_route":"approved"}, ...}
"edges":[
  {"from":"review","to":"fix",   "trigger":"any","event":"complete","exit_reason":"rejected"},
  {"from":"fix",   "to":"review","trigger":"any","event":"complete"},
  {"from":"review","to":"retro", "trigger":"any","event":"complete","exit_reason":"approved"}
]
```
`route_policy` guarantees termination: after N runs, engine overrides the route to exit the cycle.

### Error handling / 错误处理 (A → success handler / error handler)

```json
"edges":[
  {"from":"A","to":"on_success","trigger":"any","event":"complete"},
  {"from":"A","to":"on_failure","trigger":"any","event":"failed"}
]
```

## Templates / 模板变量

Available in `command` and `prompt` fields. Engine renders them before spawning the node.

| Variable | Expands to |
|----------|-----------|
| `{{datarouter.X.content}}` | Upstream node X's output text |
| `{{datarouter.X.route}}` | Upstream node X's route value |
| `{{metadata.run_count}}` | Current execution round (1-based) |
| `{{metadata.timed_out}}` | `true` if previous run timed out |
| `{{node_dir}}` | Resolved scripts directory for this node |

- Shell mode (`type: "shell"`) auto-escapes substituted values.
- `{{node_dir}}` path resolution: node-level `scripts_dir` > workflow-level > `NEXUS_SCRIPTS_DIR` env > exe-relative search > `./scripts`.

## Providers / 提供器

| Type | Use case | Dependencies |
|------|----------|--------------|
| `llm_sdk` | Claude via Anthropic SDK (recommended) | `pip install anthropic`, API key |
| `llm` | Any LLM CLI (claude, opencode, nga...) | CLI on PATH |
| `shell` | Scripts, pipes, redirects | None |
| `subprocess` | Direct spawn (no shell) | Avoid; use `shell` |

### llm vs llm_sdk: 谁执行 tool 调用？

| | `llm` (CLI) | `llm_sdk` (SDK) |
|---|---|---|
| 谁调 API | CLI 二进制 | `llm_sdk.py` wrapper |
| 谁执行 tool_use | CLI 内置 Agent loop，自动 | **必须自己在 wrapper 里写 tool loop** |
| 文件操作 | CLI 自带 Read/Write/Edit 工具 | wrapper 用 Python `open()` 等实现 |
| 引擎感知 | 不关心，只拿最终 stdout | 不关心，只拿最终 stdout |

> CLI 是自带工具的 Agent；SDK 是裸 API client，tool loop 在 `llm_sdk.py` 里手动实现。

### llm_sdk quick config / SDK 快速配置

```json
{"type":"llm_sdk","model":"claude-sonnet-5-20251001","prompt":"...","routes":["ok","err"],"max_tokens":4096}
```

Credentials auto-detected: `ANTHROPIC_API_KEY` → `ANTHROPIC_AUTH_TOKEN` → `~/.claude/settings.json` → `api_key_env` field.
`ANTHROPIC_BASE_URL` is auto-read for non-Anthropic endpoints (DeepSeek etc.).

### llm quick config / CLI 快速配置

```json
{"type":"llm","command":"claude -p \"{{prompt}}\" --output-format stream-json --verbose --include-partial-messages --dangerously-skip-permissions","prompt":"...","routes":["ok"]}
```

### scripts_dir / 脚本目录

```json
{"scripts_dir":"./my_scripts","nodes":[
  {"id":"A","scripts_dir":"./a_scripts","providers":[...]},
  {"id":"B","providers":[...]}
]}
```
Node-level overrides workflow-level, which falls back to env → auto-search → `./scripts`.

## Validation / 校验

**Always validate after generating JSON.** Fix all errors before execution.

```bash
nexus-cli --validate --workflow <file.json>
# or via MCP:
{"method":"validate_workflow","params":{"workflow_json":"<JSON>"},"id":1}
```

- **Errors block execution.** Common: `DuplicateNodeId`, `NoEntryNode`, `DatarouterRefWithoutDataflow`, `UnrecognizedTemplate`.
- **Warnings are advisory.** Common: `DataflowWithoutSchedulingEdge` (B may run before A produces data), `UnmatchedRoute` (LLM route has no matching edge).

## Pitfalls / 常见陷阱

| Symptom | Likely cause |
|---------|-------------|
| Downstream gets empty data | Missing `dataflows` declaration, OR dataflow exists but no scheduling edge (B ran before A) |
| Cycle never exits | Missing `route_policy` or `threshold` on the exit path |
| Node that should start never does | `exit_reason` on edge doesn't exactly match node's `route` output |
| `{{datarouter.X.content}}` not rendered | Missing `dataflows: [{from:"X",to:current_node}]` |
| `{{node_dir}}` not rendered | `scripts_dir` not configured; check fallback chain |
| Validator: UnrecognizedTemplate | Template uses `{{prefix.key}}` — only `metadata.*`, `datarouter.*.*`, and `node_dir` are valid |
| llm_sdk: "API key not found" | Set `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN`, or configure `api_key_env` |
| llm_sdk: "401 invalid x-api-key" | `ANTHROPIC_BASE_URL` not set (needed for non-Anthropic endpoints) |
| llm_sdk: "anthropic package not installed" | `pip install anthropic` on target machine |
| llm_sdk: wrong model | Use the provider's model name: `deepseek-v4-pro[1m]` for DeepSeek, `claude-sonnet-5-...` for Anthropic |
| Dashboard: `release/release/` double path | `scripts_dir` is relative to CWD. Dashboard runs from `release/`, so `scripts_dir: "./release/scripts"` resolves to `release/release/scripts/`. Use `scripts_dir: "./scripts"` and let scripts auto-detect CWD. |
| llm_sdk: node fails after many tool calls | LLM wasted tokens on path-searching or file-discovery before reaching its task. ① Pass **absolute paths** (never relative). ② Don't tell LLM to "read ALL related documents" or "explore the directory" — it tries every path variant and exhausts the 20-turn tool budget. Give one exact path. ③ Set `max_tokens` high enough for the actual task output, not the search preamble. |
| llm_sdk: file not found at relative path | `llm_sdk.py` CWD is the engine process CWD, which varies by deployment (Dashboard, CLI, MCP). Always resolve paths to **absolute** before passing via dataflow. Relative paths are fragile. |
| llm_sdk: `UnicodeEncodeError: 'charmap' codec can't encode character` | Python stdout on Windows uses the system code page (cp1252), not UTF-8. Output with Chinese characters fails to encode. The wrapper's `write_output()` uses `ensure_ascii=True` — non-ASCII is escaped as `\uXXXX`. `serde_json` on the engine side decodes it back correctly. |

## Dashboard / 加载到 Dashboard

Dashboard 运行在 `http://127.0.0.1:48080`。工作流通过 REST API 加载和执行。

### 加载工作流

**API 格式要求（关键）：** Dashboard 期望 `{"name":"...", "definition":{...}}` 包装格式，NOT 裸 JSON。

```bash
# ❌ 错误：裸 JSON → definition 字段缺失 → 存入空对象 {}
curl -X POST http://127.0.0.1:48080/api/workflows -H "Content-Type: application/json" \
  -d @workflow.json

# ✅ 正确：包装格式
curl -X POST http://127.0.0.1:48080/api/workflows -H "Content-Type: application/json" \
  -d '{"name":"My Workflow","definition":{...}}'
```

**UTF-8 编码注意：** 当 workflow JSON 含中文、emoji 等特殊字符时，curl/bash 可能损坏编码导致 `invalid unicode code point` 错误。此时用 Python 直接 POST：

```python
import json, urllib.request
with open('workflow.json', 'r', encoding='utf-8') as f:
    definition = json.load(f)
body = json.dumps({'name': 'My Workflow', 'definition': definition}, ensure_ascii=False).encode('utf-8')
req = urllib.request.Request('http://127.0.0.1:48080/api/workflows', data=body, method='POST')
req.add_header('Content-Type', 'application/json; charset=utf-8')
resp = urllib.request.urlopen(req)
print(resp.read().decode('utf-8'))  # → {"id":"...","status":"created"}
```

### API 端点

| 方法 | 路径 | 说明 |
|------|------|------|
| `GET` | `/api/workflows` | 列出所有工作流 |
| `POST` | `/api/workflows` | 创建工作流 `{"name":"...","definition":{...}}` |
| `GET` | `/api/workflows/{id}` | 获取工作流详情（含解析后的 definition） |
| `PUT` | `/api/workflows/{id}` | 更新工作流 |
| `DELETE` | `/api/workflows/{id}` | 删除工作流 |
| `POST` | `/api/workflows/{id}/run` | 触发运行 → 返回 `{run_id, dashboard_url}` |
| `GET` | `/api/runs` | 列出所有运行记录 |
| `GET` | `/api/runs/{id}` | 获取运行详情 |
| `WS` | `/ws/runs/{run_id}` | WebSocket 实时状态推送 |

### 运行工作流

```bash
# 通过 API 触发
curl -X POST http://127.0.0.1:48080/api/workflows/{id}/run
# → {"run_id":"...","dashboard_url":"http://127.0.0.1:48080","monitor_url":"ws://..."}
```

### Pitfalls / 加载陷阱

| Symptom | Likely cause |
|---------|-------------|
| Dashboard View 显示空 | POST body 缺少 `name`/`definition` 包装，直接传了裸 JSON |
| `invalid unicode code point` | curl/bash 损坏了含中文/emoji 的 JSON；改用 Python urllib POST |
| `definition` 存为字符串 | `definition` 字段可以是 object 或 string，Dashboard 内部 auto-serialize。两种都支持 |
| 端口占用 | Dashboard 已在运行中，直接调用 API 即可，无需重启 |

## MCP / MCP 集成

```json
{"method":"run_workflow","params":{"workflow_json":"<JSON>","dashboard_url":"http://127.0.0.1:48080"},"id":1}
```
Returns `{run_id, dashboard_url, monitor_url}`.
