# Nexus — 有向图驱动的插件编排引擎

Nexus 是一个有向图驱动的插件编排引擎。它读取 JSON 定义的工作流，构建有向图，然后调度执行图中的节点。所有节点统一为 `subprocess` 类型，通过退出码判断完成状态，通过 stdout 返回结果。**不改引擎代码，通过 NODE_PROTOCOL 即可接入任何工具。**

## 快速开始

```bash
# 验证工作流（不执行）
nexus-cli run test_workflow.json --validate-only

# 运行一个工作流
nexus-cli run test_workflow.json

# 详细日志（含流式输出）
nexus-cli run test_workflow.json --verbose

# 执行完后 dump 节点状态
nexus-cli run test_workflow.json --dump-state
```

## 架构

```
nexus-engine/    ← 核心引擎库（lib crate）
nexus-cli/       ← 命令行工具（binary crate，1.5MB静态链接）
nexus-mcp-server/← MCP 服务器（JSON-RPC stdio 协议）
```

### 执行流程

```
Scheduler (调度层) → 决定谁 ready
    ↓ NodeReady
Semaphore (并发控制) → acquire() 等空位
    ↓ spawn
SubprocessExecutor → stdin 写 JSON + 逐行读 stdout + wait
    ↓ 实时 chunk（nexus::node::chunk） + 最终 stdout
DataRouter → 存储输出，传递给下游
```

### 流式输出

节点执行期间，每行 stdout 会通过 `nexus::node::chunk` 事件实时上报（`--verbose` 时可见）。
节点退出后，完整输出进入 DataRouter 传递给下游。

## 节点协议（NODE_PROTOCOL）

**任何语言实现的进程都可以成为 Nexus 节点，不改引擎代码。**

```
stdin  ← NodeContext JSON（引擎写入）
stdout → 纯文本结果（引擎实时逐行读取）
exit   → 0 = 完成，非0 = 失败
```

分支路由在 stdout 首行加前缀：
```
__nexus_exit_reason: approved
业务输出内容...
```

流式事件在 stdout 行加前缀（进程无需等退出即可发送）：
```
__nexus_event: 进度：2/5 完成
```

## 工作流示例

```json
{
  "nodes": [
    {
      "id": "step1",
      "providers": [{"type": "subprocess", "command": "python my_plugin.py"}],
      "process_timeout_secs": 30,
      "predecessors": []
    },
    {
      "id": "step2",
      "providers": [{"type": "subprocess", "command": "powershell -File step2.ps1"}],
      "process_timeout_secs": 60,
      "predecessors": [
        {"node_id": "step1", "trigger": "all", "event": "complete"}
      ],
      "inputs": ["step1"]
    }
  ]
}
```

### 集成 OpenCode / Claude Code

```bash
nexus-cli run workflows\opencode-example.json
```

OpenCode、Claude Code 等 AI 工具通过 `subprocess` 节点直接接入，不需要包装脚本：

```json
{
  "type": "subprocess",
  "command": "opencode run --format json --dangerously-skip-permissions -- \"{{inputs.prompt}}\""
}
```

所有子进程统一由 SubprocessExecutor 管理（spawn、pipe、timeout、逐行流式读取）。
**不需要 AI 专用 executor，不需要包装脚本。**

## CLI 参数

```
nexus-cli run <workflow.json>
  --max-concurrency N    最大并发节点数（默认：CPU 核数）
  --node-timeout S       节点默认超时秒数（默认：3600，被节点的 process_timeout_secs 覆盖）
  --verbose              详细日志（含流式 chunk 输出）
  --validate-only        仅验证，不执行
  --dump-state           完成后输出节点状态快照
```

| 退出码 | 含义 |
|--------|------|
| 0 | 成功 |
| 1 | 验证/读取错误 |
| 2 | 运行时错误 |
| 3 | 引擎空闲超时 |

## 设计文档

| 文档 | 内容 |
|------|------|
| `docs/architecture/ARCHITECTURE.md` | 系统架构设计 |
| `docs/architecture/NODE_PROTOCOL.md` | 节点通信协议（含流式协议） |
| `docs/philosophy/DESIGN_PHILOSOPHY.md` | 设计哲学 |
| `code/review/` | 所有检视意见及处理方案 |

## 打包部署

```bash
# 构建
cargo build --release

# 打包（包含运行时依赖）
powershell -File scripts\package.ps1
```

产物在 `nexus-dist/` 目录：
```
nexus-dist/
├── bin/
│   ├── nexus-cli.exe            # 命令行工具（1.5MB，独立 exe）
│   └── nexus-mcp-server.exe     # MCP 服务器
├── workflows/
│   └── opencode-example.json    # OpenCode 工作流示例
├── test_workflow.json           # 测试工作流
└── README.md
```

目标平台需要：Rust 1.83+（仅编译时需要），运行时无需额外依赖。

## 系统要求

- 编译：Rust 1.83+ (edition 2024)
- 运行：Windows (GNU/MSVC) 或 Linux
