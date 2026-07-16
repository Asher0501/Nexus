# Nexus v0.2.0

Nexus 是一个有向图驱动的插件编排引擎。定义工作流 DAG → 引擎按调度边与数据流边自动执行。

## 包含的二进制文件

| 平台 | 文件 | 大小 | 说明 |
|------|------|------|------|
| Windows | `bin/nexus-cli.exe` | ~1.7 MB | 命令行工作流执行器 |
| Windows | `bin/nexus-dashboard.exe` | ~5 MB | HTTP REST API + WebSocket 服务端（端口 48080） |
| Windows | `bin/nexus-mcp-server.exe` | ~0.9 MB | JSON-RPC stdio 服务端（可接入 MCP 客户端） |
| Linux | `bin/linux/nexus-cli` | ~1.9 MB | 命令行工作流执行器 |
| Linux | `bin/linux/nexus-dashboard` | ~5 MB | HTTP REST API + WebSocket 服务端（端口 48080） |
| Linux | `bin/linux/nexus-mcp-server` | ~1 MB | JSON-RPC stdio 服务端（可接入 MCP 客户端） |

## 快速开始

### 1. CLI 运行工作流

**Windows:**
```bash
cd release
./bin/nexus-cli run examples/http-test.json --verbose
```

**Linux:**
```bash
cd release
./bin/linux/nexus-cli run examples/http-test.json --verbose
```

### 2. 启动 Dashboard

**Windows:**
```bash
cd release
./bin/nexus-dashboard.exe
```
**Linux:**
```bash
cd release
./bin/linux/nexus-dashboard
```

服务监听 `http://127.0.0.1:48080`。

### 3. API 端点

| 方法 | 路径 | 说明 |
|------|------|------|
| `GET` | `/api/workflows` | 列出所有工作流 |
| `POST` | `/api/workflows` | 创建工作流 `{"name":"...","definition":{...}}` |
| `GET` | `/api/workflows/{id}` | 获取工作流详情 |
| `PUT` | `/api/workflows/{id}` | 更新工作流 |
| `DELETE` | `/api/workflows/{id}` | 删除工作流 |
| `GET` | `/api/workflows/{id}/graph` | 获取 DAG 拓扑（节点/边/数据流） |
| `POST` | `/api/workflows/{id}/run` | 触发运行 |
| `GET` | `/api/runs` | 列出所有运行记录 |
| `GET` | `/api/runs/{id}` | 获取运行详情 |
| `POST` | `/api/runs/{id}/stop` | 停止运行中的工作流 |
| `WS` | `/ws/runs/{run_id}` | WebSocket 实时状态推送 |

### 4. MCP Server（stdio 模式）

```bash
echo '{"jsonrpc":"2.0","method":"validate_workflow","params":{"workflow_json":"{\"nodes\":[{\"id\":\"hello\",\"providers\":[{\"type\":\"subprocess\",\"command\":\"echo hi\"}],\"process_timeout_secs\":10}]}"},"id":1}' | ./bin/nexus-mcp-server
```

支持的方法: `validate_workflow` | `parse_workflow` | `describe_schema` | `run_workflow`

## Provider 类型

| Provider | 说明 | 适用场景 |
|----------|------|----------|
| `subprocess` | 直接 spawn 子进程 | 简单命令，无 shell 语法 |
| `shell` | 通过 shell 包装执行 | 管道、重定向、引号参数 |
| `http` | HTTP 请求（GET/POST/PUT/DELETE/PATCH） | 微服务编排、API 调用、webhook |
| `llm` | 通过 LLM CLI（claude, opencode, nga...） | 已有 CLI 工具的场景 |
| `llm_sdk` | 通过 Anthropic Python SDK + ToolBridge | 工具调用、人工介入、自定义工具 |

### llm_sdk 内置工具（ToolBridge）

`llm_sdk` 节点通过 ToolBridge 自动获得以下工具：

| 工具 | 说明 |
|------|------|
| `read_file` | 读取文件或列出目录 |
| `write_file` | 写入文件（自动 .bak 备份） |
| `execute_command` | 执行 shell 命令 |

可选：通过 `~/.nexus/mcp.json` 配置 MCP servers，ToolBridge 自动发现并注册工具。

## 工作流定义格式

最小工作流示例 (`examples/` 目录下有更多):

```json
{
  "nodes": [
    {
      "id": "fetch",
      "providers": [{ "type": "http", "url": "https://api.example.com/data", "method": "GET" }],
      "process_timeout_secs": 30
    },
    {
      "id": "validate",
      "providers": [{ "type": "subprocess", "command": "python validator.py" }],
      "process_timeout_secs": 10
    }
  ],
  "edges": [
    { "from": "fetch", "to": "validate", "trigger": "all", "event": "complete" }
  ],
  "dataflows": [
    { "from": "fetch", "to": "validate" }
  ]
}
```

