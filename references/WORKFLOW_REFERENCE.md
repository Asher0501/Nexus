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
    - [4.4 stderr 处理](#44-stderr-处理)
    - [4.5 行协议前缀](#45-行协议前缀)
    - [4.6 退出码](#46-退出码)
    - [4.7 exit_reason 设置](#47-exit_reason-设置)
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
9. [CLI 参考与调试](#9-cli-参考与调试)
   - [9.1 CLI 用法](#91-cli-用法)
   - [9.2 验证错误速查](#92-验证错误速查)
   - [9.3 通用调试步骤](#93-通用调试步骤)

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
- 同时做反向检查：所有节点必须能通过边链到达某个出口（无出边的节点），否则报 `ExitNotReachable`。
- 节点按在 `nodes[]` 中的定义顺序编号，此顺序影响图的拓扑结构。

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
| `max_retries` | `u64 \| null` | ❌ | `null` | 最大重试次数。`null` = 继承引擎全局默认值 3。注意：重试仅对 Timeout 和 SpawnError 生效，exit-code 失败（非 0 退出）不会自动重试 |

**约束：**
- `id` 必须在整个 `WorkflowDef.nodes[]` 中唯一。重复则报 `DuplicateNodeId`。
- `providers` 为空数组时，Validator 报 `NoValidProvider`。
- `providers` 数组支持多个元素，但引擎**仅使用第一个 provider**执行节点。其余元素预留。
- `returns` 非空时，节点可以通过 stdout 中写入 `__nexus_exit_reason:` 设置返回值（可在任意位置），引擎据此做分支路由。
- `process_timeout_secs` 必须是正数。传入 0 会导致节点立即超时，当前无编译期检查。

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
- 引擎当前支持两种 Provider 类型：`subprocess`（已实现）和 `http`（预留，执行时报错 "not implemented"）

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
- 同一条 `(from, to)` 组合可以出现多次（不同 event/exit_reason），Builder 为每个 `SchedulingEdgeDef` 生成一条独立 `EdgeDef`
- `from` 和 `to` 引用的节点 ID 必须存在于 `nodes[]` 中，否则报 `InvalidPredecessor`
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
- **如果 dataflows 声明了 A→B，但调度 edges 中没有 A→B 的依赖，B 可能在 A 完成之前就开始执行，此时 B 收到的 inputs 中 A 的值为空字符串。** dataflows 不能替代 edges 的调度约束——两者独立，需要分别声明

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

> ⚠️ **PredecessorDef 已废弃。** `NodeDef` 结构体中已不存在 `predecessors` 和 `inputs` 字段。所有调度关系必须在顶层 `edges[]` 中声明，数据流在 `dataflows[]` 中声明。旧语法可被 serde 解析（未知字段静默忽略），但 predecessor 信息会完全丢失，导致所有节点被视为入口节点并行执行。

---

## 2. 调度语义

### 2.1 边触发算法

```
for each out_edge of completed_node:
  1. 如果边已被触发（triggered=true），跳过
  2. 如果事件类型不匹配，跳过
  3. 如果 exit_reason 配置了且不匹配，跳过
  4. event_count++                              ← 每条边有独立计数器
  5. 如果 event_count >= threshold 且 !triggered：
     → triggered = true
     → 目标节点入队（NodeReady）
```

fan-in 场景（A 和 B 完成后触发 C）通过两条独立边实现，每条边各自的计数器独立触发。

### 2.2 All vs Any

当前架构中每条 `EdgeDef` 只有一个 `from` 节点（单源边）。Fan-in 场景通过多条独立边实现。在此架构下 All 和 Any 的语义：

| 策略 | 行为 | 适用场景 |
|------|------|---------|
| **All** | 要求触发事件的节点就是边的 `from` 节点（单源场景下总是满足）。边的 event_count 只累加来自该 `from` 节点的匹配事件 | 单源边 + threshold>1（如自环节点重复执行 N 次后才触发下游） |
| **Any** | 任何匹配事件直接计入 event_count，不检查事件来源 | 与 All 在单源边场景下行为相同；为多源预留 |

**单源边下的实际含义：**

在当前的单源边架构中，fan-in（多上游合并到一个下游）直接表达为多条独立边：
```json
{ "from": "A", "to": "C", "trigger": "all", "event": "complete" }
{ "from": "B", "to": "C", "trigger": "all", "event": "complete" }
```
C 在 A 和 B 各自的边都被触发后才开始执行。每一条边独立计数，All/Any 在当前单源架构下的区别等用于 threshold 累加器的来源过滤。

**组合示例（threshold 自环）：**
```
A ──Complete/All/threshold=3──→ A  (自环 3 次后)
  ──Complete/All/threshold=3──→ B  (触发下游)
```

### 2.3 事件类型

| 事件类型 | 触发条件 | 说明 |
|---------|---------|------|
| `complete` | exit_code = 0 | 正常完成 |
| `failed` | exit_code ≠ 0 | 节点自己报告异常 |
| `timeout` | 超时强杀 | 引擎在 `process_timeout_secs` 后调用 `child.kill()` 终止进程（Unix: SIGKILL，Windows: TerminateProcess） |

三种事件完全对称——引擎的机械计数和触发出边逻辑对三者一视同仁。

### 2.4 阈值（threshold）

- 默认值：1
- 表示"事件发生 N 次后才触发下游"
- `threshold > 1` 常用于自环节点（节点完成后触发自己再次执行）
- 阈值保证了环的终止：环内至少有一个节点的边配置了 `threshold > 1`
- 每条 EdgeDef 都有自己的独立计数器（event_count），互不影响

### 2.5 分支路由（exit_reason）

节点通过 `returns` 声明可能的返回值。运行后在 stdout 中写入 `__nexus_exit_reason: <value>` 设置实际值（可在任意位置，引擎逐行扫描）：

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

重试仅适用于以下场景：
- **Timeout**：节点执行超时，引擎 kill 进程后尝试重试
- **SpawnError**：子进程启动失败（如命令不存在）

**exit-code 失败（exit_code ≠ 0）不触发自动重试**，直接走 Failed 出边。

```
节点 Timeout / SpawnError
  → retry_count = retry_counts[node]
  → if retry_count < max_retries (默认 3):
      retry_counts[node]++
      clear_output(node)                ← 清除上次的脏输出
      send(NodeReady(node))             ← 重新执行，不触发下游边
  → else:
       正常触发 Timeout/Failed 出边
```

| 级别 | 配置项 | 默认值 |
|------|--------|--------|
| 引擎全局 | `EngineConfig.max_timeout_retries` | 3 |
| 节点级 | `NodeDef.max_retries` | 继承全局（字段预留，当前仅使用全局值） |

### 2.7 并发控制

引擎使用 `tokio::sync::Semaphore` 限制同时执行的节点数：

| 配置来源 | 字段 | 默认值 |
|---------|------|--------|
| 引擎全局 | `EngineConfig.max_concurrency` | CPU 核数 |
| 节点级 | `NodeDef.max_concurrency` | 继承全局（预留） |

许可用尽时，`acquire().await` 挂起当前协程直到有节点完成释放槽位。并发槽位是所有节点共享的全局资源，不是按节点分配。

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

引擎将以下 JSON 写入子进程的标准输入，然后**关闭 stdin pipe**（子进程读 stdin 会收到 EOF）：

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

**重要：** 引擎写入 stdin 后立即关闭 pipe。子进程必须能处理：先读完 stdin（直到 EOF），再计算，再写 stdout。典型模式：

```
1. 子进程启动
2. 读 stdin 直到 EOF（获得完整 JSON）
3. 解析 JSON → 获取 inputs
4. 执行业务逻辑
5. 写 stdout
6. exit 0
```

如果子进程需要交互式输入或长时间保持 stdin 打开，则不适用本协议。

### 4.3 stdout 输出格式

向 stdout 写入纯文本结果。引擎**逐行实时读取**。

### 4.4 stderr 处理

子进程的 stderr 被引擎独立读取，通过 `tracing::warn!(target: "nexus::node::stderr")` 上报。

**关键点：**
- stderr 内容**不会进入节点 output**（不参与 DataRouter 数据传递）
- stderr 不会触发 `__nexus_` 行协议解析
- stderr 行在子进程退出后的 drain 阶段也会被捕获并上报
- stderr 不会出现在 NodeChunk 通道中（仅 stdout 实时流式传输）

### 4.5 行协议前缀

| 前缀 | 含义 | 是否进入业务输出 |
|------|------|----------------|
| `__nexus_log: TEXT` | 中间日志（tracing::info!） | ❌ |
| `__nexus_event: TEXT` | 结构化输出片段 | ✅ — 追加到 output_buf |
| `__nexus_exit_reason: VALUE` | 设置分支路由返回值 | ❌ — 仅设置 exit_reason |
| `__nexus_log_end` | 结束特殊前缀处理模式：之后所有行（无论有无前缀）均被视为普通输出追加到 output_buf | — |
| 无前缀 | 普通输出行 | ✅ — 追加到 output_buf |

**实时性：** 每行 stdout 在到达时立即通过 `nexus::node::chunk` 事件上报，不需要等进程退出。

### 4.6 退出码

| 退出码 | 含义 | 引擎行为 |
|--------|------|---------|
| 0 | 正常完成 | 采用 stdout 内容作为节点输出 |
| 非0 | 节点自己报告异常 | 走失败处理路径（触发 Failed 出边） |
| 被信号杀死 | 异常终止 | 走失败处理路径 |

### 4.7 exit_reason 设置

两种方式：
1. **stdout 行协议：** 在 stdout 中写入 `__nexus_exit_reason: <value>`（运行时实时设置，推荐）
2. **节点返回值逻辑：** 通过 `returns` 声明 + 退出码（引擎侧处理）

---

## 5. 流式输出

### 5.1 NodeChunk 机制

引擎为每个执行中的节点创建独立的 chunk channel（`mpsc::channel::<NodeChunk>(256)`，有界队列）。

每行 stdout 到达时：
1. 通过 `try_send` 发送到 chunk channel（channel 满时静默丢弃该 chunk，不影响 output 完整性）
2. 后台消费任务通过 `tracing::info!(target: "nexus::node::chunk")` 上报
3. 同时追加到 `NodeOutcome.output` 缓冲区
4. 进程退出后，完整 output 进入 DataRouter

**注意：** chunk channel 是有界队列（容量 256），用于实时进度展示。`NodeOutcome.output` 始终包含完整输出，不受 channel 溢出影响。实时 chunk 丢失不影响节点最终输出。

### 5.2 实时事件上报

`--verbose` 模式下，`nexus::node::chunk` 事件在终端可见：

```
2026-07-06T12:27:05Z INFO nexus::node::chunk: node_id="review" text="step_start"
2026-07-06T12:27:05Z INFO nexus::node::chunk: node_id="review" text="text"
2026-07-06T12:27:05Z INFO nexus::node::chunk: node_id="review" text="step_finish"
```

### 5.3 模板插值

命令字符串中的 `{{inputs.node_id}}` 在执行前被 SubprocessExecutor 替换为上游节点的输出。数据来源由顶层 `dataflows[]` 声明决定。

```json
{
  "nodes": [
    { "id": "config", "providers": [...], "process_timeout_secs": 10 },
    { "id": "review", "providers": [{
      "type": "subprocess",
      "command": "opencode run --format json -- \"{{inputs.config}}\""
    }], "process_timeout_secs": 300 }
  ],
  "edges": [
    { "from": "config", "to": "review", "trigger": "all", "event": "complete" }
  ],
  "dataflows": [
    { "from": "config", "to": "review" }
  ]
}
```

**规则：**
- 模板语法：`{{inputs.节点ID}}`，其中节点ID 必须对应 `dataflows` 中某个 `from` 的值
- 替换发生在 spawn 之前
- 不存在的 key 被替换为空字符串
- 普通文本不受影响

---

## 6. 模式模板

> ⚠️ 所有模板均使用当前推荐的顶层 `edges` / `dataflows` 语法。旧版内联 `predecessors` / `inputs` 字段已废弃，不可在 `NodeDef` 中使用。

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
      "process_timeout_secs": 10
    },
    {
      "id": "branch_a",
      "providers": [{"type": "subprocess", "command": "python a.py"}],
      "process_timeout_secs": 30
    },
    {
      "id": "branch_b",
      "providers": [{"type": "subprocess", "command": "python b.py"}],
      "process_timeout_secs": 30
    },
    {
      "id": "merge",
      "providers": [{"type": "subprocess", "command": "python merge.py"}],
      "process_timeout_secs": 30
    }
  ],
  "edges": [
    { "from": "source", "to": "branch_a", "trigger": "all", "event": "complete" },
    { "from": "source", "to": "branch_b", "trigger": "all", "event": "complete" },
    { "from": "branch_a", "to": "merge", "trigger": "all", "event": "complete" },
    { "from": "branch_b", "to": "merge", "trigger": "all", "event": "complete" }
  ],
  "dataflows": [
    { "from": "source", "to": "branch_a" },
    { "from": "source", "to": "branch_b" },
    { "from": "branch_a", "to": "merge" },
    { "from": "branch_b", "to": "merge" }
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
      "returns": ["approved", "rejected"]
    },
    {
      "id": "deploy",
      "providers": [{"type": "subprocess", "command": "python deploy.py"}],
      "process_timeout_secs": 120
    },
    {
      "id": "fix",
      "providers": [{"type": "subprocess", "command": "python fix.py"}],
      "process_timeout_secs": 60
    }
  ],
  "edges": [
    { "from": "source", "to": "review", "trigger": "all", "event": "complete" },
    { "from": "review", "to": "deploy", "trigger": "all", "event": "complete", "exit_reason": "approved" },
    { "from": "review", "to": "fix", "trigger": "all", "event": "complete", "exit_reason": "rejected" }
  ],
  "dataflows": [
    { "from": "source", "to": "review" }
  ]
}
```

### 6.4 带阈值的自环（Threshold Loop）

```
collector ──Complete/threshold=3──→ collector  (自环 3 次后)
          ──Complete/threshold=3──→ aggregator
```

```json
{
  "nodes": [
    {
      "id": "start",
      "providers": [{"type": "subprocess", "command": "echo begin"}],
      "process_timeout_secs": 10
    },
    {
      "id": "collector",
      "providers": [{"type": "subprocess", "command": "python collector.py"}],
      "process_timeout_secs": 30
    },
    {
      "id": "aggregator",
      "providers": [{"type": "subprocess", "command": "python aggregator.py"}],
      "process_timeout_secs": 30
    }
  ],
  "edges": [
    { "from": "start", "to": "collector", "trigger": "all", "event": "complete" },
    { "from": "collector", "to": "collector", "trigger": "all", "event": "complete", "threshold": 3 },
    { "from": "collector", "to": "aggregator", "trigger": "all", "event": "complete", "threshold": 3 }
  ],
  "dataflows": [
    { "from": "collector", "to": "aggregator" }
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
      "process_timeout_secs": 30
    }
  ],
  "edges": []
}
```

### 6.6 并行聚合（Parallel Aggregation）

```
  A ──→┐
  B ──→ M
  C ──→┘
```

```json
{
  "nodes": [
    { "id": "a", "providers": [{"type":"subprocess","command":"a.py"}], "process_timeout_secs": 10 },
    { "id": "b", "providers": [{"type":"subprocess","command":"b.py"}], "process_timeout_secs": 10 },
    { "id": "c", "providers": [{"type":"subprocess","command":"c.py"}], "process_timeout_secs": 10 },
    { "id": "merge", "providers": [{"type":"subprocess","command":"merge.py"}], "process_timeout_secs": 10 }
  ],
  "edges": [
    { "from": "a", "to": "merge", "trigger": "all", "event": "complete" },
    { "from": "b", "to": "merge", "trigger": "all", "event": "complete" },
    { "from": "c", "to": "merge", "trigger": "all", "event": "complete" }
  ],
  "dataflows": [
    { "from": "a", "to": "merge" },
    { "from": "b", "to": "merge" },
    { "from": "c", "to": "merge" }
  ]
}
```

### 6.7 错误处理（Error Handling）

```
  A ──Complete──→ B
  A ──Failed  ──→ error_handler
```

```json
{
  "nodes": [
    {
      "id": "risky_task",
      "providers": [{"type": "subprocess", "command": "python risky.py"}],
      "process_timeout_secs": 30
    },
    {
      "id": "on_success",
      "providers": [{"type": "subprocess", "command": "python success.py"}],
      "process_timeout_secs": 10
    },
    {
      "id": "on_failure",
      "providers": [{"type": "subprocess", "command": "python notify_failure.py"}],
      "process_timeout_secs": 10
    }
  ],
  "edges": [
    { "from": "risky_task", "to": "on_success", "trigger": "all", "event": "complete" },
    { "from": "risky_task", "to": "on_failure", "trigger": "all", "event": "failed" }
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
    "process_timeout_secs": 300
  }],
  "edges": []
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
    "process_timeout_secs": 600
  }],
  "edges": []
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
      "process_timeout_secs": 10
    },
    {
      "id": "llm_review",
      "providers": [{
        "type": "subprocess",
        "command": "opencode run --format json --dangerously-skip-permissions --model claude-sonnet-4 -- \"{{inputs.config}}\""
      }],
      "process_timeout_secs": 300
    },
    {
      "id": "report",
      "providers": [{"type": "subprocess", "command": "python save_report.py --input \"{{inputs.llm_review}}\""}],
      "process_timeout_secs": 10
    }
  ],
  "edges": [
    { "from": "config", "to": "llm_review", "trigger": "all", "event": "complete" },
    { "from": "llm_review", "to": "report", "trigger": "all", "event": "complete" }
  ],
  "dataflows": [
    { "from": "config", "to": "llm_review" },
    { "from": "llm_review", "to": "report" }
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
      "process_timeout_secs": 10
    },
    {
      "id": "lead_review",
      "providers": [{"type": "subprocess", "command": "python lead_approve.py"}],
      "process_timeout_secs": 3600,
      "returns": ["approved", "rejected"]
    },
    {
      "id": "manager_review",
      "providers": [{"type": "subprocess", "command": "python manager_approve.py"}],
      "process_timeout_secs": 7200,
      "returns": ["approved", "rejected"]
    },
    {
      "id": "rejected",
      "providers": [{"type": "subprocess", "command": "python notify_rejected.py"}],
      "process_timeout_secs": 10
    },
    {
      "id": "execute",
      "providers": [{"type": "subprocess", "command": "python execute.py"}],
      "process_timeout_secs": 300
    }
  ],
  "edges": [
    { "from": "request", "to": "lead_review", "trigger": "all", "event": "complete" },
    { "from": "lead_review", "to": "manager_review", "trigger": "all", "event": "complete", "exit_reason": "approved" },
    { "from": "lead_review", "to": "rejected", "trigger": "all", "event": "complete", "exit_reason": "rejected" },
    { "from": "manager_review", "to": "execute", "trigger": "all", "event": "complete", "exit_reason": "approved" }
  ],
  "dataflows": [
    { "from": "request", "to": "lead_review" },
    { "from": "lead_review", "to": "manager_review" },
    { "from": "manager_review", "to": "execute" }
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

**重要：自环节点第一次执行需要从入口可达。** 仅有一条自环边但没有任何入边的节点永远不会被触发——因为没有 entry 节点或上游节点能启动它。

**自环的典型模式：**
```
entry_node ──Complete/threshold=1──→ collector   ← entry 触发第一次
collector  ──Complete/threshold=3──→ collector   ← 自环负责重复
collector  ──Complete/threshold=3──→ aggregator  ← 3 次后触发下游
```
collector 由 entry_node 触发首次执行，之后自环边负责后续 2 次重复（threshold=3 需要 3 次事件），第 3 次完成后触发 aggregator。

### 8.4 输出大小限制

- 单节点 stdout 输出不应超过 100MB
- 超出可能导致 OOM 或 pipe buffer 满导致的死锁
- 大量输出建议用文件传递模式

### 8.5 命令拆分的局限性

`SubprocessExecutor` 使用 `split_whitespace()` 拆分 command（非 shell 解释器）。这意味着：
- 引擎直接 spawn 进程，**不经过 shell**——无 shell 注入风险
- 含引号的参数（`--name "John Doe"`）不支持，因为 `split_whitespace` 将引号视为普通字符
- shell 管道和重定向（`|`、`>`）不生效
- 环境变量设置（`VAR=val command`）不生效
- 以上场景均需用包装脚本：`cmd /c`, `powershell -File`, 或 `sh -c`

### 8.6 exit_reason 字符串匹配

exit_reason 是精确字符串匹配。尾随空格、大小写都影响匹配结果。`"approved"` ≠ `"approved "`。

---

## 9. CLI 参考与调试

### 9.1 CLI 用法

```bash
nexus-cli run <workflow.json>

  --max-concurrency N    最大并发节点数（默认：CPU 核数）
  --node-timeout S       节点默认超时秒数（默认：3600），被节点级
                         process_timeout_secs 覆盖
  --max-timeout-retries N
                         超时和 spawn 失败的重试次数（默认：3）
  --verbose              详细日志（含流式 chunk 输出）
  --validate-only        仅验证，不执行
  --dump-state           完成后输出节点状态快照

退出码：
  0  Success
  1  Validation error
  2  Runtime error
  3  Idle timeout（预留）
```

### 9.2 验证错误速查

`nexus-cli run --validate-only` 在 JSON 无效时打印以下错误：

| 错误信息 | 原因 |
|---------|------|
| `empty graph: no nodes defined` | nodes 数组为空 |
| `duplicate node ID: 'X'` | 两个节点 ID 相同 |
| `no entry node: all nodes have predecessors` | 所有节点都有入边，没有入口节点 |
| `unreachable node 'X': not reachable from any entry node` | 节点在调度图中不可达 |
| `cycle without entry: deadlock detected` | 存在无入口的环（可能自环 threshold=1，或多个节点形成环路且环内无入口节点） |
| `exit not reachable from node 'X'` | 节点 X 没有路径到达任何出口节点（无出边的节点）。检查 X 的出边是否指向了可达出口的路径 |
| `node 'X' has no valid provider` | providers 数组为空 |
| `node 'X' references non-existent predecessor 'Y'` | edges 中 from/to 引用了不存在的节点 ID |
| `input source 'Y' for node 'X' is not reachable from any entry` | dataflows 中 from 节点不可达 |
| `build invariant failure: ...` | 内部图构造不变量检查失败。通常由不完整的节点/边配置引起：所有节点必须有 params 和 transfer 函数、所有边必须被某个 transfer 引用。排查节点 ID 引用和 `dataflows` 声明是否完整 |

### 9.3 通用调试步骤

1. **验证 JSON 结构**：`nexus-cli run workflow.json --validate-only`，修复所有错误
2. **单节点测试**：先用一个 echo 节点验证基础流程 `nexus-cli run --validate-only`
3. **查看流式输出**：`nexus-cli run workflow.json --verbose` 查看每行 stdout 实时输出
4. **查看最终状态**：`nexus-cli run workflow.json --dump-state` 查看所有节点终态
5. **检查日志**：运行日志写入 `log/run-{timestamp}.log`
