# Nexus Workflow Skill

根据用户的需求描述，自动生成 Nexus 工作流 JSON 定义。

## 触发词

- "nexus workflow"
- "生成工作流"
- "create workflow"
- "写一个工作流"

## 工作流程式

```json
{
  "nodes": [
    {
      "id": "<节点 ID，简短英文>",
      "providers": [{"type": "subprocess", "command": "<命令>"}],
      "process_timeout_secs": <超时秒数>,
      "predecessors": [
        {"node_id": "<上游节点 ID>", "trigger": "all", "event": "complete"}
      ],
      "inputs": ["<上游节点 ID>"],
      "max_concurrency": <可选，非必填>,
      "returns": ["approved", "rejected"]
    }
  ]
}
```

## 规则

1. 每个节点必须有唯一的 `id`
2. 入口节点（无前驱）的 `predecessors` 为 `[]`
3. `providers` 目前只支持 `type: "subprocess"`，command 写实际要执行的命令
4. `process_timeout_secs` 是必填字段
5. `inputs` 声明需要哪些上游节点的输出
6. `predecessors[].trigger` 可选 `"all"` 或 `"any"`
7. `predecessors[].event` 可选 `"complete"`、`"failed"`、`"timeout"`
8. `predecessors[].threshold` 默认为 1，可填大于 1 的值表示需要 N 次事件才触发
9. `returns` 声明该节点的可能返回值，用于分支路由
10. `max_concurrency` 可以覆盖引擎全局并发数

## 节点协议

所有节点通过 stdin/stdout 与引擎通信：

- stdin：引擎写入 NodeContext JSON
- stdout：节点输出纯文本结果
- 首行以 `__nexus_exit_reason: <value>` 开头时为退出原因
- exit 0 = 成功，非 0 = 失败

节点可以用任何语言实现——Python、PowerShell、Node.js、Rust 等。

## 示例

### 基础链式工作流

```
A → B → C
```

```json
{
  "nodes": [
    {
      "id": "fetch",
      "providers": [{"type": "subprocess", "command": "python fetcher.py"}],
      "process_timeout_secs": 30,
      "predecessors": []
    },
    {
      "id": "process",
      "providers": [{"type": "subprocess", "command": "python processor.py"}],
      "process_timeout_secs": 60,
      "predecessors": [
        {"node_id": "fetch", "trigger": "all", "event": "complete"}
      ],
      "inputs": ["fetch"]
    },
    {
      "id": "report",
      "providers": [{"type": "subprocess", "command": "python reporter.py"}],
      "process_timeout_secs": 30,
      "predecessors": [
        {"node_id": "process", "trigger": "all", "event": "complete"}
      ],
      "inputs": ["process"]
    }
  ]
}
```

### Fan-out / Fan-in

```
    ┌→ B ─┐
  A ┤     ├→ D
    └→ C ─┘
```

```json
{
  "nodes": [
    {
      "id": "source",
      "providers": [{"type": "subprocess", "command": "echo data"}],
      "process_timeout_secs": 10,
      "predecessors": []
    },
    {
      "id": "branch_a",
      "providers": [{"type": "subprocess", "command": "python a.py"}],
      "process_timeout_secs": 30,
      "predecessors": [
        {"node_id": "source", "trigger": "all", "event": "complete"}
      ],
      "inputs": ["source"]
    },
    {
      "id": "branch_b",
      "providers": [{"type": "subprocess", "command": "python b.py"}],
      "process_timeout_secs": 30,
      "predecessors": [
        {"node_id": "source", "trigger": "all", "event": "complete"}
      ],
      "inputs": ["source"]
    },
    {
      "id": "merge",
      "providers": [{"type": "subprocess", "command": "python merge.py"}],
      "process_timeout_secs": 30,
      "predecessors": [
        {"node_id": "branch_a", "trigger": "all", "event": "complete"},
        {"node_id": "branch_b", "trigger": "all", "event": "complete"}
      ],
      "inputs": ["branch_a", "branch_b"]
    }
  ]
}
```

### 分支路由

```json
{
  "nodes": [
    {
      "id": "review",
      "providers": [{"type": "subprocess", "command": "python reviewer.py"}],
      "process_timeout_secs": 60,
      "predecessors": [
        {"node_id": "source", "trigger": "all", "event": "complete"}
      ],
      "inputs": ["source"],
      "returns": ["approved", "rejected"]
    },
    {
      "id": "deploy",
      "providers": [{"type": "subprocess", "command": "python deploy.py"}],
      "process_timeout_secs": 120,
      "predecessors": [
        {"node_id": "review", "trigger": "all", "event": "complete", "exit_reason": "approved"}
      ]
    },
    {
      "id": "notify_rejected",
      "providers": [{"type": "subprocess", "command": "python notify.py rejected"}],
      "process_timeout_secs": 10,
      "predecessors": [
        {"node_id": "review", "trigger": "all", "event": "complete", "exit_reason": "rejected"}
      ]
    }
  ]
}
```

### 集成 OpenCode

```json
{
  "nodes": [
    {
      "id": "config",
      "providers": [{"type": "subprocess", "command": "cmd.exe /c echo {\"prompt\":\"...\",\"model\":\"...\"}"}],
      "process_timeout_secs": 10,
      "predecessors": []
    },
    {
      "id": "ai_task",
      "providers": [{"type": "subprocess", "command": "powershell -ExecutionPolicy Bypass -File node-opencode.ps1"}],
      "process_timeout_secs": 300,
      "predecessors": [
        {"node_id": "config", "trigger": "all", "event": "complete"}
      ],
      "inputs": ["config"]
    }
  ]
}
```

## 参考

- NODE_PROTOCOL: `docs/architecture/NODE_PROTOCOL.md`
- 架构设计: `docs/architecture/ARCHITECTURE.md`
- 完整参考: `docs/REFERENCE.md`（面向 agent 的 schema 和机制说明）