## route_policy — 循环终止控制

工作流中的有向环必须配置终止策略：

| 策略 | 说明 | 示例 |
|------|------|------|
| `max_runs` | 运行 N 轮后强制切换路由 | `{"type":"max_runs","max":3,"then_route":"approved"}` |
| `max_duration` | 累计执行 N 秒后强制切换路由 | `{"type":"max_duration","max_secs":300,"then_route":"timeout"}` |

## 模板变量

在 `command`、`prompt`、`url`、`body` 中可用：

| 变量 | 展开为 |
|------|--------|
| `{{datarouter.X.content}}` | 上游节点 X 的输出文本 |
| `{{datarouter.X.route}}` | 上游节点 X 的路由值 |
| `{{metadata.run_count}}` | 当前执行轮次（1-based） |
| `{{metadata.timed_out}}` | 上次执行是否超时 |
| `{{node_dir}}` | 当前节点的 scripts 目录路径 |

## 目录结构

```
release/
├── bin/                          # 预编译二进制文件
│   ├── nexus-cli.exe
│   ├── nexus-dashboard.exe
│   ├── nexus-mcp-server.exe
│   └── linux/                    # Linux 版本
├── scripts/                      # 运行时脚本
│   ├── llm_node.py               # LLM CLI 节点 wrapper
│   ├── llm_sdk.py                # LLM SDK 节点 wrapper
│   ├── nexus_protocol.py         # 共享协议层（stdin/stdout/parse/sanitize）
│   └── tool_bridge.py            # ToolBridge（MCP + 内置工具）
├── static/
│   ├── index.html                # Dashboard SPA
│   ├── arch-review-loop.json     # 架构检视工作流
│   └── review-loop.json          # 通用检视循环工作流
├── examples/                     # 示例工作流 JSON
│   ├── http-test.json            # HTTP GET 示例
│   ├── http-test-post.json       # HTTP POST 示例
│   ├── http-test-branch.json     # HTTP 分支路由 + 错误处理
│   ├── http-test-demo.json       # HTTP 综合演示
│   ├── auto-review-fix-loop.json # 代码审查修复循环
│   ├── design-review-fix-loop.json # 设计文档审查修复循环
│   ├── max-duration-test.json    # max_duration 策略演示
│   └── load_http_tests.py        # 一键加载 HTTP 测试到 Dashboard
├── README.md
├── QUICKSTART.md                 # 快速入门指南
├── WORKFLOW_REFERENCE.md         # 工作流定义完整参考
└── NEXUS_WORKFLOW_SKILL.md       # Claude Code Skill 参考
```

## 系统要求

- **Windows 10+ (x86_64)** — 提供预编译二进制文件
- **Linux (x86_64)** — 提供预编译二进制文件（`bin/linux/`）
- **macOS** — 需从源码构建（见下方构建说明）
- 预编译二进制文件开箱即用，无需安装 VC++ Redistributable 等额外运行时
- **`type: "llm"` 节点依赖**：Python 3 + Claude Code CLI (`claude`)
- **`type: "llm_sdk"` 节点依赖**：Python 3 + `pip install anthropic` + API key
- `.bat` 脚本仅适用于 Windows。Linux/macOS 需用 shell 等价脚本

## 环境变量

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `NEXUS_LLM_WRAPPER` | 自动检测 | LLM node Python wrapper 路径 |
| `NEXUS_LLM_SDK_WRAPPER` | 自动检测 | LLM SDK Python wrapper 路径 |
| `ANTHROPIC_API_KEY` | — | Anthropic API 密钥 |
| `ANTHROPIC_BASE_URL` | `https://api.anthropic.com` | API 端点 URL |
| `NEXUS_HOST` | `127.0.0.1` | Dashboard HTTP 监听地址 |
| `NEXUS_PORT` | `48080` | Dashboard HTTP 监听端口 |
| `NEXUS_SCRIPTS_DIR` | 自动检测 | 全局脚本目录 |
| `NEXUS_MCP_CONFIG` | `~/.nexus/mcp.json` | MCP server 配置文件路径 |
| `NEXUS_HUMAN_DIR` | `./tmp/human_io` | 人工介入问答文件目录 |

## 构建

### Windows
```bash
cd engine
cargo build --release
# 产物在 engine/target/release/
```

### Linux / macOS
```bash
cd engine
cargo build --release
# 产物在 engine/target/release/
# 将 bin/* 和 scripts/ 放到同一目录下即可组成 release 包
```

> **注意**：Linux/macOS 构建时需要 `libsqlite3-dev` (Linux) 或 Xcode Command Line Tools (macOS)。
