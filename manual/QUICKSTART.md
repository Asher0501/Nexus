# 快速开始

> 5 分钟内运行你的第一个 Nexus 工作流。

**分类**：manual

---

## 安装

```bash
cargo build --release
```

产物 `target/release/nexus-cli.exe`（~1.5MB 静态链接），丢到任何 Windows/Linux 机器就能跑。零运行时依赖。

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
nexus-cli run workflow.json
```

## 验证工作流

先验证 JSON 结构和工作流拓扑：

```bash
nexus-cli run workflow.json --validate-only
```

## 看详细输出

```bash
nexus-cli run workflow.json --verbose
```

查看每行 stdout 实时输出。

## 查看节点最终状态

```bash
nexus-cli run workflow.json --dump-state
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

## 更多

- **[WORKFLOW_REFERENCE.md](./WORKFLOW_REFERENCE.md)** — 工作流定义完整参考
- **[CLI_REFERENCE.md](./CLI_REFERENCE.md)** — CLI 全部参数说明
- **[AGENT_GUIDE.md](./AGENT_GUIDE.md)** — Agent 编写工作流的指南
