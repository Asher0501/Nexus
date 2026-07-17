# 🕸️ Nexus

> **有向图驱动的插件编排引擎。** 定义工作流 DAG，引擎自动执行。LLM Agent、HTTP 调用、Shell 脚本——任何能通过 stdin/stdout 输出 JSON 的进程都是一个节点。

[![Rust](https://img.shields.io/badge/language-Rust-orange?style=flat-square)](https://www.rust-lang.org/)
[![Tests](https://img.shields.io/badge/tests-309%20passed-green?style=flat-square)]()

---

## 快速开始

```bash
# CLI — 运行工作流
cd release
./bin/nexus-cli run examples/http-test.json --verbose

# Dashboard — REST API + 实时 DAG 可视化
./bin/nexus-dashboard
# → http://127.0.0.1:48080

# MCP Server — JSON-RPC 标准输入输出
echo '{"jsonrpc":"2.0","method":"validate_workflow","params":{"workflow_json":"{\"nodes\":[{\"id\":\"hello\",\"providers\":[{\"type\":\"subprocess\",\"command\":\"echo hi\"}],\"process_timeout_secs\":10}]}"},"id":1}' | ./bin/nexus-mcp-server
```

## 安装

Windows 和 Linux 预编译二进制在 `release/bin/`。开箱即用。

```bash
git clone https://github.com/Asher0501/Nexus.git
cd Nexus/engine
cargo build --release
```

## Hello World

```json
{"nodes":[{"id":"hello","providers":[{"type":"subprocess","command":"echo {\"route\":\"ok\",\"content\":\"Hello Nexus\"}"}],"process_timeout_secs":10}]}
```

## 代码审查循环

```json
{
  "nodes": [
    {"id":"seed","providers":[{"type":"shell","command":"echo fn divide(a,b){a/b}"}],"process_timeout_secs":10},
    {"id":"review","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","prompt":"审查: {{datarouter.seed.content}}。输出 {\"route\":\"approved|needs_fix\",\"content\":\"...\"}","routes":["approved","needs_fix"],"max_tokens":4096}],"process_timeout_secs":600,"route_policy":{"type":"max_runs","max":3,"then_route":"approved"}},
    {"id":"fix","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","prompt":"修复: {{datarouter.review.content}}","routes":["done"],"max_tokens":4096}],"process_timeout_secs":600}
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

## 核心概念

**节点（Node）** 是能通过 stdin/stdout 输出 JSON 的进程。**调度边（Edge）** 定义执行顺序。**数据流（Dataflow）** 定义数据走向。两者独立——数据可以沿任意方向流动，不依赖调度顺序。

**提供器（Provider）** 决定节点如何执行：

| 提供器 | 说明 |
|--------|------|
| `subprocess` | 直接启动进程 |
| `shell` | 通过 Shell 执行（支持管道、重定向） |
| `http` | 发起 HTTP 请求 |
| `llm` | 启动 LLM 命令行工具（claude、opencode 等） |
| `llm_sdk` | 直接调用 Anthropic API（内置工具：read_file、write_file、execute_command、ask_human） |

**路由策略（route_policy）** 保证循环终止：`max_runs`（N 轮后退出）或 `max_duration`（累计 N 秒后退出）。

**人工介入（Human-in-the-loop）**：`llm_sdk` 节点不确定时会调用 `ask_human` 工具询问人类。Dashboard 在节点面板中展示问题，CLI 从终端读取答案。

**ToolBridge**：`llm_sdk` 自动从 `~/.nexus/mcp.json` 中配置的 MCP 服务器发现工具。

## 核心特性

- **h_e + g_e 正交分解** — 边是无状态的纯函数，环路天然支持
- **分支路由** — 节点输出 `{"route":"approved","content":"..."}`，边按 `exit_reason` 匹配
- **扇出/扇入** — `trigger: "all"` 实现并行聚合
- **超时+重试** — 每节点可配超时和重试（仅超时/启动失败触发重试）
- **10 项结构校验** — 静态 DAG 分析检测死锁、不可达节点、数据流缺失
- **实时 DAG 可视化** — Dashboard 通过 WebSocket 实时推送节点状态，点击节点即可交互
- **MCP 集成** — stdio JSON-RPC 服务端，可从 Claude Code 运行工作流

## 文档

| 文档 | 内容 |
|------|------|
| [Workflow Reference](release/WORKFLOW_REFERENCE.md) | 完整 Schema、调度语义、节点协议 |
| [Quickstart](release/QUICKSTART.md) | 5 分钟入门 |
| [Skill Reference](release/NEXUS_WORKFLOW_SKILL.md) | Claude Code 工作流生成指南 |

## 构建

```bash
cd engine
cargo build --release
# 产物在 engine/target/release/
# 将产物复制到 release/bin/，配合 release/scripts/ 即为完整发行包
```
