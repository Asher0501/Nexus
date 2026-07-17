# Nexus Workflow JSON Generator / Nexus 工作流 JSON 生成器

Generate valid Nexus workflow JSON from user intent. Full schema in `WORKFLOW_REFERENCE.md`.

## 1. Quick patterns / 快速模式

Start here. Combine these building blocks to form any topology.

### Single node
```json
{"nodes":[{"id":"ask","providers":[{"type":"llm_sdk","model":"...","prompt":"...","routes":["ok"]}],"process_timeout_secs":120}],"edges":[],"dataflows":[]}
```

### Chain A → B → C
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
`route_policy` guarantees termination. Also available: `max_duration` (exit after N seconds cumulative).

### Error handling / 错误处理
```json
"edges":[
  {"from":"A","to":"on_success","trigger":"any","event":"complete"},
  {"from":"A","to":"on_failure","trigger":"any","event":"failed"}
]
```

## 2. Templates / 模板变量

Available in `command`, `prompt`, `url`, and `body` fields.

| Variable | Expands to |
|----------|-----------|
| `{{datarouter.X.content}}` | Upstream node X's output text |
| `{{datarouter.X.route}}` | Upstream node X's route value |
| `{{metadata.run_count}}` | Current execution round (1-based) |
| `{{metadata.timed_out}}` | `true` if previous run timed out |
| `{{node_dir}}` | Resolved scripts directory for this node |

- Shell mode (`type: "shell"`) auto-escapes substituted values.
- `{{node_dir}}` resolution: node-level `scripts_dir` > workflow-level > `NEXUS_SCRIPTS_DIR` env > exe-relative search > `./scripts`.

## 3. Providers / 提供器

| Type | Use case | Dependencies |
|------|----------|--------------|
| `llm_sdk` | Claude via Anthropic SDK (recommended) | `pip install anthropic`, API key |
| `llm` | Any LLM CLI (claude, opencode, nga...) | CLI on PATH |
| `http` | HTTP requests (GET/POST/PUT/DELETE) | None |
| `shell` | Scripts, pipes, redirects | None |
| `subprocess` | Direct spawn (no shell) | Avoid; use `shell` |

### llm vs llm_sdk

| | `llm` (CLI) | `llm_sdk` (SDK) |
|---|---|---|
| 谁调 API | CLI 二进制 | `llm_sdk.py` wrapper |
| 谁执行 tool_use | CLI 内置 Agent loop | wrapper 自己实现 tool loop |
| 内置工具 | CLI 自带 Read/Write/Edit/Bash | `read_file`, `write_file`, `execute_command`, `ask_human` (+ MCP servers) |
| 引擎感知 | 不关心 | 不关心 |

> CLI 是自带工具的 Agent；SDK 是裸 API client，tool loop + 工具由 wrapper 提供。两者可互换——改 `type` 即可。

### llm_sdk quick config
```json
{"type":"llm_sdk","model":"claude-sonnet-5-20251001","prompt":"...","routes":["ok","err"],"max_tokens":4096}
```
Credentials: `ANTHROPIC_API_KEY` → `ANTHROPIC_AUTH_TOKEN` → `~/.claude/settings.json`。

### llm quick config
```json
{"type":"llm","command":"claude -p \"{{prompt}}\" --output-format stream-json --verbose --include-partial-messages --dangerously-skip-permissions","prompt":"...","routes":["ok"]}
```

### scripts_dir
```json
{"scripts_dir":"./my_scripts","nodes":[
  {"id":"A","scripts_dir":"./a_scripts","providers":[...]},
  {"id":"B","providers":[...]}
]}
```
Node-level > workflow-level > `NEXUS_SCRIPTS_DIR` env > auto-search > `./scripts`.

## 4. 文件路径规则 / File path rules ⚠️

**最易踩坑。** Engine、Dashboard、CLI 的 CWD 各不相同，相对路径几乎必然出错。

| # | 规则 | 说明 |
|---|------|------|
| 1 | **始终用绝对路径** | 传给 LLM 的文件路径用绝对路径。不用 `./` 或 `../`。Windows 用正斜杠：`C:/Users/.../file.md` |
| 2 | **不要叫 LLM 探索目录** | Prompt 中不要写 "read ALL files in the directory"。LLM 逐个猜文件名，耗尽 20 轮 tool turn。给精确路径。 |
| 3 | **seed 节点用 hex 编码** | JSON payload 用 hex 编码避免 shell 引号冲突：`python -c __import__('sys').stdout.write(bytes.fromhex('HEX').decode())` |
| 4 | **scripts_dir 用项目根相对路径** | `scripts_dir: "./scripts"` 而非 `"../scripts"`。引擎自动向上搜索。 |
| 5 | **路径跨平台** | 不要硬编码平台路径。从 seed command 动态生成或让用户提供。 |

