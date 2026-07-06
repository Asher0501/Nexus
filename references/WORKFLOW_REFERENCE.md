# Nexus Workflow Reference — Agent Manual

> 面向 agent 的工作流定义完整参考手册。包含全部 schema、机制说明、模式模板和边界情况。
> Agent 在生成工作流 JSON 时必须严格遵循本手册的所有规则。
> 本手册是生成工作流的唯一权威依据。

---

## 目录

1. [工作流定义（WorkflowDef）](#1-工作流定义workflowdef)
   - [1.1 顶层结构](#11-顶层结构)
   - [1.2 NodeDef — 节点定义](#12-nodedef--节点定义)
   - [1.3 ProviderDef — 执行方式](#13-providerdef--执行方式)
   - [1.4 SchedulingEdgeDef — 调度边](#14-schedulingedgedef--调度边)
   - [1.5 DataFlowDef — 数据流](#15-dataflowdef--数据流)
   - [1.6 PredecessorDef — 前驱声明](#16-predecessordef--前驱声明)
2. [调度语义](#2-调度语义)
   - [2.1 边触发算法](#21-边触发算法)
   - [2.2 All vs Any](#22-all-vs-any)
   - [2.3 事件类型](#23-事件类型)
   - [2.4 阈值（threshold）](#24-阈值threshold)
   - [2.5 分支路由（exit_reason）](#25-分支路由exit_reason)
   - [2.6 重试机制](#26-重试机制)
   - [2.7 并发控制](#27-并发控制)
3. [数据路由](#3-数据路由)
   - [3.1 DataRouter 语义](#31-datarouter-语义)
   - [3.2 dataflows 声明](#32-dataflows-声明)
   - [3.3 snapshot semantics](#33-snapshot-semantics)
4. [节点协议（NODE_PROTOCOL）](#4-节点协议node_protocol)
   - [4.1 通信流程](#41-通信流程)
   - [4.2 stdin 输入格式](#42-stdin-输入格式)
   - [4.3 stdout 输出格式](#43-stdout-输出格式)
   - [4.4 行协议前缀](#44-行协议前缀)
   - [4.5 退出码](#45-退出码)
   - [4.6 exit_reason 设置](#46-exit_reason-设置)
5. [流式输出](#5-流式输出)
   - [5.1 NodeChunk 机制](#51-nodechunk-机制)
   - [5.2 实时事件上报](#52-实时事件上报)
   - [5.3 模板插值](#53-模板插值)
6. [模式模板](#6-模式模板)
   - [6.1 链式（Sequential Chain）](#61-链式sequential-chain)
   - [6.2 扇出/扇入（Fan-out / Fan-in）](#62-扇出扇入fan-out--fan-in)
   - [6.3 条件分支（Conditional Branch）](#63-条件分支conditional-branch)
   - [6.4 带阈值的自环（Threshold Loop）](#64-带阈值的自环threshold-loop)
   - [6.5 入口/出口边界（Entry/Exit Boundary）](#65-入口出口边界entryexit-boundary)
   - [6.6 并行聚合（Parallel Aggregation）](#66-并行聚合parallel-aggregation)
   - [6.7 错误处理（Error Handling）](#67-错误处理error-handling)
7. [集成示例](#7-集成示例)
   - [7.1 OpenCode 代码审查](#71-opencode-代码审查)
   - [7.2 Claude Code 代码重构](#72-claude-code-代码重构)
   - [7.3 链式 AI 处理管道](#73-链式-ai-处理管道)
   - [7.4 逐级上报审批流程](#74-逐级上报审批流程)
8. [边界情况与限制](#8-边界情况与限制)

---

## 1. 工作流定义（WorkflowDef）

### 1.1 顶层结构

```json
{
  "nodes": [ ... ],
  "edges": [ ... ],
  "dataflows": [ ... ]
}
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `nodes` | `NodeDef[]` | ✅ | — | 工作流中的所有节点 |
| `edges` | `SchedulingEdgeDef[]` | ❌ | `[]` | 调度拓扑边（谁完成后谁可以开始） |
| `dataflows` | `DataFlowDef[]` | ❌ | `[]` | 数据拓扑边（谁的数据传给谁） |

**关键规则：**
- `edges` 和 `dataflows` 是两张**独立的图**。调度拓扑决定触发顺序，数据拓扑决定数据流向。两者不需要一致。
- 入口节点 = 在 `edges` 中没有入边的节点。Builder 自动识别。
- 至少有一个入口节点，否则 Validator 报 `NoEntryNode`。
- 所有节点都必须在调度拓扑中从入口可达，否则报 `UnreachableNode`。

---

### 1.2 NodeDef — 节点定义

```json
{
  "id": "fetch_data",
  "providers": [{ "type": "subprocess", "command": "python fetcher.py" }],
  "process_timeout_secs": 30,
  "max_concurrency": null,
  "returns": ["approved", "rejected"],
  "max_retries": null
}
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `id` | `string` | ✅ | — | 唯一标识符，在整个工作流中不可重复 |
| `providers` | `ProviderDef[]` | ✅ | — | 执行方式数组，至少一个有效 provider |
| `process_timeout_secs` | `u64` | ✅ | — | 节点超时秒数。超时后引擎强杀进程，按 Timeout 事件处理 |
| `max_concurrency` | `u64 \| null` | ❌ | `null` | 该节点的最大并发执行数。`null` = 继承引擎全局 `max_concurrency` |
| `returns` | `string[]` | ❌ | `[]` | 分支路由的可选返回值列表。空数组 = 不启用分支路由 |
| `max_retries` | `u64 \| null` | ❌ | `null` | 最大重试次数。`null` = 继承引擎全局默认值 3 |

**约束：**
- `id` 必须在整个 `WorkflowDef.nodes[]` 中唯一。重复则报 `DuplicateNodeId`。
- `providers` 为空数组时，Validator 报 `NoValidProvider`。
- `returns` 非空时，节点可以通过 stdout 首行 `__nexus_exit_reason:` 设置返回值，引擎据此做分支路由。
- `process_timeout_secs` 必须 > 0。

---

### 1.3 ProviderDef — 执行方式

当前仅支持 `subprocess` 类型：

```json
{
  "type": "subprocess",
  "command": "python fetcher.py --url {{inputs.url}}"
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `type` | `"subprocess"` | ✅ | 唯一有效值。所有工具统一通过 subprocess 接入 |
| `command` | `string` | ✅ | 要执行的命令。支持 `{{inputs.node_id}}` 模板插值 |

**关于 command 的规则：**
- command 是一个完整的命令行字符串，引擎按空格拆分为 `[program, arg1, arg2, ...]`
- 如果有复杂的引号/空格，建议用包装脚本（如 `cmd.exe /c`、`powershell -File`）
- `{{inputs.node_id}}` 在执行时替换为上游 node_id 的输出内容
- 不支持管道 `|`、重定向 `>` 等 shell 特性——需要用 `cmd /c` 或包装脚本

---

### 1.4 SchedulingEdgeDef — 调度边

```json
{
  "from": "fetch_data",
  "to": "validate",
  "trigger": "all",
  "event": "complete",
  "exit_reason": null,
  "threshold": 1
}
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `from` | `string` | ✅ | — | 上游节点 ID（数据/事件来源） |
| `to` | `string` | ✅ | — | 下游节点 ID（目标节点） |
| `trigger` | `"all" \| "any"` | ✅ | — | 组合逻辑。All = 所有上游到齐后才计数；Any = 任一上游事件直接计数 |
| `event` | `"complete" \| "failed" \| "timeout"` | ✅ | — | 匹配的事件类型 |
| `exit_reason` | `string \| null` | ❌ | `null` | 匹配的 exit_reason 值。`null` = 不检查 exit_reason |
| `threshold` | `u64` | ❌ | `1` | 触发所需的事件次数 |

**关键规则：**
- 同一条 `(from, to)` 组合可以出现多次（不同 event/exit_reason），Builder 按 `(to, trigger, event, exit_reason, threshold)` 聚合
- `from` 和 `to` 引用的节点 ID 必须存在于 `nodes[]` 中，否则报 `InvalidEdgeSource` / `InvalidEdgeTarget`
- `threshold` 默认 1。大于 1 时，需要触发 N 次相同事件才触发下游

---

### 1.5 DataFlowDef — 数据流

```json
{
  "from": "fetch_data",
  "to": "validate",
  "alias": "input_code"
}
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `from` | `string` | ✅ | — | 上游节点 ID（数据来源） |
| `to` | `string` | ✅ | — | 下游节点 ID（数据去向） |
| `alias` | `string \| null` | ❌ | `from` 的值 | 数据在目标节点 inputs 中的 key 名称 |

**关键规则：**
- `dataflows` 可以引用非直接前驱节点——数据路由不依赖调度顺序
- `from` 和 `to` 引用的节点 ID 必须存在于 `nodes[]` 中，否则报 `InputSourceNotFound`
- `from` 节点必须从入口可达，否则报 `InputSourceUnreachable`

---

### 1.6 PredecessorDef — 前驱声明

```json
{
  "node_id": "fetch_data",
  "trigger": "all",
  "event": "complete",
  "exit_reason": null,
  "threshold": 1
}
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `node_id` | `string` | ✅ | — | 上游节点 ID |
| `trigger` | `"all" \| "any"` | ✅ | — | 组合逻辑 |
| `event` | `"complete" \| "failed" \| "timeout"` | ✅ | — | 匹配的事件类型 |
| `exit_reason` | `string \| null` | ❌ | `null` | 匹配的 exit_reason 值 |
| `threshold` | `u64` | ❌ | `1` | 触发所需的事件次数 |

> PredecessorDef 是 `NodeDef` 字段（旧语法），现在推荐直接在顶层 `edges[]` 声明调度拓扑。两者语义等价，但 `edges[]` 更清晰。

---

## 2. 调度语义

### 2.1 边触发算法

```
for each out_edge of completed_node:
  1. 如果边已被触发（triggered=true），跳过
  2. 如果事件类型不匹配，跳过
  3. 如果 exit_reason 配置了且不匹配，跳过
  4. All 策略：如果 received 集合未包含所有 from_nodes，跳过
  5. event_count++
  6. 如果 event_count >= threshold 且 !triggered：
     → triggered = true
     → 目标节点入队（NodeReady）
```

### 2.2 All vs Any

| 策略 | 行为 | 适用场景 |
|------|------|---------|
| **All** | 所有上游至少参与一次后才开始累计 event_count。同一源节点的重复事件不重复计数 | 并行分支全部完成后才触发合并节点 |
| **Any** | 任何上游的事件都直接计入 event_count，不需要等所有上游到齐 | 任意一个分支完成就触发 |

**组合示例：**
```
{B, C} ──All/Complete/threshold=3──→ D
{A}    ──Any/Timeout/threshold=1──→ E
```

D 就绪条件 = (B 和 C 都正常完成且 Complete 事件合计 >= 3 次) OR (A 超时 1 次)

### 2.3 事件类型

| 事件类型 | 触发条件 | 说明 |
|---------|---------|------|
| `complete` | exit_code = 0 | 正常完成 |
| `failed` | exit_code ≠ 0 | 节点自己报告异常 |
| `timeout` | 超时强杀 | 引擎在 `process_timeout_secs` 后 kill 进程 |

三种事件完全对称——引擎的机械计数和触发出边逻辑对三者一视同仁。

### 2.4 阈值（threshold）

- 默认值：1
- 表示"事件发生 N 次后才触发下游"
- `threshold > 1` 常用于自环节点（节点完成后触发自己再次执行）
- 阈值保证了环的终止：环内至少有一个节点的边配置了 `threshold > 1`

### 2.5 分支路由（exit_reason）

节点通过 `returns` 声明可能的返回值。运行后在 stdout 首行写 `__nexus_exit_reason: <value>` 设置实际值：

```json
{
  "id": "review_node",
  "returns": ["approved", "rejected"]
}
```

```
# 节点 stdout：
__nexus_exit_reason: approved
业务输出内容...
```

引擎按 exit_reason 匹配出边：
```
review_node ──Complete/exit_reason="approved"──→ deploy_node
review_node ──Complete/exit_reason="rejected"──→ fix_node
```

**规则：**
- `exit_reason` 是字符串精确匹配。`"approved"` 和 `"approved "` 不同。
- 如果 `edges` 中声明了 `exit_reason` 过滤，只有精确匹配的边才触发。
- 如果节点没有设置 `returns`，引擎忽略 exit_reason。

### 2.6 重试机制

```
节点 Failed/Timeout
  → retry_count = retry_counts[node]
  → if retry_count < max_retries (默认 3):
      retry_counts[node]++
      send(NodeReady(node))          ← 重新执行，不触发下游边
  → else:
      正常触发 Failed/Timeout 出边
```

| 级别 | 配置项 | 默认值 |
|------|--------|--------|
| 引擎全局 | `EngineConfig.max_retries` | 3 |
| 节点级 | `NodeDef.max_retries` | 继承全局 |

### 2.7 并发控制

引擎使用 `tokio::sync::Semaphore` 限制同时执行的节点数：

| 配置来源 | 字段 | 默认值 |
|---------|------|--------|
| 引擎全局 | `EngineConfig.max_concurrency` | CPU 核数 |

许可用尽时，`acquire().await` 挂起当前协程直到有节点完成释放槽位。

---

## 3. 数据路由

### 3.1 DataRouter 语义

DataRouter 由 `dataflows[]` 驱动（而非 `NodeDef.inputs`）。它不关心调度拓扑，只做存储和转发。

### 3.2 dataflows 声明

```json
"dataflows": [
  { "from": "fetch_data", "to": "validate" }
]
```

等价于：validate 节点的 inputs 中有一个 key 为 `fetch_data`，值为 fetch_data 的最新输出。

也可以用 `alias` 重命名：
```json
{ "from": "fetch_data", "to": "validate", "alias": "input_code" }
```

此时 validate 的 inputs 中 key 为 `input_code`，值为 fetch_data 的最新输出。

### 3.3 Snapshot Semantics

- 每个节点只保留**最新一次**的执行输出
- 当目标节点触发时，DataRouter 按 `dataflows` 中声明的 `from → to` 关系组装输入
- **不保证**这些输出来自"同一轮"执行
- 尚未执行的源节点返回空字符串

---

## 4. 节点协议（NODE_PROTOCOL）

### 4.1 通信流程

```
引擎 → spawn(你的命令)
引擎 → stdin 写入 NodeContext JSON
引擎 → 关闭 stdin（子进程收到 EOF）
节点 → 读取 stdin JSON → 计算
节点 → stdout 写入纯文本结果（逐行实时读取）
节点 → exit 0（完成）／非0（失败）
引擎 → 每行 stdout 实时上报（nexus::node::chunk）
引擎 → 进程退出后存储完整输出到 DataRouter → 触发下游
```

### 4.2 stdin 输入格式

```json
{
  "inputs": {
    "source_node_id": "上游节点输出的纯文本"
  },
  "extensions": {}
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `inputs` | `object` | 上游节点的输出。key = 来源节点 ID，value = 该节点输出的纯文本 |
| `extensions` | `object` | 节点类型特有的配置参数（保留字段） |

### 4.3 stdout 输出格式

向 stdout 写入纯文本结果。引擎**逐行实时读取**。

### 4.4 行协议前缀

| 前缀 | 含义 | 是否进入业务输出 |
|------|------|----------------|
| `__nexus_log: TEXT` | 中间日志（tracing::info!） | ❌ |
| `__nexus_event: TEXT` | 结构化输出片段 | ✅ — 追加到 output_buf |
| `__nexus_exit_reason: VALUE` | 设置分支路由返回值 | ❌ — 仅设置 exit_reason |
| `__nexus_log_end` | 取消后续所有行的前缀检查 | — |
| 无前缀 | 普通输出行 | ✅ — 追加到 output_buf |

**实时性：** 每行 stdout 在到达时立即通过 `nexus::node::chunk` 事件上报，不需要等进程退出。

### 4.5 退出码

| 退出码 | 含义 | 引擎行为 |
|--------|------|---------|
| 0 | 正常完成 | 采用 stdout 内容作为节点输出 |
| 非0 | 节点自己报告异常 | 走失败处理路径（触发 Failed 出边） |
| 被信号杀死 | 异常终止 | 走失败处理路径 |

### 4.6 exit_reason 设置

两种方式：
1. **stdout 行协议：** 在 stdout 中写入 `__nexus_exit_reason: <value>`（运行时实时设置，推荐）
2. **节点返回值逻辑：** 通过 `returns` 声明 + 退出码（引擎侧处理）

---

## 5. 流式输出

### 5.1 NodeChunk 机制

引擎为每个执行中的节点创建独立的 chunk channel（`mpsc::unbounded_channel`）。

每行 stdout 到达时：
1. 实时发送到 chunk channel
2. 后台消费任务通过 `tracing::info!(target: "nexus::node::chunk")` 上报
3. 同时追加到 `NodeOutcome.output` 缓冲区
4. 进程退出后，完整 output 进入 DataRouter

### 5.2 实时事件上报

`--verbose` 模式下，`nexus::node::chunk` 事件在终端可见：

```
2026-07-06T12:27:05Z INFO nexus::node::chunk: node_id="review" text="step_start"
2026-07-06T12:27:05Z INFO nexus::node::chunk: node_id="review" text="text"
2026-07-06T12:27:05Z INFO nexus::node::chunk: node_id="review" text="step_finish"
```

### 5.3 模板插值

命令字符串中的 `{{inputs.node_id}}` 在执行前被 SubprocessExecutor 替换为上游节点的输出。

```json
{
  "id": "review",
  "providers": [{
    "type": "subprocess",
    "command": "opencode run --format json -- \"{{inputs.config}}\""
  }],
  "inputs": ["config"]
}
```

**规则：**
- 模板语法：`{{inputs.节点ID}}`
- 替换发生在 spawn 之前
- 不存在的 key 被替换为空字符串
- 普通文本不受影响

---

## 6. 模式模板

### 6.1 链式（Sequential Chain）

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

### 6.2 扇出/扇入（Fan-out / Fan-in）

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

### 6.3 条件分支（Conditional Branch）

```
        ┌→ approved → deploy
review ─┤
        └→ rejected → fix
```

```json
{
  "nodes": [
    {
      "id": "review",
      "providers": [{"type": "subprocess", "command": "python reviewer.py"}],
      "process_timeout_secs": 60,
      "predecessors": [{"node_id": "source", "trigger": "all", "event": "complete"}],
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
      "id": "fix",
      "providers": [{"type": "subprocess", "command": "python fix.py"}],
      "process_timeout_secs": 60,
      "predecessors": [
        {"node_id": "review", "trigger": "all", "event": "complete", "exit_reason": "rejected"}
      ]
    }
  ]
}
```

### 6.4 带阈值的自环（Threshold Loop）

```
A ──Complete/threshold=3──→ A  (自环 3 次后触发 B)
  ──Complete/threshold=3──→ B
```

```json
{
  "nodes": [
    {
      "id": "collector",
      "providers": [{"type": "subprocess", "command": "python collector.py"}],
      "process_timeout_secs": 30,
      "predecessors": [
        {"node_id": "collector", "trigger": "all", "event": "complete", "threshold": 3},
        {"node_id": "start", "trigger": "all", "event": "complete"}
      ]
    },
    {
      "id": "aggregator",
      "providers": [{"type": "subprocess", "command": "python aggregator.py"}],
      "process_timeout_secs": 30,
      "predecessors": [
        {"node_id": "collector", "trigger": "all", "event": "complete", "threshold": 3}
      ],
      "inputs": ["collector"]
    }
  ]
}
```

### 6.5 入口/出口边界（Entry/Exit Boundary）

- 入口节点：`edges` 中没有入边的节点
- 出口节点：`edges` 中没有出边的节点
- 单个节点可以同时是入口和出口

```json
{
  "nodes": [
    {
      "id": "single_node",
      "providers": [{"type": "subprocess", "command": "python task.py"}],
      "process_timeout_secs": 30,
      "predecessors": []
    }
  ]
}
```

### 6.6 并行聚合（Parallel Aggregation）

```
  A ──→┐
  B ──→ M ──→ D (All, threshold=1)
  C ──→┘
```

```json
{
  "nodes": [
    {"id": "a", "providers": [{"type":"subprocess","command":"a.py"}], "process_timeout_secs": 10, "predecessors": []},
    {"id": "b", "providers": [{"type":"subprocess","command":"b.py"}], "process_timeout_secs": 10, "predecessors": []},
    {"id": "c", "providers": [{"type":"subprocess","command":"c.py"}], "process_timeout_secs": 10, "predecessors": []},
    {
      "id": "merge",
      "providers": [{"type":"subprocess","command":"merge.py"}],
      "process_timeout_secs": 10,
      "predecessors": [
        {"node_id": "a", "trigger": "all", "event": "complete"},
        {"node_id": "b", "trigger": "all", "event": "complete"},
        {"node_id": "c", "trigger": "all", "event": "complete"}
      ],
      "inputs": ["a", "b", "c"]
    }
  ]
}
```

### 6.7 错误处理（Error Handling）

```
  A ──Complete──→ B
  A ──Failed ──→ error_handler
```

```json
{
  "nodes": [
    {
      "id": "risky_task",
      "providers": [{"type": "subprocess", "command": "python risky.py"}],
      "process_timeout_secs": 30,
      "predecessors": []
    },
    {
      "id": "on_success",
      "providers": [{"type": "subprocess", "command": "python success.py"}],
      "process_timeout_secs": 10,
      "predecessors": [
        {"node_id": "risky_task", "trigger": "all", "event": "complete"}
      ]
    },
    {
      "id": "on_failure",
      "providers": [{"type": "subprocess", "command": "python notify_failure.py"}],
      "process_timeout_secs": 10,
      "predecessors": [
        {"node_id": "risky_task", "trigger": "all", "event": "failed"}
      ]
    }
  ]
}
```

---

## 7. 集成示例

### 7.1 OpenCode 代码审查

```json
{
  "nodes": [{
    "id": "code_review",
    "providers": [{
      "type": "subprocess",
      "command": "opencode run --format json --dangerously-skip-permissions --model claude-sonnet-4 -- \"审查以下代码：$(cat src/main.ts)\""
    }],
    "process_timeout_secs": 300,
    "predecessors": []
  }]
}
```

### 7.2 Claude Code 代码重构

```json
{
  "nodes": [{
    "id": "refactor",
    "providers": [{
      "type": "subprocess",
      "command": "claude -p \"将 src/main.ts 重构为更模块化的结构\" --output-format json --allowedTools Read,Write,Edit"
    }],
    "process_timeout_secs": 600,
    "predecessors": []
  }]
}
```

### 7.3 链式 AI 处理管道

```
config (提供 prompt) → llm_review (AI review) → report (输出报告)
```

```json
{
  "nodes": [
    {
      "id": "config",
      "providers": [{"type": "subprocess", "command": "cmd /c echo 检查 src/ 下的所有 TypeScript 文件中的潜在 bug"}],
      "process_timeout_secs": 10,
      "predecessors": []
    },
    {
      "id": "llm_review",
      "providers": [{
        "type": "subprocess",
        "command": "opencode run --format json --dangerously-skip-permissions --model claude-sonnet-4 -- \"{{inputs.config}}\""
      }],
      "process_timeout_secs": 300,
      "predecessors": [{"node_id": "config", "trigger": "all", "event": "complete"}],
      "inputs": ["config"]
    },
    {
      "id": "report",
      "providers": [{"type": "subprocess", "command": "python save_report.py --input \"{{inputs.llm_review}}\""}],
      "process_timeout_secs": 10,
      "predecessors": [{"node_id": "llm_review", "trigger": "all", "event": "complete"}],
      "inputs": ["llm_review"]
    }
  ]
}
```

### 7.4 逐级上报审批流程

```
申请 → 组长审批 →（approved → 经理审批）→（approved → 实施）
                →（rejected → 驳回通知）
```

```json
{
  "nodes": [
    {
      "id": "request",
      "providers": [{"type": "subprocess", "command": "python submit_request.py"}],
      "process_timeout_secs": 10,
      "predecessors": []
    },
    {
      "id": "lead_review",
      "providers": [{"type": "subprocess", "command": "python lead_approve.py --input \"{{inputs.request}}\""}],
      "process_timeout_secs": 3600,
      "predecessors": [{"node_id": "request", "trigger": "all", "event": "complete"}],
      "inputs": ["request"],
      "returns": ["approved", "rejected"]
    },
    {
      "id": "manager_review",
      "providers": [{"type": "subprocess", "command": "python manager_approve.py --input \"{{inputs.lead_review}}\""}],
      "process_timeout_secs": 7200,
      "predecessors": [
        {"node_id": "lead_review", "trigger": "all", "event": "complete", "exit_reason": "approved"}
      ],
      "inputs": ["lead_review"],
      "returns": ["approved", "rejected"]
    },
    {
      "id": "rejected",
      "providers": [{"type": "subprocess", "command": "python notify_rejected.py"}],
      "process_timeout_secs": 10,
      "predecessors": [
        {"node_id": "lead_review", "trigger": "all", "event": "complete", "exit_reason": "rejected"}
      ]
    },
    {
      "id": "execute",
      "providers": [{"type": "subprocess", "command": "python execute.py"}],
      "process_timeout_secs": 300,
      "predecessors": [
        {"node_id": "manager_review", "trigger": "all", "event": "complete", "exit_reason": "approved"}
      ],
      "inputs": ["manager_review"]
    }
  ]
}
```

---

## 8. 边界情况与限制

### 8.1 空图

零节点的工作流是合法的，引擎立即收敛。

### 8.2 孤立节点

多个相互之间没有边连接的节点是合法的——它们同时作为入口节点被触发。

### 8.3 自环边

节点可以声明指向自己的边（自环）。自环边也需要遵循阈值的收敛约束：
- 如果自环边 `threshold = 1` 且节点没有其他出边 → Validator 报 `CycleWithoutEntry`
- 如果自环边 `threshold > 1` → 节点执行 N 次后触发下游

### 8.4 输出大小限制

- 单节点 stdout 输出不应超过 100MB
- 超出可能导致 OOM 或 pipe buffer 满导致的死锁
- 大量输出建议用文件传递模式

### 8.5 命令拆分的局限性

`SubprocessExecutor` 按空格拆分 command。以下情况需要包装脚本：
- 含引号的参数（`--format json` 没问题，但 `--name "John Doe"` 不行）
- shell 管道和重定向（`|`、`>`）
- 环境变量设置（需要 `cmd /c` 或包装脚本）

### 8.6 exit_reason 字符串匹配

exit_reason 是精确字符串匹配。尾随空格、大小写都影响匹配结果。`"approved"` ≠ `"approved "`。
