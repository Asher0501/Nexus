# Nexus

> 有向图驱动的子进程编排引擎。定义固定拓扑的工作流 DAG，引擎机械执行，Claude 实时监视。

## 定位 — 与 Claude Subagent 互补

| Claude Subagent | Nexus Workflow |
|---|---|
| 模型驱动，探索式 | 固定拓扑，确定性 |
| "找找这个项目的 bug" | "代码审查 → 修复 → 复盘" |
| 每步决策由 LLM 做 | 每步路由由 exit_reason / 边条件决定 |
| 适合未知探索 | 适合已知流程、重复执行 |

**Nexus 集成进 Claude Code（MCP）：Claude 定义 JSON → 触发执行 → 连接 Dashboard 监视 → 出问题介入。**

## 组成

```
engine/
├── crates/
│   ├── engine/       核心引擎 — DAG 调度、数据路由、分支匹配
│   ├── cli/          CLI — nexus-cli run workflow.json
│   ├── mcp-server/   MCP Server — stdio JSON-RPC，接入 Claude
│   └── dashboard/    Dashboard 后端 — REST API + WebSocket 实时推送
├── scripts/          构建与 LLM wrapper 脚本
└── Cargo.toml        Rust workspace
```

## 三个二进制

| 二进制 | 用途 | 接口 |
|--------|------|------|
| `nexus-cli` | 命令行直接执行工作流 | CLI (`nexus-cli run workflow.json`) |
| `nexus-mcp-server` | 接入 Claude Code 等 MCP 客户端 | stdio JSON-RPC |
| `nexus-dashboard` | 运行时监视面板 | HTTP REST + WebSocket (port 48080) |

## 快速开始

```bash
cd engine
cargo build --release

# CLI 运行
./target/release/nexus-cli run release/examples/branch-routing-e2e.json --verbose

# 启动监视面板
./target/release/nexus-dashboard
# → http://127.0.0.1:48080/api/workflows

# MCP 模式
echo '{"jsonrpc":"2.0","method":"describe_schema","params":{},"id":1}' | ./target/release/nexus-mcp-server
```

## MCP 工具

| 工具 | 说明 |
|------|------|
| `validate_workflow` | 验证 JSON 结构 + DAG 拓扑合法性 |
| `parse_workflow` | 解析 JSON 返回 GraphDef |
| `describe_schema` | 返回 WorkflowDef JSON Schema |
| `run_workflow` | 执行工作流，返回 run_id 供监视 |

## 文档

- `release/WORKFLOW_REFERENCE.md` — 工作流定义完整参考（schema、调度语义、模式模板）
- `release/QUICKSTART.md` — 5 分钟上手
- `engine/README.md` — 引擎开发指引