**生成流程**：用户说"审查文件 X" → seed 输出该文件的绝对路径 → dataflow 传 `{{datarouter.seed.content}}` → LLM 用 `read_file` 直接读。

## 5. Pattern library / 模式库

Real, validated workflows. Adapt by renaming nodes, swapping providers, adjusting prompts.

### P1: Code review loop
Review → fix → review cycle, `max_runs` exit.

```json
{"nodes":[
  {"id":"seed","providers":[{"type":"shell","command":"echo fn divide(a:i32,b:i32)->i32{a/b}"}],"process_timeout_secs":10},
  {"id":"review","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","system_prompt":"You are a senior code reviewer.","prompt":"Review this code. Output route 'needs_fix' or 'approved'.\n\n{{datarouter.seed.content}}\n{{datarouter.fixed_code.content}}","routes":["approved","needs_fix"],"max_tokens":4096}],"process_timeout_secs":600,"route_policy":{"type":"max_runs","max":3,"then_route":"approved"}},
  {"id":"fix","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","system_prompt":"You are a code fixer.","prompt":"Fix ALL issues. Output corrected code.\n\nIssues: {{datarouter.review.content}}","routes":["done"],"max_tokens":4096}],"process_timeout_secs":600},
  {"id":"report","providers":[{"type":"shell","command":"echo Final: {{datarouter.review.content}}"}],"process_timeout_secs":10}
],
"edges":[
  {"from":"seed","to":"review","trigger":"any","event":"complete"},
  {"from":"review","to":"fix","trigger":"any","event":"complete","exit_reason":"needs_fix"},
  {"from":"fix","to":"review","trigger":"any","event":"complete"},
  {"from":"review","to":"report","trigger":"any","event":"complete","exit_reason":"approved"}
],
"dataflows":[
  {"from":"seed","to":"review"},{"from":"review","to":"fix"},
  {"from":"fix","to":"review","alias":"fixed_code"},{"from":"review","to":"report"}
]}
```

### P2: Fan-out parallel analysis
Two analyzers in parallel, merge with `trigger:"all"`.

```json
{"nodes":[
  {"id":"seed","providers":[{"type":"shell","command":"echo Analyze this."}],"process_timeout_secs":10},
  {"id":"audit_a","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","prompt":"Audit A. Output route 'ok'.\nContext: {{datarouter.seed.content}}","routes":["ok"],"max_tokens":2048}],"process_timeout_secs":300},
  {"id":"audit_b","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","prompt":"Audit B. Output route 'ok'.\nContext: {{datarouter.seed.content}}","routes":["ok"],"max_tokens":2048}],"process_timeout_secs":300},
  {"id":"merge","providers":[{"type":"shell","command":"echo Merged"}],"process_timeout_secs":10}
],
"edges":[
  {"from":"seed","to":"audit_a","trigger":"any","event":"complete"},
  {"from":"seed","to":"audit_b","trigger":"any","event":"complete"},
  {"from":"audit_a","to":"merge","trigger":"all","event":"complete"},
  {"from":"audit_b","to":"merge","trigger":"all","event":"complete"}
],
"dataflows":[
  {"from":"seed","to":"audit_a"},{"from":"seed","to":"audit_b"},
  {"from":"audit_a","to":"merge"},{"from":"audit_b","to":"merge"}
]}
```

### P3: Conditional branch + error handling
```json
{"nodes":[
  {"id":"validator","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","prompt":"Validate. Output route 'pass' or 'fail'.\nInput: {{datarouter.seed.content}}","routes":["pass","fail"],"max_tokens":1024}],"process_timeout_secs":120},
  {"id":"on_pass","providers":[{"type":"shell","command":"echo PASSED"}],"process_timeout_secs":10},
  {"id":"on_fail","providers":[{"type":"shell","command":"echo FAILED"}],"process_timeout_secs":10}
],
"edges":[
  {"from":"validator","to":"on_pass","trigger":"any","event":"complete","exit_reason":"pass"},
  {"from":"validator","to":"on_fail","trigger":"any","event":"complete","exit_reason":"fail"}
]}
```
Add `"event":"failed"` edges to catch runtime errors separately.

### P4: HTTP orchestration chain
```json
{"nodes":[
  {"id":"fetch","providers":[{"type":"http","url":"https://jsonplaceholder.typicode.com/users/1","method":"GET"}],"process_timeout_secs":15},
  {"id":"post","providers":[{"type":"http","url":"https://jsonplaceholder.typicode.com/posts","method":"POST","body":"{\"title\":\"test\",\"body\":\"{{datarouter.fetch.content}}\"}"}],"process_timeout_secs":15}
],
"edges":[{"from":"fetch","to":"post","trigger":"any","event":"complete"}],
"dataflows":[{"from":"fetch","to":"post"}]
}
```

