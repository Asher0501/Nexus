# 快速开始

> 5 分钟内运行你的第一个 Nexus 工作流。

**分类**：manual

---

## 安装

Release 包已包含 Windows 和 Linux (x86_64) 预编译二进制文件，无需额外安装依赖。

```bash
# 解压即用，可将 bin/ (Windows) 或 bin/linux/ (Linux) 加入 PATH 方便全局调用
# 也可直接从任意目录运行（引擎自动解析 scripts/ 路径）
```

`bin/` 目录包含三个二进制文件（Windows `.exe`），`bin/linux/` 为 Linux 版本：

| 文件 (Windows) | 文件 (Linux) | 说明 |
|------|------|------|
| `bin/nexus-cli.exe` | `bin/linux/nexus-cli` | 命令行工作流执行器 |
| `bin/nexus-dashboard.exe` | `bin/linux/nexus-dashboard` | HTTP REST API + WebSocket 服务端 |
| `bin/nexus-mcp-server.exe` | `bin/linux/nexus-mcp-server` | JSON-RPC stdio 服务端 |

## 运行第一个工作流

创建一个 `workflow.json`：

```json
{
  "nodes": [
    {
      "id": "hello",
      "providers": [{"type": "subprocess", "command": "echo Hello Nexus"}],
      "process_timeout_secs": 10
    }
  ]
}
```

运行：

```bash
# Windows
./bin/nexus-cli run workflow.json

# Linux
./bin/linux/nexus-cli run workflow.json
```

## 验证工作流

先验证 JSON 结构和工作流拓扑：

```bash
# Windows
./bin/nexus-cli run workflow.json --validate-only
# Linux
./bin/linux/nexus-cli run workflow.json --validate-only
```

## 看详细输出

```bash
# Windows
./bin/nexus-cli run workflow.json --verbose
# Linux
./bin/linux/nexus-cli run workflow.json --verbose
```

查看每行 stdout 实时输出。

## 查看节点最终状态

```bash
# Windows
./bin/nexus-cli run workflow.json --dump-state
# Linux
./bin/linux/nexus-cli run workflow.json --dump-state
```

## 链式工作流

A → B → C 的链式执行：

```json
{
  "nodes": [
    {
      "id": "fetch",
      "providers": [{"type": "subprocess", "command": "python fetcher.py"}],
      "process_timeout_secs": 30
    },
    {
      "id": "process",
      "providers": [{"type": "subprocess", "command": "python processor.py"}],
      "process_timeout_secs": 60
    },
    {
      "id": "report",
      "providers": [{"type": "subprocess", "command": "python reporter.py"}],
      "process_timeout_secs": 30
    }
  ],
  "edges": [
    { "from": "fetch", "to": "process", "trigger": "all", "event": "complete" },
    { "from": "process", "to": "report", "trigger": "all", "event": "complete" }
  ],
  "dataflows": [
    { "from": "fetch", "to": "process" },
    { "from": "process", "to": "report" }
  ]
}
```

## Dashboard 模式

启动 HTTP 服务端在 `http://127.0.0.1:48080`：

```bash
# Windows
./bin/nexus-dashboard.exe
# Linux
./bin/linux/nexus-dashboard
```

## MCP Server 模式

通过 stdio JSON-RPC 接入 MCP 客户端：

```bash
# Windows
echo '{"jsonrpc":"2.0","method":"validate_workflow","params":{"workflow_json":"{\"nodes\":[{\"id\":\"hello\",\"providers\":[{\"type\":\"subprocess\",\"command\":\"echo hi\"}],\"process_timeout_secs\":10}]}"},"id":1}' | ./bin/nexus-mcp-server.exe
# Linux
echo '{"jsonrpc":"2.0","method":"validate_workflow","params":{"workflow_json":"{\"nodes\":[{\"id\":\"hello\",\"providers\":[{\"type\":\"subprocess\",\"command\":\"echo hi\"}],\"process_timeout_secs\":10}]}"},"id":1}' | ./bin/linux/nexus-mcp-server
```

## LLM 节点

使用 `type: "llm"` 可将任意 LLM CLI 作为工作流节点。引擎通过 `scripts/llm_node.py` 自动处理跨平台 CLI 发现、输出解析和路由。

> **依赖**：LLM 节点需要系统安装 Python 3 + Claude Code CLI。

## 示范工作流

Dashboard 内置了三个可从 `static/` 导入的示范工作流，覆盖 Nexus 全部特性：

### c2 — Fan-out / Fan-in + 有向环

`seed → fan → w1,w2 → merge → review ⇄ fix → exit`

| 特性 | 位置 |
|------|------|
| fan-out / fan-in (`trigger: "all"`) | merge 节点 |
| 有向环 (review⇄fix) | `route_policy.max_runs=3` 控制退出 |
| exit_reason 路由 | `"again"` → fix, `"stop"` → exit |
| 文件输出 (prompt) | review → review.md, fix → fix.md |

### c3 — 条件分支 + 自环 + Failed 边

`go → split → A(llm) ⇄ A_self → merge ← B(subprocess) ← fallback → final → exit`

| 特性 | 位置 |
|------|------|
| 条件分支 | split 按 route `"a"`/`"b"` 路由 |
| 自环 + threshold | A → A_self → A, `threshold:2` |
| `subprocess` provider | B 节点 |
| `event: "failed"` 边 | B → fallback |
| dataflow alias | `a_out`/`b_out` |

### c4 — Timeout + Retry + 恢复

`go → risky(sleep3.bat, timeout=1s, max_retries=1) → fallback → fallback_report → exit`

| 特性 | 位置 |
|------|------|
| `event: "timeout"` 边 | risky → fallback |
| `max_retries` (节点级) | `risky.max_retries: 1` |
| `exit_reason: null` (匹配任意 route) | fallback 的出边 |

## 更多

- **[WORKFLOW_REFERENCE.md](./WORKFLOW_REFERENCE.md)** — 工作流定义完整参考
- **[README.md](./README.md)** — 完整 API 参考和架构说明
