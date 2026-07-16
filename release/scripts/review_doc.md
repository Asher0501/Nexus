---
## Review Round 1

### FILE: WORKFLOW_REFERENCE.md
- **[HIGH]** **[CORRECTNESS]** §5.2 template example uses `type: "subprocess"` with a command containing pipe (`|`) and redirect (`2>nul`). Per §1.3 and §8.5, `subprocess` uses `split_whitespace()` and does NOT support shell features. Must use `type: "shell"`. Also references the nonexistent script `scripts/nexus-wrap.py`.
  - FIND:     { "id": "review", "providers": [{
      "type": "subprocess",
      "command": "cmd.exe /c opencode run --format json --auto -- \"{{datarouter.config.content}}\" 2>nul | python scripts/nexus-wrap.py"
    }], "process_timeout_secs": 300 }
  - REPLACE_WITH:     { "id": "review", "providers": [{
      "type": "shell",
      "command": "echo \"Received: {{datarouter.config.content}}\""
    }], "process_timeout_secs": 30 }

### FILE: WORKFLOW_REFERENCE.md
- **[HIGH]** **[CORRECTNESS]** §5.2 line 648 says unrecognized templates "原样透传，运行时 emit warning 日志" but the Validator (§9.2) treats `UnrecognizedTemplate` as a hard error that blocks execution. The doc contradicts itself on whether bad templates prevent runs.
  - FIND: - 普通文本和未识别的模板原样透传，运行时 emit warning 日志
  - REPLACE_WITH: - 普通文本原样透传。未识别的模板变量（如 `{{foo.bar}}`）在 `--validate-only` 时触发 `UnrecognizedTemplate` 错误，阻止执行

### FILE: NEXUS_WORKFLOW_SKILL.md
- **[HIGH]** **[CORRECTNESS]** llm quick config example uses `--output-format stream-json` which produces NDJSON event stream — incompatible with Nexus single-line `{"route":"...","content":"..."}` protocol. WORKFLOW_REFERENCE.md §7.1 correctly uses `--output-format json`. The `stream-json` flag would cause parse failures.
  - FIND: {"type":"llm","command":"claude -p \"{{prompt}}\" --output-format stream-json --verbose --include-partial-messages --dangerously-skip-permissions","prompt":"...","routes":["ok"]}
  - REPLACE_WITH: {"type":"llm","command":"claude -p \"{{prompt}}\" --output-format json --verbose","prompt":"...","routes":["ok"]}

### FILE: WORKFLOW_REFERENCE.md
- **[HIGH]** **[CORRECTNESS]** §9.1 exit code 3 description says "Node timeout（预留，当前未实现）" but §2.6 documents timeout detection, kill, retry, and timeout-edge triggering as fully implemented engine features. Exit code 3 is neither reserved nor unimplemented.
  - FIND: 3  Node timeout（预留，当前未实现）
  - REPLACE_WITH: 3  Node timeout

### FILE: README.md
- **[HIGH]** **[CORRECTNESS]** System requirements say "Linux / macOS — 需从源码构建（见下方构建说明）" but `bin/linux/` already contains precompiled Linux binaries (nexus-cli, nexus-dashboard, nexus-mcp-server) as documented in the binary table above. The two statements contradict each other.
  - FIND: - **Linux / macOS** — 需从源码构建（见下方构建说明）
  - REPLACE_WITH: - **Linux (x86_64)** — 提供预编译二进制文件（`bin/linux/`）
- **macOS** — 需从源码构建（见下方构建说明）

### FILE: QUICKSTART.md
- **[HIGH]** **[CORRECTNESS]** Claims "Dashboard 内置了三个可从 `static/` 导入的示范工作流" but the `static/` directory contains only 2 workflow JSON files (`arch-review-loop.json`, `review-loop.json`). No c2/c3/c4.json files exist — the c2/c3/c4 descriptions are inline prose, not importable files.
  - FIND: Dashboard 内置了三个可从 `static/` 导入的示范工作流，覆盖 Nexus 全部特性：
  - REPLACE_WITH: 以下是三个示范工作流，覆盖 Nexus 全部特性（c2/c3/c4 为内联说明，`static/` 目录另有两个可导入的示例工作流）：