### P5: Retry loop with max_duration exit
```json
{"id":"task","route_policy":{"type":"max_duration","max_secs":600,"then_route":"timeout"}, ...}
"edges":[
  {"from":"task","to":"retry","trigger":"any","event":"complete","exit_reason":"err"},
  {"from":"retry","to":"task","trigger":"any","event":"complete"},
  {"from":"task","to":"on_ok","trigger":"any","event":"complete","exit_reason":"ok"},
  {"from":"task","to":"on_timeout","trigger":"any","event":"complete","exit_reason":"timeout"}
]
```

## 6. LLM Prompt 编写

### 输出格式：必须包含 route JSON
LLM 节点的最终 stdout 必须是单行 JSON `{"route":"...","content":"..."}`。Prompt 末尾总是加：

```
Output ONLY a single-line JSON:
{"route":"approved|needs_fix","content":"your findings"}
```

### Work Phase / Output Phase 分离
LLM 倾向于跳过工作直接输出路由 JSON。拆成两步：

```
WORK PHASE (use tools, do NOT output JSON yet):
1. Read the file.
2. Analyze all dimensions.

ONLY AFTER all work is complete, output routing JSON:
{"route":"needs_fix","content":"summary"}
```

### 文件输出 + stdout 路由
详细内容写文件，stdout 只放路由：

```
1. Write detailed analysis to review.md using write_file.
2. Output ONLY: {"route":"approved","content":"3 issues found, see review.md"}
```

### CLI 命令最佳实践
- 去掉 `-p "{{prompt}}"`，prompt 走 stdin（避免 Windows 命令行长度限制）
- `--output-format stream-json`（实时 chunk）
- `--dangerously-skip-permissions`（非交互模式）

## 7. Validation / 校验

**Always validate after generating.** Fix all errors before execution.

```bash
nexus-cli run <file.json> --validate-only
```

- **Errors block execution:** `DuplicateNodeId`, `NoEntryNode`, `DatarouterRefWithoutDataflow`, `UnrecognizedTemplate`.
- **Warnings are advisory:** `DataflowWithoutSchedulingEdge`, `UnmatchedRoute`.

## 8. Pitfalls / 常见陷阱

| Symptom | Likely cause |
|---------|-------------|
| Downstream gets empty data | Missing `dataflows` or dataflow exists without scheduling edge |
| Cycle never exits | Missing `route_policy` or `threshold` |
| Node never starts | `exit_reason` doesn't exactly match node's `route` output |
| `{{datarouter.X.content}}` empty | Missing `dataflows: [{from:"X",to:current_node}]` |
| `{{node_dir}}` not rendered | `scripts_dir` not configured |
| Validator: UnrecognizedTemplate | Only `metadata.*`, `datarouter.*.*`, `node_dir` are valid prefixes |
| llm_sdk: API key not found | Set `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, or `api_key_env` |
| llm_sdk: wrong model | Use full model ID: `claude-sonnet-5-20251001` |
| llm_sdk: surrogates not allowed | Engine auto-filters. If still occurs, check input encoding. |
| Dashboard: View empty | POST body missing `name`/`definition` wrapper |
| Dashboard: UTF-8 broken | curl damages CJK. Use Python `urllib.request` with `charset=utf-8`. |
| Dashboard: double path | `scripts_dir: "./release/scripts"` → `release/release/scripts/`. Use `"./scripts"`. |
| Node fails after many tool calls | LLM wasted tokens on path-searching. See §4. |

## 9. Dashboard Integration

Dashboard: `http://127.0.0.1:48080`. Use `{"name":"...","definition":{...}}` wrapper format.

```python
import json, urllib.request
with open('workflow.json', 'r', encoding='utf-8') as f:
    definition = json.load(f)
body = json.dumps({'name': 'My Workflow', 'definition': definition}, ensure_ascii=False).encode('utf-8')
req = urllib.request.Request('http://127.0.0.1:48080/api/workflows', data=body, method='POST')
req.add_header('Content-Type', 'application/json; charset=utf-8')
print(urllib.request.urlopen(req).read().decode('utf-8'))
```

| Method | Path | Description |
|--------|------|-------------|
| GET/POST | `/api/workflows` | List / Create |
| GET/PUT/DELETE | `/api/workflows/{id}` | Read / Update / Delete |
| GET | `/api/workflows/{id}/graph` | DAG topology |
| POST | `/api/workflows/{id}/run` | Trigger execution |
| GET | `/api/runs` | List runs |
| GET | `/api/runs/{id}` | Run details |
| POST | `/api/runs/{id}/stop` | Stop running workflow |
| WS | `/ws/runs/{run_id}` | Live events |

## 10. MCP Integration

```json
{"method":"run_workflow","params":{"workflow_json":"<JSON>","dashboard_url":"http://127.0.0.1:48080"},"id":1}
```
Returns `{run_id, dashboard_url, monitor_url}`.
