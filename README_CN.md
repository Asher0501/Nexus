# 🕸️ Nexus

> **声明式工作流编排——DAG 即 JSON，引擎自动执行。** LLM Agent、HTTP 调用、Shell 脚本、人工介入——任何能通过 stdin/stdout 输出 JSON 的进程都是节点。引擎本身零 Python 依赖。

[![Rust](https://img.shields.io/badge/language-Rust-orange?style=flat-square)](https://www.rust-lang.org/)
[![Tests](https://img.shields.io/badge/tests-309%20passed-green?style=flat-square)]()
[![Platforms](https://img.shields.io/badge/platform-Windows%20%7C%20Linux-blue?style=flat-square)]()

---

## Nexus vs LangGraph

| | LangGraph | Nexus |
|---|-----------|-------|
| **工作流定义** | Python 代码（StateGraph） | 纯 JSON——无需编程 |
| **节点实现** | Python 函数 | 任意进程（任意语言、任意运行时） |
| **节点协议** | 函数调用（进程内） | stdin/stdout JSON（跨进程、跨语言） |
| **路由方式** | 代码中的条件边 | JSON 中的 `exit_reason` 字符串匹配 |
| **数据流** | 共享 State 对象 | 独立的 `dataflows` 图——可与执行方向相反 |
| **循环终止** | `interrupt()` + 外部恢复 | `route_policy`（N 轮或 N 秒），无状态——无需检查点 |
| **人工介入** | 在节点处强制 `interrupt()` | LLM 不确定时自主调用 `ask_human` 工具 |
| **执行模型** | 进程内 Python | 子进程编排——异构节点共存一个 DAG |
| **部署方式** | 需要 Python 运行时 | 单个二进制文件（~4MB），零运行时依赖 |

LangGraph 是 Python agent 的**图即代码**库。Nexus 是**图即数据**引擎——JSON 定义，随处执行，节点可以是任何东西。

## 适用场景

| 场景 | 为什么用 Nexus |
|------|---------------|
| **LLM Agent 流水线** | 多阶段审查→修复→验证循环，自动终止 |
| **自监督工作流** | 设计审查、代码审计——LLM 自主审查、修复、再审查 |
| **微服务编排** | HTTP Provider——声明式调用任何 API，无需写脚本 |
| **CI/CD 自定义流水线** | Shell + Subprocess 节点，条件分支，重试逻辑 |
| **审批流程** | `ask_human` 人工介入——LLM 提问，人类回答，继续执行 |
| **跨语言编排** | Rust 引擎调度 Python、Node.js、Go、Shell——任何输出 JSON 的进程 |
| **数据 ETL** | 扇出并行处理，`trigger: "all"` 扇入聚合 |

## 快速开始

```bash
cd release
./bin/nexus-cli run examples/http-test.json --verbose   # CLI 执行
./bin/nexus-dashboard                                   # Dashboard → http://127.0.0.1:48080
```

## 安装

预编译二进制（Windows/Linux）在 `release/bin/`。开箱即用。

```bash
git clone https://github.com/Asher0501/Nexus.git
cd Nexus/engine && cargo build --release
```

## 五分钟示例：代码审查循环

```json
{
  "nodes": [
    {"id":"seed","providers":[{"type":"shell","command":"echo fn divide(a,b){a/b}"}],"process_timeout_secs":10},
    {"id":"review","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","system_prompt":"你是代码审查专家。","prompt":"审查：\n{{datarouter.seed.content}}\n{{datarouter.fixed_code.content}}\n\n仅输出：{\"route\":\"approved|needs_fix\",\"content\":\"审查意见\"}","routes":["approved","needs_fix"],"max_tokens":4096}],"process_timeout_secs":600,"route_policy":{"type":"max_runs","max":3,"then_route":"approved"}},
    {"id":"fix","providers":[{"type":"llm_sdk","model":"claude-sonnet-5-20251001","prompt":"修复以下所有问题，输出完整修正代码。\n\n{{datarouter.review.content}}","routes":["done"],"max_tokens":4096}],"process_timeout_secs":600}
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

审查→修复→审查循环。`route_policy.max_runs=3` 保证一定终止。

## 核心设计

### h_e + g_e — 无状态边分解

每条边由两个正交的**纯函数**构成——没有 `triggered` 标记，没有状态机：

- **h_e**：事件类型匹配 + exit_reason 过滤 + 阈值计数器（无状态，每次事件独立评估）
- **g_e**：策略聚合——`any`（立即触发）或 `all`（等所有上游到齐）

**价值**：环路天然支持，无需特殊处理。节点通过环重新进入自身时，h_e 独立重新评估。调度器代码更短、更简单。行为更容易推理。

### 调度/数据流分离

`edges` 控制执行顺序。`dataflows` 控制数据走向。**两者是独立的图。** 数据可以反向流动、跨级传递、使用别名。

**价值**：最大路由灵活性。下游节点可以消费任意上游节点的输出，不受执行拓扑约束。

### 提供器抽象

5 种 Provider 类型，统一的 JSON 协议——stdin 入上下文，stdout 出 `{route, content}`：

| Provider | 示例 |
|----------|------|
| `subprocess` | `"command": "python script.py"` |
| `shell` | `"command": "grep error log.txt \| wc -l"` |
| `http` | `"url": "https://api.example.com", "method": "POST"` |
| `llm` | `"command": "claude -p \"{{prompt}}\""` |
| `llm_sdk` | `"model": "claude-sonnet-5-20251001"` |

**价值**：异构节点共存一个 DAG。一次 HTTP 健康检查触发 LLM 分析，分析结果由 Shell 脚本处理。无需胶水代码。

### LLM 原生工具架构

`llm_sdk` 节点可调用 `read_file`、`write_file`、`execute_command` 和 `ask_human`。LLM 自主决定何时使用哪个工具。

**价值**：LLM 驱动工作流，而非相反。它觉得需要澄清 → 调用 `ask_human`。它觉得需要读文件 → 调用 `read_file`。无需预设交互节点。

### 人工介入即工具

`ask_human` 是 LLM 调用的工具，不是强制的检查点。不确定时，LLM 自己提问。Dashboard 在节点面板中显示问题。CLI 从终端读取。内存 HTTP 池——零轮询、零文件。

**价值**：人类判断在 LLM 认为需要时介入，而非预设代码中断点。支持扇出——多个 LLM 同时提问，问题排队，逐条回答。

### 路由策略保循环终止

`route_policy.max_runs`（N 轮）或 `route_policy.max_duration`（N 秒）强制引擎覆盖节点路由，退出循环。

**价值**：循环天然安全。不会无限执行。无需手动检查点/恢复。

## 核心特性

- **分支路由** — `{"route":"approved"}` → 匹配 `exit_reason:"approved"` 的边触发
- **扇出/扇入** — 并行节点，`trigger:"all"` 聚合等待
- **超时+重试** — 每节点超时，仅超时/启动失败触发重试
- **10 项结构校验** — 死锁、不可达节点、数据流缺失——执行前静态捕获
- **实时 DAG 可视化** — Dashboard WebSocket 实时推送节点状态 + 点击交互
- **停止按钮** — 取消运行中的工作流，Pending 节点标记 Skipped
- **MCP 集成** — stdio JSON-RPC 服务端，从 Claude Code 运行工作流
- **跨平台** — Windows + Linux 二进制，单个 ~4MB 可执行文件

## 文档

| 文档 | 内容 |
|------|------|
| [Workflow Reference](release/WORKFLOW_REFERENCE.md) | 完整 Schema、调度语义、节点协议 |
| [Quickstart](release/QUICKSTART.md) | 5 分钟入门与示范用例 |
| [Skill Reference](release/NEXUS_WORKFLOW_SKILL.md) | Claude Code 工作流生成指南 |

## 构建

```bash
cd engine
cargo build --release
# → engine/target/release/nexus-cli, nexus-dashboard, nexus-mcp-server
```