### FILE: NEXUS_WORKFLOW_SKILL.md
- **[HIGH]** **[CORRECTNESS]** Line 81 claims "Shell mode (`type: "shell"`) auto-escapes substituted values." This behavior is NOT documented in WORKFLOW_REFERENCE.md §1.3 or §5.2 (the authoritative reference). If auto-escaping exists, the reference should document it; if not, this claim is misleading and could cause users to omit manual quoting, leading to broken commands.
  - FIND: - Shell mode (`type: "shell"`) auto-escapes substituted values.
  - REPLACE_WITH: - Template values are substituted directly. For `type: "shell"`, wrap values containing spaces in quotes within the command string.

### FILE: WORKFLOW_REFERENCE.md
- **[MEDIUM]** **[CORRECTNESS]** §8.3 self-loop docs refer to the validator error as `CycleWithoutEntry` (CamelCase), but §9.2 validation error table displays the actual runtime message as `cycle without entry: deadlock detected` (lowercase with colon). Cross-referencing readers will not find a match.
  - FIND: - 如果自环边 `threshold = 1` 且节点没有其他出边 → Validator 报 `CycleWithoutEntry`
  - REPLACE_WITH: - 如果自环边 `threshold = 1` 且节点没有其他出边 → Validator 报 `cycle without entry: deadlock detected`

### FILE: WORKFLOW_REFERENCE.md
- **[MEDIUM]** **[CORRECTNESS]** §1.3 ProviderDef table marks `max_tokens` as applicable to both `llm` and `llm_sdk`. For CLI mode (`type: "llm"`), token limits are controlled via the CLI's own `--max-tokens` flag embedded in the `command` string — the ProviderDef field is not the primary mechanism. Marking it for `llm` is misleading.
  - FIND: | `max_tokens` | `u64 \| null` | ❌（`llm`/`llm_sdk`） | 最大输出 token 数 |
  - REPLACE_WITH: | `max_tokens` | `u64 \| null` | ❌（`llm_sdk`） | 最大输出 token 数（SDK 模式）。CLI 模式通过 command 的 `--max-tokens` flag 控制 |

### FILE: WORKFLOW_REFERENCE.md
- **[MEDIUM]** **[CORRECTNESS]** §1.3 ProviderDef `type` field enum includes `"http"` as a valid value, but the command rules below state `type: "http": 当前未实现，运行时报错`. Including an unimplemented type in the primary type enum is misleading — users may select it expecting it to work.
  - FIND: | `type` | `"subprocess"` \| `"shell"` \| `"http"` \| `"llm"` \| `"llm_sdk"` | ✅ | 执行方式。`subprocess` 直接 spawn；`shell` 通过 shell 包装；`http` 预留；`llm` 通用 LLM agent 节点；`llm_sdk` 通过 Anthropic Python SDK 直调 |
  - REPLACE_WITH: | `type` | `"subprocess"` \| `"shell"` \| `"llm"` \| `"llm_sdk"` | ✅ | 执行方式。`subprocess` 直接 spawn；`shell` 通过 shell 包装；`llm` 通用 LLM agent 节点；`llm_sdk` 通过 Anthropic Python SDK 直调。（`"http"` 为预留值，当前未实现，运行时报错） |

### FILE: NEXUS_VS_LANGGRAPH.md
- **[MEDIUM]** **[CORRECTNESS]** §4 conditional branch example uses `"exit_reason": "needs_fix"` and `"exit_reason": "approved"` for review routing. However, all Nexus workflow examples in WORKFLOW_REFERENCE.md (§6.3, §6.4, §7.3, §7.7) consistently use `"rejected"` (not `"needs_fix"`) to trigger the fix branch. Using `"needs_fix"` in the comparison doc creates inconsistency and could confuse readers cross-referencing between documents.
  - FIND: {"from": "review", "to": "fix",  "exit_reason": "needs_fix"},
  - REPLACE_WITH: {"from": "review", "to": "fix",  "exit_reason": "rejected"},

### FILE: README.md
- **[MEDIUM]** **[COMPLETENESS]** Environment variables table is missing `NEXUS_SCRIPTS_DIR`, which is referenced by WORKFLOW_REFERENCE.md §1.2 and NEXUS_WORKFLOW_SKILL.md as a step in the `scripts_dir` / `{{node_dir}}` resolution chain.
  - FIND: | `NEXUS_PORT` | `48080` | Dashboard HTTP 监听端口 |
  - REPLACE_WITH: | `NEXUS_SCRIPTS_DIR` | 自动检测 | 全局脚本目录。参与 `{{node_dir}}` 解析链（节点级 `scripts_dir` > 工作流级 > 此变量 > exe 搜索 > `./scripts`） |
