# Nexus

> 有向图驱动的子进程编排引擎 —— 定义 DAG 工作流，引擎机械执行，Claude 实时监视。

<p align="center">
  <img src="https://img.shields.io/badge/language-Rust-orange?style=flat-square" alt="Rust">
  <img src="https://img.shields.io/badge/tests-238%20passed-green?style=flat-square" alt="Tests">
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="License">
  <img src="https://img.shields.io/badge/MCP-Claude%20Code-7c3aed?style=flat-square" alt="MCP">
</p>

---

## 架构总览

```mermaid
graph TB
    subgraph "Claude Code"
        CC[Claude Agent]
    end

    subgraph "Nexus"
        MCP[nexus-mcp-server<br/>stdio JSON-RPC]
        CLI[nexus-cli<br/>命令行]
        DASH[nexus-dashboard<br/>HTTP :48080]

        subgraph "Engine Core"
            SCHED[Scheduler<br/>h_e + g_e 调度]
            ROUTER[DataRouter<br/>数据流]
            SHELL[NodeShell<br/>subprocess / LLM]
        end
    end

    CC -->|MCP Protocol| MCP
    MCP --> SCHED
    CLI --> SCHED
    SCHED --> ROUTER
    SCHED --> SHELL
    SHELL -->|stdout JSON| ROUTER

    DASH -->|REST| SCHED
    DASH -->|WebSocket| SCHED
    CC -.->|monitor_url| DASH
```

## 定位

| Claude Subagent | Nexus Workflow |
|---|---|
| 模型驱动，探索式 | 固定拓扑，确定性 |
| "找找这个项目的 bug" | "代码审查 → 修复 → 复盘" |
| LLM 做每步决策 | 引擎按 exit_reason 机械路由 |
| 适合未知探索 | 适合已知流程、重复执行 |

**Claude 定义 JSON → MCP 触发 → Dashboard 实时监视 → 出问题介入。**

## 工作流示例

```mermaid
graph LR
    SEED((seed)) --> FAN((fan))
    FAN -->|fan-out| W1((w1))
    FAN -->|fan-out| W2((w2))
    W1 -->|all| MERGE((merge))
    W2 -->|all| MERGE
    MERGE --> REVIEW((review))
    REVIEW -->|"again"| FIX((fix))
    FIX -->|always| REVIEW
    REVIEW -->|"stop"| EXIT((exit))

    style REVIEW fill:#3b82f6,stroke:#1d4ed8,color:#fff
    style FIX fill:#8b5cf6,stroke:#6d28d9,color:#fff
    style EXIT fill:#22c55e,stroke:#16a34a,color:#fff
```

> 8 节点 DAG：fan-out → fan-in → 有向环 → exit_reason 分支路由 → 退出。

## 三个二进制

```mermaid
graph LR
    subgraph "nexus-cli"
        C1[命令行执行<br/>--verbose --validate-only --dump-state]
    end
    subgraph "nexus-mcp-server"
        C2[stdio JSON-RPC<br/>validate / parse / describe / run]
    end
    subgraph "nexus-dashboard"
        C3[REST API + WebSocket<br/>DAG 实时可视化<br/>端口 48080]
    end
```

## Claude Code 集成流程

```mermaid
sequenceDiagram
    participant Agent as Claude Agent
    participant MCP as MCP Server
    participant Engine as Nexus Engine
    participant Dash as Dashboard

    Agent->>MCP: run_workflow(JSON, dashboard_url)
    MCP->>Dash: POST /api/workflows
    Dash-->>MCP: workflow_id
    MCP->>Dash: POST /api/workflows/{id}/run
    Dash-->>MCP: run_id
    MCP-->>Agent: {run_id, monitor_url}
    Agent->>Agent: 告诉用户打开 monitor_url
    
    Dash->>Engine: spawn 引擎
    Engine-->>Dash: WebSocket 实时推送 node_status, node_chunk
    Dash-->>Agent: 浏览器 DAG 节点逐一亮起
```

## 快速开始

```bash
git clone git@github.com:Asher0501/Nexus.git
cd Nexus/engine
cargo build --release

# CLI
./target/release/nexus-cli run ../release/examples/branch-routing-e2e.json --verbose

# Dashboard
./target/release/nexus-dashboard
# → http://127.0.0.1:48080

# MCP
echo '{"jsonrpc":"2.0","method":"describe_schema","params":{},"id":1}' \
  | ./target/release/nexus-mcp-server
```

## 核心特性

| 特性 | 实现 |
|------|------|
| **h_e + g_e 正交分解** | 边 = 纯函数分支匹配 + 策略聚合，无 triggered 状态，环路自然支持 |
| **路由策略** | `route_policy.max_runs` — N 轮后自动退出，节点不感知环路 |
| **调度/数据分离** | `edges` 控制执行顺序，`dataflows` 控制数据流向，独立声明 |
| **LLM 原生集成** | `type: "llm"` → `llm_node.py` wrapper → 任意 CLI 适配 |
| **实时可视化** | WebSocket 推送 + vis-network DAG 实时着色 |
| **MCP 深度集成** | stdio JSON-RPC，可代理到 Dashboard 获得实时监控 |
| **10 项结构验证** | Validator 静态检查 DAG 合法性，含 `ReferencedInputWithoutDataflow` |

## MCP 工具

| 工具 | 说明 |
|------|------|
| `validate_workflow` | 验证 JSON 结构 + DAG 拓扑合法性 |
| `parse_workflow` | 解析 JSON 返回 GraphDef |
| `describe_schema` | 返回 WorkflowDef JSON Schema |
| `run_workflow` | 执行工作流，支持 `dashboard_url` 代理 |

## 项目结构

```
Nexus/
├── engine/                     # Rust workspace
│   ├── crates/
│   │   ├── engine/             # 核心引擎 (150 tests)
│   │   ├── cli/                # 命令行
│   │   ├── mcp-server/         # MCP 服务器 (11 tests)
│   │   └── dashboard/          # Dashboard 后端 (66 tests)
│   └── scripts/                # LLM wrapper + .bat 脚本
├── release/                    # 分发包
│   ├── bin/                    # 预编译二进制 (Win + Linux)
│   ├── scripts/                # 运行时脚本
│   ├── static/                 # Dashboard 前端 + 示范工作流
│   └── *.md                    # 文档
└── .claude/skills/             # Claude Code Skill
```

## 文档

| 文档 | 内容 |
|------|------|
| `release/WORKFLOW_REFERENCE.md` | 工作流定义完整参考 — schema、调度语义、模式模板、边界情况 |
| `release/QUICKSTART.md` | 5 分钟上手 + c2/c3/c4 示范用例，覆盖全部特性 |
| `release/NEXUS_WORKFLOW_SKILL.md` | Claude Code 生成工作流的 Skill 参考（中英双语） |
| `release/README.md` | API 参考、系统要求、构建说明 |
