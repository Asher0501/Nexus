# Nexus Workflow Skill

根据用户的需求描述，自动生成 Nexus 工作流 JSON 定义。
**所有规则和模板都必须严格遵循 `references/WORKFLOW_REFERENCE.md` 的定义。**
本 skill 只列出快速参考，完整细节一律以 WORKFLOW_REFERENCE.md 为准。

## 触发词

- "nexus workflow"
- "生成工作流"
- "create workflow"
- "写一个工作流"

## 核心原则

1. **引擎不感知节点类型。** 所有节点统一通过 `type: "subprocess"` 接入。
2. **退出码判断完成。** exit 0 = 完成，非0 = 失败。这是判断进程结束的唯一可靠方式。
3. **stdout 逐行实时读取。** 节点不需要等退出——每行 stdout 在运行时实时上报（nexus::node::chunk）。
4. **调度拓扑 ≠ 数据拓扑。** `edges` 决定谁完成后谁开始，`dataflows` 决定谁的数据传给谁。两者独立。
5. **All/Any 是完备的。** 任意布尔逻辑都可以用 All（合取）和 Any（析取）在图上等价表达。
6. **所有 AI CLI 都是 subprocess。** 没有专门的 AI executor 或包装脚本。

## 生成工作流的步骤

### 步骤 1：分析需求

从用户描述中提取：
1. **节点数**：有几个独立的处理步骤？
2. **数据流**：哪个步骤需要哪个步骤的输出？
3. **调度依赖**：哪个步骤需要在哪个步骤之后执行？
4. **分支条件**：有没有需要根据结果走不同路径的步骤？
5. **节点类型**：所有节点是 subprocess 还是需要调用 AI CLI？

### 步骤 2：确定拓扑

**调度拓扑（edges）**：画出谁完成后谁应该开始。
- 入口节点 = edges 中没有入边的节点
- 扇出 = 多个节点同时依赖同一个上游
- 扇入 = 一个节点依赖多个上游全部完成
- 分支 = 一个节点根据不同结果触发不同下游

**数据拓扑（dataflows）**：画出谁的数据需要传给谁。
- 节点 B 需要节点 A 的输出 → `{"from": "A", "to": "B"}`
- 可以用 alias 重命名 key

### 步骤 3：编写工作流 JSON

按以下模板生成。所有 field 的完整定义见 `references/WORKFLOW_REFERENCE.md`。

#### 基础链式

```json
{
  "nodes": [
    {
      "id": "step1",
      "providers": [{"type": "subprocess", "command": "命令1"}],
      "process_timeout_secs": 30
    },
    {
      "id": "step2",
      "providers": [{"type": "subprocess", "command": "命令2"}],
      "process_timeout_secs": 30
    }
  ],
  "edges": [
    { "from": "step1", "to": "step2", "trigger": "all", "event": "complete" }
  ],
  "dataflows": [
    { "from": "step1", "to": "step2" }
  ]
}
```

#### 集成 AI（OpenCode / Claude Code）

所有 AI 工具直接写完整的 CLI 命令，不需要包装脚本：

**OpenCode（入口节点）：**
```json
{
  "nodes": [
    {
      "id": "ai_task",
      "providers": [{
        "type": "subprocess",
        "command": "opencode run --format json --dangerously-skip-permissions --model claude-sonnet-4 -- \"提示词内容\""
      }],
      "process_timeout_secs": 300
    }
  ]
}
```

**Claude Code（入口节点）：**
```json
{
  "nodes": [
    {
      "id": "ai_task",
      "providers": [{
        "type": "subprocess",
        "command": "claude -p \"提示词内容\" --output-format json --model claude-sonnet-4"
      }],
      "process_timeout_secs": 300
    }
  ]
}
```

**链式传递上游数据给 AI：**
```json
{
  "nodes": [
    {
      "id": "config",
      "providers": [{"type": "subprocess", "command": "echo 审查以下代码"}],
      "process_timeout_secs": 10
    },
    {
      "id": "ai_task",
      "providers": [{
        "type": "subprocess",
        "command": "opencode run --format json --dangerously-skip-permissions -- \"{{inputs.config}}\""
      }],
      "process_timeout_secs": 300
    }
  ],
  "edges": [
    { "from": "config", "to": "ai_task", "trigger": "all", "event": "complete" }
  ],
  "dataflows": [
    { "from": "config", "to": "ai_task" }
  ]
}
```

### 步骤 4：验证

生成完后检查：
1. 所有节点 ID 唯一
2. 入口节点在 `edges` 中没有入边
3. `process_timeout_secs` 对所有节点都设置了合理的值
4. `edges` 中的 `from` 和 `to` 都存在于 `nodes` 中
5. `dataflows` 中的 `from` 和 `to` 都存在于 `nodes` 中
6. 分支路由的 `returns` 和 `exit_reason` 值匹配
7. 模板插值 `{{inputs.X}}` 中的 X 必须在 `dataflows` 中有对应的 `from` → 该节点 ID 的声明

## 规则速查

| 规则 | 说明 |
|------|------|
| type | 仅支持 `"subprocess"` |
| 入口节点 | `edges` 中没有入边的节点 |
| 唯一 ID | 所有节点 id 不可重复 |
| threshold | 默认 1，大于 1 需要 N 次才触发 |
| exit_reason | 精确字符串匹配 |
| 模板插值 | `{{inputs.node_id}}` 在 spawn 前替换 |
| 退出码判断 | exit 0 = 完成，非0 = 失败 |

## 参考

- **完整参考（本地机密）**: `references/WORKFLOW_REFERENCE.md`（所有 schema、机制、模式模板、边界情况的唯一权威来源）
- **Architecture**: `references/WORKFLOW_REFERENCE.md §1-3`
- **Node Protocol**: `references/WORKFLOW_REFERENCE.md §4`
- **Mode Templates**: `references/WORKFLOW_REFERENCE.md §6`
- **Integration Examples**: `references/WORKFLOW_REFERENCE.md §7`