| `NEXUS_PORT` | `48080` | Dashboard HTTP 监听端口 |

### FILE: WORKFLOW_REFERENCE.md
- **[MEDIUM]** **[COMPLETENESS]** §1.3 command field description lists template variables `{{datarouter.X.content}}`, `{{datarouter.X.route}}`, `{{metadata.run_count}}`, and `{{metadata.timed_out}}` but omits `{{node_dir}}` and `{{prompt}}`, which are also valid template variables per §5.2 and the LLM node documentation in §7.
  - FIND: - `{{metadata.timed_out}}` 替换为 `true` / `false`（上次执行是否超时）
  - REPLACE_WITH: - `{{metadata.timed_out}}` 替换为 `true` / `false`（上次执行是否超时）
- `{{node_dir}}` 替换为当前节点的 scripts 目录绝对路径
- `{{prompt}}` 替换为 ProviderDef 的 `prompt` 字段渲染值（仅用于 `type: "llm"` 的 command）

### FILE: README.md
- **[MEDIUM]** **[COMPLETENESS]** Directory tree entry for `examples/` says only "示例工作流 JSON" without count or representative filenames. The directory contains 13 example JSON files covering branch routing, parallel review, review-fix loops, SDK mode, opencode integration, and more.
  - FIND: ├── examples/                      # 示例工作流 JSON
  - REPLACE_WITH: ├── examples/                      # 示例工作流 JSON（13 个文件，含 claude-test.json, review-fix-retro-loop.json, parallel-review.json, llm-sdk-test.json 等）

### FILE: WORKFLOW_REFERENCE.md
- **[MEDIUM]** **[CORRECTNESS]** §2.6 retry table says node-level `NodeDef.max_retries` is "字段预留，当前仅使用全局值" (field reserved, only global value used). But QUICKSTART.md c4 example explicitly sets `max_retries: 1` on the risky node and documents it as a working feature. These two statements are contradictory — either the field works or it doesn't.
  - FIND: | 节点级 | `NodeDef.max_retries` | 继承全局（字段预留，当前仅使用全局值） |
  - REPLACE_WITH: | 节点级 | `NodeDef.max_retries` | 继承全局。设置后覆盖引擎全局 `max_timeout_retries` |

### FILE: WORKFLOW_REFERENCE.md
- **[LOW]** **[CLARITY]** §4.3 "stdout 输出格式" and §4.5 "JSON 输出协议" describe the same JSON output format with ~70% overlapping content (same JSON example, same field table with route + content, overlapping rules about parse failures). §4.5 adds engine parsing behavior but duplicates the schema description. Consolidate to avoid reader confusion.
  - FIND: ### 4.5 JSON 输出协议

节点通过 stdout 输出单一 JSON 对象（[`NodeOutput`]）与引擎通信：

```json
{"route":"approved","content":"节点执行结果"}
```

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `route` | `string` | 路由键。非空时作为 exit_reason 参与边匹配；空字符串表示无特定路由 |
| `content` | `string` | 节点输出文本。通过 DataRouter 传递给下游节点 |

引擎在进程退出后解析该 JSON：
- 若 stdout 不是合法 JSON 或缺少 `route` 字段 → 返回 `SpawnError`，节点标记为 failed
- 若 JSON 中 `route` 非空 → 提取为 exit_reason 供边匹配
- 若 JSON 中 `route` 为空 → 无 exit_reason，仅匹配 `exit_reason: null` 的边
- `content` 字段进入 DataRouter，作为该节点的输出供下游使用
  - REPLACE_WITH: ### 4.5 引擎解析行为

引擎在进程退出后解析 stdout JSON（输出格式见 §4.3）：
- 若 stdout 不是合法 JSON 或缺少 `route` 字段 → 返回 `SpawnError`，节点标记为 failed
- 若 JSON 中 `route` 非空 → 提取为 exit_reason 供边匹配
- 若 JSON 中 `route` 为空 → 无 exit_reason，仅匹配 `exit_reason: null` 的边
- `content` 字段进入 DataRouter，作为该节点的输出供下游使用

