# Nexus v0.1.0

Nexus 是一个有向图驱动的插件编排引擎。定义工作流 DAG → 引擎按调度边与数据流边自动执行。

## 包含的二进制文件

| 平台 | 文件 | 大小 | 说明 |
|------|------|------|------|
| Windows | `bin/nexus-cli.exe` | ~1.7 MB | 命令行工作流执行器 |
| Windows | `bin/nexus-dashboard.exe` | ~4 MB | HTTP REST API + WebSocket 服务端（端口 48080） |
| Windows | `bin/nexus-mcp-server.exe` | ~0.9 MB | JSON-RPC stdio 服务端（可接入 MCP 客户端） |
| Linux | `bin/linux/nexus-cli` | ~1.9 MB | 命令行工作流执行器 |
| Linux | `bin/linux/nexus-dashboard` | ~4 MB | HTTP REST API + WebSocket 服务端（端口 48080） |
| Linux | `bin/linux/nexus-mcp-server` | ~1 MB | JSON-RPC stdio 服务端（可接入 MCP 客户端） |

## 快速开始

### 1. CLI 运行工作流

**Windows:**
```bash
cd release
./bin/nexus-cli run examples/claude-test.json --verbose
```

**Linux:**
```bash
cd release
./bin/linux/nexus-cli run examples/claude-test.json --verbose
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
| `POST` | `/api/workflows/{id}/run` | 触发运行 |
| `GET` | `/api/runs` | 列出所有运行记录 |
| `GET` | `/api/runs/{id}` | 获取运行详情 |
| `WS` | `/ws/runs/{run_id}` | WebSocket 实时状态推送 |

### 4. MCP Server（stdio 模式）

**Windows:**
```bash
echo '{"jsonrpc":"2.0","method":"validate_workflow","params":{"workflow_json":"{\"nodes\":[{\"id\":\"hello\",\"providers\":[{\"type\":\"subprocess\",\"command\":\"echo hi\"}],\"process_timeout_secs\":10}]}"},"id":1}' | ./bin/nexus-mcp-server.exe
```
**Linux:**
```bash
echo '{"jsonrpc":"2.0","method":"validate_workflow","params":{"workflow_json":"{\"nodes\":[{\"id\":\"hello\",\"providers\":[{\"type\":\"subprocess\",\"command\":\"echo hi\"}],\"process_timeout_secs\":10}]}"},"id":1}' | ./bin/linux/nexus-mcp-server
```

支持的方法: `validate_workflow` | `parse_workflow` | `describe_schema` | `run_workflow`

## 工作流定义格式

最小工作流示例 (`examples/` 目录下有更多):

```json
{
  "nodes": [
    {
      "id": "fetch",
      "providers": [{ "type": "subprocess", "command": "python fetcher.py" }],
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
  ]
}
```

## 目录结构

```
release/
├── bin/
│   ├── nexus-cli.exe              # Windows CLI
│   ├── nexus-dashboard.exe        # Windows Dashboard
│   ├── nexus-mcp-server.exe       # Windows MCP Server
│   └── linux/
│       ├── nexus-cli              # Linux CLI
│       ├── nexus-dashboard        # Linux Dashboard
│       └── nexus-mcp-server       # Linux MCP Server
├── scripts/
│   ├── llm_node.py                # LLM provider 运行时依赖
│   ├── start_node.py / review_node.py / fix_opencode.py / retro_node.py
│   ├── reviewer_emit_approved.py / handler_approved.py / handler_rejected.py
│   └── review_design_philosophy.py
├── examples/                      # 示例工作流 JSON
├── QUICKSTART.md                  # 快速入门指南
├── WORKFLOW_REFERENCE.md          # 工作流定义完整参考
└── README.md                      # 本文件
```

## 系统要求

- **Windows 10+ (x86_64)** — 提供预编译二进制文件
- **Linux / macOS** — 需从源码构建（见下方构建说明）
- 预编译二进制文件开箱即用，无需安装 VC++ Redistributable 等额外运行时
- **`type: "llm"` 节点依赖**：
  - **Python 3** — 引擎通过内置 `scripts/llm_node.py` wrapper 调用 LLM CLI。需在系统 PATH 中
  - **Claude Code CLI** (`claude`) — LLM 节点的默认 CLI。通过 npm 安装：`npm install -g @anthropic-ai/claude-code`
  - 也可替换为其他 CLI（opencode、nga 等），修改 `command` 字段即可
- **`.bat` 脚本仅适用于 Windows**。Linux/macOS 需用 shell 等价脚本（如 `echo '{"route":"ok","content":"done"}'`）
- 二进制文件可放到任意目录或加入 `PATH`。命令中的 `scripts/` 相对路径会自动解析为 exe 所在目录的 `../scripts/`，无需特意 cd 到 release 目录
- 如使用自定义 `scripts/` 路径，可通过 `NEXUS_LLM_WRAPPER` 环境变量覆盖

## 环境变量

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `NEXUS_LLM_WRAPPER` | 自动检测 | LLM node Python wrapper 路径。默认基于 exe 位置自动解析（`{exe_dir}/../scripts/llm_node.py`），也可手动设置为绝对路径 |
| `NEXUS_HOST` | `127.0.0.1` | Dashboard HTTP 监听地址 |
| `NEXUS_PORT` | `48080` | Dashboard HTTP 监听端口 |

示例：自定义端口和 LLM wrapper 路径：

```bash
# Linux / Git Bash（如自定义路径，用 NEXUS_LLM_WRAPPER 覆盖自动检测）
NEXUS_PORT=9090 NEXUS_LLM_WRAPPER=./scripts/llm_node.py ./bin/nexus-dashboard

# Windows CMD
set NEXUS_PORT=9090 && bin\nexus-dashboard.exe
```

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