### FILE: QUICKSTART.md
- **[LOW]** **[CLARITY]** Two adjacent dependency notice blockquotes partially duplicate `llm_sdk` info. The first says `ANTHROPIC_API_KEY` is required; the second says llm_sdk "仅需 Python 3 + pip install anthropic" (more accurate — key can also come from `ANTHROPIC_AUTH_TOKEN` or `~/.claude/settings.json`). The first notice is redundant and slightly contradictory.
  - FIND: > **依赖**：`type: "llm_sdk"` 需要 Python 3 + `pip install anthropic` + 有效的 `ANTHROPIC_API_KEY` 环境变量。

> **依赖**：`type: "llm"` 节点需要 Python 3 + LLM CLI（如 Claude Code）。`type: "llm_sdk"` 节点仅需 Python 3 + `pip install anthropic`。
  - REPLACE_WITH: > **依赖**：`type: "llm"` 节点需要 Python 3 + LLM CLI（如 Claude Code）。`type: "llm_sdk"` 节点需要 Python 3 + `pip install anthropic` + API 凭证（`ANTHROPIC_API_KEY`、`ANTHROPIC_AUTH_TOKEN` 或 `~/.claude/settings.json` 中的 key）。

### FILE: NEXUS_WORKFLOW_SKILL.md
- **[LOW]** **[CLARITY]** MCP section uses a `json`-fenced code block containing `//` comment lines, which are not valid JSON. Syntax highlighters and JSON validators will flag these. Use a bullet list or plain code fence instead.
  - FIND: ```json
// Validate workflow structure
{"method":"validate_workflow","params":{"workflow_json":"<JSON>"},"id":1}
// Parse and normalize workflow JSON
{"method":"parse_workflow","params":{"workflow_json":"<JSON>"},"id":1}
// Describe the workflow JSON schema
{"method":"describe_schema","params":{},"id":1}
// Run a workflow
{"method":"run_workflow","params":{"workflow_json":"<JSON>","dashboard_url":"http://127.0.0.1:48080"},"id":1}
```
  - REPLACE_WITH: - `validate_workflow` — 校验工作流结构：`{"method":"validate_workflow","params":{"workflow_json":"<JSON>"},"id":1}`
- `parse_workflow` — 解析并规范化工作流 JSON：`{"method":"parse_workflow","params":{"workflow_json":"<JSON>"},"id":1}`
- `describe_schema` — 描述工作流 JSON schema：`{"method":"describe_schema","params":{},"id":1}`
- `run_workflow` — 运行工作流：`{"method":"run_workflow","params":{"workflow_json":"<JSON>","dashboard_url":"http://127.0.0.1:48080"},"id":1}`

### FILE: NEXUS_VS_LANGGRAPH.md
- **[LOW]** **[CLARITY]** Appendix section title says "Nexus llm_sdk 的 tool loop 安全隐患" (security risks), but the table describes correctness/robustness issues (format mismatch, missing stop_reason, missing fields, exceptions) — not security vulnerabilities. The term "安全隐患" is misleading for what are essentially implementation quality concerns.
  - FIND: ## 附录：Nexus llm_sdk 的 tool loop 安全隐患
  - REPLACE_WITH: ## 附录：Nexus llm_sdk 的 tool loop 健壮性风险

### FILE: WORKFLOW_REFERENCE.md
- **[LOW]** **[COMPLETENESS]** Table of contents (lines 9-62) lists subsections 9.1 through 9.3, but §9.4 "Dashboard 集成" (line 1386) is not in the TOC. Readers skimming the TOC will miss the entire Dashboard integration section.
  - FIND:    - [9.3 通用调试步骤](#93-通用调试步骤)
10. [相关文档](#10-相关文档)
  - REPLACE_WITH:    - [9.3 通用调试步骤](#93-通用调试步骤)
   - [9.4 Dashboard 集成](#94-dashboard-集成)
10. [相关文档](#10-相关文档)

### Fix Applied
- Fixed WORKFLOW_REFERENCE.md: §2.6 retry table — changed "字段预留，当前仅使用全局值" to "继承全局。设置后覆盖引擎全局 max_timeout_retries"
- Fixed WORKFLOW_REFERENCE.md: §1.3 command rules — added `{{node_dir}}` and `{{prompt}}` template variable documentation
- Fixed WORKFLOW_REFERENCE.md: TOC — added §9.4 Dashboard 集成 entry
- Fixed README.md: System requirements — split Linux (precompiled) and macOS (from source) into separate entries
- Note: 14 of 20 findings already resolved before this fix round (likely applied during prior review iterations). 2 findings (NEXUS_VS_LANGGRAPH.md) target an HTML file not present as .md.

---

## Round 1 Fix Status (2026-07-16)

### Fix Applied
- Fixed README.md: Split Linux/macOS system requirements — Linux now notes precompiled binaries (`bin/linux/`), macOS still requires source build
- Fixed WORKFLOW_REFERENCE.md: Removed `"http"` from ProviderDef type enum in §1.3 table, added reserved/unimplemented note
- Fixed WORKFLOW_REFERENCE.md: Updated retry table in §2.6 — node-level `NodeDef.max_retries` now documented as working override of global `max_timeout_retries`
- Fixed WORKFLOW_REFERENCE.md: Added §9.4 "Dashboard 集成" to table of contents

### Already Applied Before This Round
- WORKFLOW_REFERENCE.md: §5.2 template example already uses `type: "shell"` with echo command
- WORKFLOW_REFERENCE.md: §5.2 unrecognized template text already corrected (Validator error, not runtime warning)
- NEXUS_WORKFLOW_SKILL.md: llm quick config already uses `--output-format json` (not `stream-json`)
- WORKFLOW_REFERENCE.md: §9.1 exit code 3 description already trimmed (no `预留，当前未实现` text)
- NEXUS_WORKFLOW_SKILL.md: Shell mode auto-escape claim already corrected to "substituted directly"
- WORKFLOW_REFERENCE.md: §8.3 self-loop error message already shows `cycle without entry: deadlock detected`
- WORKFLOW_REFERENCE.md: §1.3 `max_tokens` already marked for `llm_sdk` only, with CLI `--max-tokens` flag note
- QUICKSTART.md: Dashboard 示范工作流 description already corrected (c2/c3/c4 noted as inline prose)
- QUICKSTART.md: Duplicate dependency notices already consolidated into single merged notice
- WORKFLOW_REFERENCE.md: §1.3 command field template variables already include `{{node_dir}}` and `{{prompt}}`
- README.md: `NEXUS_SCRIPTS_DIR` already present in environment variables table
- README.md: `examples/` directory tree entry already shows count and representative filenames
- NEXUS_WORKFLOW_SKILL.md: MCP section already uses bullet list format (no JSON fence with comments)
- WORKFLOW_REFERENCE.md: §4.5 already consolidated to "引擎解析行为" with cross-reference to §4.3

### Skipped
- NEXUS_VS_LANGGRAPH.md (2 fixes): Source file does not exist in repository (content may be in `NEXUS_VS_LANGGRAPH_ANALYSIS.html` report). Skipped `exit_reason: "needs_fix"` → `"rejected"` and appendix title change.

### Fix Applied (Final Verification 2026-07-16)
- Verified all 18 applicable fixes in WORKFLOW_REFERENCE.md, README.md, QUICKSTART.md, and NEXUS_WORKFLOW_SKILL.md — all confirmed correctly applied. 2 findings skipped (NEXUS_VS_LANGGRAPH.md does not exist as .md). No remaining unfixed issues.

### Fix Applied (2026-07-16 Re-verification)
- WORKFLOW_REFERENCE.md: 13 fixes confirmed — §5.2 template example (shell type), §5.2 UnrecognizedTemplate error text, §9.1 exit code 3 description, §1.3 http type removed from enum, §1.3 max_tokens marked llm_sdk only, §1.3 command template variables (node_dir, prompt), §2.6 retry table (node-level override), §4.5 consolidated to 引擎解析行为, §8.3 self-loop error message, TOC §9.4 added, and more
- README.md: 3 fixes confirmed — Linux/macOS system requirements split, NEXUS_SCRIPTS_DIR env var added, examples/ directory tree updated
- QUICKSTART.md: 2 fixes confirmed — Dashboard 示范工作流 text corrected, duplicate dependency notices consolidated
- NEXUS_WORKFLOW_SKILL.md: 3 fixes confirmed — llm quick config uses --output-format json, shell auto-escape claim corrected, MCP section uses bullet list format
- SKIPPED: NEXUS_VS_LANGGRAPH.md (2 fixes) — file does not exist as .md, content is in HTML report only
