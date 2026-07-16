# 工作流定义参考

> Nexus 工作流定义的完整权威规范。包含全部 schema、机制说明、模式模板和边界情况。

**分类**：manual

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
   - [3.3 Snapshot Semantics](#33-snapshot-semantics)
4. [节点协议（NODE_PROTOCOL）](#4-节点协议node_protocol)
   - [4.1 通信流程](#41-通信流程)
   - [4.2 stdin 输入格式](#42-stdin-输入格式)
   - [4.3 stdout 输出格式](#43-stdout-输出格式)
   - [4.4 stderr 处理](#44-stderr-处理)
   - [4.5 引擎解析行为](#45-引擎解析行为)
   - [4.6 退出码](#46-退出码)
   - [4.7 exit_reason 设置](#47-exit_reason-设置)
5. [流式输出](#5-流式输出)
   - [5.1 JSON 流式输出](#51-json-流式输出)
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
7. [LLM Agent 节点](#7-llm-agent-节点type-llm--type-llm_sdk)
   - [7.1 单节点 Claude 调用（CLI）](#71-单节点-claude-调用cli)
   - [7.2 单节点 OpenCode 调用（CLI）](#72-单节点-opencode-调用cli)
   - [7.3 代码审查循环（CLI）](#73-代码审查review-fix-retro-循环cli)
   - [7.4 任意新 CLI](#74-任意新-clinga-codeagent-等)
   - [7.5 逐级上报审批流程（CLI）](#75-逐级上报审批流程cli)
   - [7.6 单节点 SDK 直调](#76-单节点-llm_sdk-直调)
   - [7.7 审查循环（SDK）](#77-审查循环-llm_sdk)
8. [边界情况与限制](#8-边界情况与限制)
9. [CLI 参考与调试](#9-cli-参考与调试)
   - [9.1 CLI 用法](#91-cli-用法)
   - [9.2 验证错误速查](#92-验证错误速查)
   - [9.3 通用调试步骤](#93-通用调试步骤)
   - [9.4 Dashboard 集成](#94-dashboard-集成)

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
| --- | --- | --- | --- | --- |
| `nodes` | `NodeDef[]` | ✅ | — | 工作流中的所有节点 |
| `edges` | `SchedulingEdgeDef[]` | ❌ | `[]` | 调度拓扑边（谁完成后谁可以开始） |
| `dataflows` | `DataFlowDef[]` | ❌ | `[]` | 数据拓扑边（谁的数据传给谁） |
| `scripts_dir` | `string \| null` | ❌ | `null` | 全局脚本目录。所有节点的默认 `scripts_dir`。节点级配置覆盖此值 |

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
  "process_timeout_secs": 30
}
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `id` | `string` | ✅ | — | 唯一标识符，在整个工作流中不可重复 |
| `providers` | `ProviderDef[]` | ✅ | — | 执行方式数组，至少一个有效 provider |
| `process_timeout_secs` | `u64` | ✅ | — | 节点超时秒数。超时后引擎强杀进程，按 Timeout 事件处理 |
| `route_policy` | `object \| null` | ❌ | `null` | 路由策略。配置后 NodeShell 根据系统状态（如 `run_count`）计算 `DataRouter.route`，覆盖节点自身的 route。详见 §2.5 |
| `returns` | `string[]` | ❌ | `[]` | 节点可能输出的 route 值列表。供工具链和 IDE 使用，不参与运行时路由决策（运行时路由由 `route` + `exit_reason` 决定） |
| `max_retries` | `u64 \| null` | ❌ | `null` | 节点级超时/SpawnError 重试上限。`null` 时继承引擎全局默认值（3） |
| `scripts_dir` | `string \| null` | ❌ | `null` | 节点级脚本目录。覆盖 `WorkflowDef.scripts_dir`。用于定位 `llm_node.py`、`llm_sdk.py` 及自定义脚本。在模板中通过 `{{node_dir}}` 引用 |

**约束：**
- `id` 必须在整个 `WorkflowDef.nodes[]` 中唯一。重复则报 `DuplicateNodeId`。
- `providers` 为空数组时，Validator 报 `NoValidProvider`。
- `providers` 数组支持多个元素，但引擎**仅使用第一个 provider**执行节点。其余元素预留。
- `process_timeout_secs` 必须是正数。传入 0 会在验证时被拒绝（`ZeroTimeout`）。
- `scripts_dir` 解析优先级：节点级 > 工作流级 > `NEXUS_SCRIPTS_DIR` 环境变量 > 引擎二进制向上搜索 `scripts/` > `./scripts`

---

### 1.3 ProviderDef — 执行方式

支持三种执行方式：

```json
// 直接执行（不使用 shell）
{
  "type": "subprocess",
  "command": "python fetcher.py --url {{datarouter.url.content}}"
}

// 通过 shell 执行（支持管道、重定向、引号）
{
  "type": "shell",
  "command": "opencode run --format json --auto -- \"代码审查\" | python wrap.py"
}

// LLM Agent 节点（通用，支持 claude、opencode 及任意 CLI）
{
  "type": "llm",
  "command": "claude -p \"{{prompt}}\" --output-format json --verbose",
  "prompt": "审查代码: {{datarouter.seed.content}}。回复 JSON: {\"route\":\"approved|rejected\",\"content\":\"...\"}",
  "routes": ["approved", "rejected"]
}

// LLM SDK 节点（Anthropic SDK 直调，无需 CLI）
{
  "type": "llm_sdk",
  "model": "claude-sonnet-5-20251001",
  "prompt": "审查代码。输出 ONLY: {\"route\":\"ok|err\",\"content\":\"summary\"}",
  "routes": ["ok", "err"],
  "system_prompt": "You are a code reviewer.",
  "max_tokens": 4096
}
```

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `type` | `"subprocess"` \| `"shell"` \| `"llm"` \| `"llm_sdk"` | ✅ | 执行方式。`subprocess` 直接 spawn；`shell` 通过 shell 包装；`llm` 通用 LLM agent 节点；`llm_sdk` 通过 Anthropic Python SDK 直调。（`"http"` 为预留值，当前未实现，运行时报错） |
| `command` | `string` | ✅（`subprocess`/`shell`/`llm`） | 要执行的命令。支持模板插值 |
| `prompt` | `string` | ❌（`llm`/`llm_sdk`） | Prompt 模板。省略则自动拼接所有 inputs |
| `routes` | `string[]` | ❌（`llm`/`llm_sdk`） | 期望的 route 值列表 |
| `model` | `string` | ✅（`llm_sdk`） | Anthropic 模型 ID，如 `claude-sonnet-5-20251001` |
| `api_key_env` | `string \| null` | ❌（`llm_sdk`） | API key 环境变量名，默认 `ANTHROPIC_API_KEY` |
| `system_prompt` | `string \| null` | ❌（`llm_sdk`） | 系统级指令 |
| `max_tokens` | `u64 \| null` | ❌（`llm_sdk`） | 最大输出 token 数（SDK 模式）。CLI 模式通过 command 的 `--max-tokens` flag 控制 |

**关于 command 的规则：**
- `type: "subprocess"`：command 按空格拆分为 `[program, arg1, arg2, ...]`，不支持管道 `|`、重定向 `>`、引号等 shell 特性
- `type: "shell"`：command 被 shell 包装后再执行，管道、重定向、引号都自然工作
- `type: "http"`：当前未实现，运行时报错
- `type: "llm"`：通用 LLM agent 节点，引擎通过内置 Python wrapper 执行，自动处理跨平台 CLI 发现（Windows .exe/.cmd、Linux 原生二进制）、实时流式输出写入 log、结果解析与路由。`{{prompt}}` 在渲染后被替换为完整的 prompt 文本。任何 CLI 工具（claude、opencode、nga、codeagent 等）均可直接使用
- `{{datarouter.X.content}}` 在执行时替换为上游节点 X 的输出内容
- `{{datarouter.X.route}}` 替换为上游节点 X 的 route 值
- `{{metadata.run_count}}` 替换为当前节点的执行轮次（1-based）
- `{{metadata.timed_out}}` 替换为 `true` / `false`（上次执行是否超时）
- `{{node_dir}}` 替换为当前节点的 scripts 目录绝对路径
- `{{prompt}}` 替换为 ProviderDef 的 `prompt` 字段渲染值（仅用于 `type: "llm"` 的 command）



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
| --- | --- | --- | --- | --- |
| `from` | `string` | ✅ | — | 上游节点 ID（数据/事件来源） |
| `to` | `string` | ✅ | — | 下游节点 ID（目标节点） |
| `trigger` | `"all" \| "any"` | ✅ | — | 组合逻辑。All = 所有指向同一下游的 All 策略边都触发后才入队下游；Any = 立即入队下游 |
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
| --- | --- | --- | --- | --- |
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
| --- | --- | --- | --- | --- |
| `node_id` | `string` | ✅ | — | 上游节点 ID |
| `trigger` | `"all" \| "any"` | ✅ | — | 组合逻辑 |
| `event` | `"complete" \| "failed" \| "timeout"` | ✅ | — | 匹配的事件类型 |
| `exit_reason` | `string \| null` | ❌ | `null` | 匹配的 exit_reason 值 |
| `threshold` | `u64` | ❌ | `1` | 触发所需的事件次数 |

> ⚠️ **PredecessorDef 已废弃。** `NodeDef` 结构体中已不存在 `predecessors` 和 `inputs` 字段。所有调度关系必须在顶层 `edges[]` 中声明，数据流在 `dataflows[]` 中声明。旧语法可被 serde 解析（未知字段静默忽略），但 predecessor 信息会完全丢失，导致所有节点被视为入口节点并行执行。

---

## 2. 调度语义

### 2.1 边触发算法

引擎采用 **h_e + g_e 正交分解**（详见 `theory/NODE_TRANSFER.md`）。每条边由两个正交函数构成：
- **h_e（分支匹配）**：纯函数，不记忆、无 triggered 状态。事件类型匹配 + exit_reason 过滤器 + 阈值计数器。
- **g_e（策略聚合）**：Any → 立即入队；All → fan_in_pending 归零后入队，重置供下一轮使用。

```
f_v(v, event, exit_reason) =
    { w | e = (v, w, h_e, g_e) ∈ E
          ∧ h_e(event, exit_reason)
          ∧ g_e({ u.ready | u ∈ pred(w) }) }
```

实际执行时，引擎对已完成节点 v 调用其局部转移函数 f_v（由 NodeTransfer::evaluate() 实现）：

```
1. 更新节点 v 的状态和事件计数器
2. 对 v 的每条出边 e = (v, w, h_e, g_e)：
   (a) 如果事件类型不匹配，跳过
   (b) 如果 exit_reason 配置了且不匹配，跳过
   (c) event_count++                         ← h_e 阈值计数器
   (d) 如果 event_count < threshold，跳过    ← 阈值未到，等待下一次事件
   (e) g_e 策略聚合：
       Any → 立即入队 w
       All → fan_in_pending[w] -= 1
             如果 fan_in_pending[w] == 0：入队 w，重置 fan_in_pending[w]
             （否则等待其他入边触发）
```

**关键设计要点：**
- **没有 triggered 状态。** h_e 是无状态的纯函数——每条边不记忆"是否已经触发过"。每条新事件独立评估所有匹配边。环路因而不需要特殊处理就能多轮执行。
- **threshold 是唯一的"重复防护"机制。** 如果希望一条边只触发一次，设 threshold=1 且确保源节点只产生一次匹配事件。对于环路场景，设 threshold=N 让环在 N 轮后停止。
- **All 策略的 fan_in_pending 每轮重置。** 下游节点入队后，fan_in_pending 恢复初始值，下一轮重新计数。
- **h_e 和 g_e 完全正交。** 分支匹配不依赖聚合策略，聚合策略不关心匹配细节。

fan-in 场景（A 和 B 完成后触发 C）通过两条独立边实现。

### 2.2 All vs Any

| 策略 | 行为 | 适用场景 |
| --- | --- | --- |
| **Any** | 匹配的边触发后立即入队下游节点 | 默认策略；单源边、simple chain |
| **All** | 边触发后不立即入队，fan_in_pending 减 1，只有所有指向同一 `to` 的 All 策略边都触发后才入队下游 | 扇入聚合（多个上游完成后才触发下游） |

**示例：**

```json
// Any：A 一完成就触发 B
{ "from": "A", "to": "B", "trigger": "any", "event": "complete" }

// All：A 和 B 都完成后才触发 C  
{ "from": "A", "to": "C", "trigger": "all", "event": "complete" },
{ "from": "B", "to": "C", "trigger": "all", "event": "complete" }
```

**注意：** All 语义下如果某个上游的事件类型与边不匹配（例如边声明 event=complete 但上游 exit=1 产生的是 failed），该边永不会触发，下游也永不会入队。这是一个未定义行为——引擎不会主动 deadlock 检测，工作流会挂起。

### 2.3 事件类型

| 事件类型 | 触发条件 | 说明 |
| --- | --- | --- |
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

边通过 `exit_reason` 字段做精确字符串匹配，决定节点的哪个下游被触发。
exit_reason 的匹配依据是 **route** 的最终值。

#### route 的來源

route 有两个写入者，按优先级：

| 优先级 | 写入者 | 写入条件 | 说明 |
|--------|--------|----------|------|
| 低 | 节点 stdout 的 `route` 字段 | 节点输出中包含 `route` 时 | 节点自己标注路由标签 |
| 高 | NodeShell 策略 `route_policy` | 节点配置了 `route_policy` 且策略条件满足时 | NodeShell 根据系统状态计算路由 |

**写入规则：**
- 节点 stdout 的 `route` **始终生效**，不受任何开关控制
- NodeShell 仅在 **配置了 `route_policy` 且条件满足时** 写入，覆盖节点的值
- 如果两者都没有提供值，route 为空字符串

#### 路由匹配

```
边匹配条件：
  1. event 匹配（由 exit_code 决定：0→complete，非0→failed，超时→timeout）
  2. 如果边声明了 exit_reason → 精确匹配 route 值
     如果 route 为空且边 exit_reason 非空 → 不匹配（fallthrough）
     如果边 exit_reason 为 null → 不检查 route，仅凭 event 触发
```

**基础路由（默认 fallback）：**
- 不配置 `route_policy` 时，route 来自节点 stdout 的 `route`（如果存在）
- `route` 为空时视为无 exit_reason，仅匹配 `exit_reason: null` 的边
- 如果没有任何边匹配，该节点不触发任何下游

**路由策略（route_policy）：**
- 节点配置了 `route_policy` 后，NodeShell 根据系统状态（`run_count`、`timed_out` 等）计算 route
- 节点 stdout 的 `route` 被覆盖，不影响路由
- 示例见 §6.4 有向环

**分支未匹配时的行为：**
- 如果没有任何边匹配 exit_reason，则该节点不会触发任何下游
- 未触发的下游节点保持在 Pending 状态
- 引擎看门狗检测到连续 3 次超后（默认 30s），将所有 Pending 节点标记为 Skipped，工作流收敛

#### RoutePolicy 配置

```json
{
  "route_policy": {
    "type": "max_runs",
    "max": 3,
    "then_route": "approved"
  }
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `type` | `string` | ✅ | 策略类型。当前仅支持 `"max_runs"` |
| `max` | `u64` | ✅ | 运行次数上限。达到此值后策略生效 |
| `then_route` | `string` | ✅ | 策略生效时 NodeShell 写入的 route 值 |

**语义：**
- `run_count < max` → 策略不生效，route 由节点 stdout 决定
- `run_count >= max` → 策略生效，NodeShell 将 route 覆盖为 `then_route`

### 2.6 重试机制

重试仅适用于以下场景：
- **Timeout**：节点执行超时，引擎 kill 进程后尝试重试
- **SpawnError**：子进程启动失败（如命令不存在）

**exit-code 失败（exit_code ≠ 0）不触发自动重试**，直接走 Failed 出边。

```
节点 Timeout / SpawnError
  → retry_count = retry_counts[node]
  → if retry_count < max_timeout_retries (默认 3):
      retry_counts[node]++
      clear_output(node)                ← 清除上次的脏输出
      send(NodeReady(node))             ← 重新执行，不触发下游边
  → else:
       正常触发 Timeout/Failed 出边
```

| 级别 | 配置项 | 默认值 |
| --- | --- | --- |
| 引擎全局 | `EngineConfig.max_timeout_retries` | 3 |
| 节点级 | `NodeDef.max_retries` | 继承全局。设置后覆盖引擎全局 `max_timeout_retries` |

### 2.7 并发控制

引擎使用 `tokio::sync::Semaphore` 限制同时执行的节点数：

| 配置来源 | 字段 | 默认值 |
| --- | --- | --- |
| 引擎全局 | `EngineConfig.max_concurrency` | CPU 核数 |

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
- **同节点覆盖：** 在有向环场景中，同一节点可能多次执行并覆盖自己的输出。下游节点通过 `dataflows` 收到的始终是该节点的**最新一次**输出。当前没有版本号或轮次标记来区分"第 N 次覆盖"。如果需要区分，节点可通过 `metadata.run_count`（见 §4.2）自行判断当前是第几次执行
- 尚未执行的源节点返回空字符串
- 当 dataflows 中声明的源节点尚无输出时，DataRouter 返回空字符串并记录 `[Engine.DataRouter] Task node:{id} no msg.` 日志。

---

## 4. 节点协议（NODE_PROTOCOL）

### 4.1 通信流程

```
引擎 → spawn(你的命令)
引擎 → stdin 写入 NodeContext JSON
引擎 → 关闭 stdin（子进程收到 EOF）
节点 → 读取 stdin JSON → 计算
节点 → stdout 写入 JSON {"route":"...","content":"..."}
节点 → exit 0（完成）／非0（失败）
引擎 → 进程退出后解析 JSON → 提取 route/content → DataRouter → 触发下游
```

### 4.2 stdin 输入格式

引擎将以下 JSON 写入子进程的标准输入，然后**关闭 stdin pipe**（子进程读 stdin 会收到 EOF）：

```json
{
  "inputs": {
    "source_node_id": "上游节点输出的纯文本"
  },
  "extensions": {},
  "metadata": {
    "run_count": 1,
    "timed_out": false
  }
}
```

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `inputs` | `object` | 上游节点的输出。key = 来源节点 ID，value = 该节点输出的纯文本 |
| `extensions` | `object` | 节点类型特有的配置参数（保留字段） |
| `metadata` | `object` | 执行元信息，包含 `run_count`（当前是第几次执行）和 `timed_out`（上次执行是否超时） |

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

节点必须向 stdout 输出一个 JSON 对象，格式为：

```json
{"route":"exit_reason_value","content":"节点输出文本"}
```

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `route` | `string` | ✅ | 路由键，用于边匹配（exit_reason）。空字符串表示无特定路由 |
| `content` | `string` | ❌ | 节点输出内容。缺省时为空字符串 |

**关键规则：**
- 节点必须输出**唯一一个** JSON 对象。多余输出会被忽略或导致解析失败。
- `route` 字段必须存在。缺少 `route` 的 JSON 会被引擎拒绝，触发 `SpawnError`。
- `content` 字段可选。缺省时默认为空字符串。
- 纯文本输出（非 JSON）会被引擎拒绝。
- 引擎在进程退出后一次性解析 stdout，不做逐行实时解析。

### 4.4 stderr 处理

子进程的 stderr 被引擎独立读取，通过 `tracing::warn!(target: "nexus::node::stderr")` 上报。

**关键点：**
- stderr 内容**不会进入节点 output**（不参与 DataRouter 数据传递）
- stderr 内容不会被解析为 JSON（仅 stdout 参与 JSON 协议）
- stderr 行在子进程退出后的 drain 阶段也会被捕获并上报

### 4.5 引擎解析行为

引擎在进程退出后解析 stdout JSON（输出格式见 §4.3）：
- 若 stdout 不是合法 JSON 或缺少 `route` 字段 → 返回 `SpawnError`，节点标记为 failed
- 若 JSON 中 `route` 非空 → 提取为 exit_reason 供边匹配
- 若 JSON 中 `route` 为空 → 无 exit_reason，仅匹配 `exit_reason: null` 的边
- `content` 字段进入 DataRouter，作为该节点的输出供下游使用

### 4.6 退出码

| 退出码 | 含义 | 引擎行为 |
| --- | --- | --- |
| 0 | 正常完成 | 解析 stdout JSON → 提取 route/content 作为节点输出 |
| 非0 | 节点自己报告异常 | 走失败处理路径（触发 Failed 出边） |
| 被信号杀死 | 异常终止 | 走失败处理路径 |

### 4.7 exit_reason 设置

节点通过 stdout JSON 的 `route` 字段设置退出原因。引擎解析 stdout JSON 后，将 `route` 值与边的 `exit_reason` 字段做精确匹配：

- `route` 非空字符串 → 作为 exit_reason 参与边的精确匹配
- `route` 为空字符串 → 无 exit_reason，仅匹配 `exit_reason: null` 的边

**JSON 示例：**
```json
{"route":"approved","content":"审查通过"}
```
引擎提取 `route = "approved"` 作为 exit_reason，与声明 `exit_reason: "approved"` 的边匹配。

---

## 5. 流式输出

### 5.1 JSON 流式输出

引擎在进程退出后一次性解析 stdout JSON，不支持逐行流式输出。节点必须保证最终 stdout 包含一个完整的 JSON 对象。

若节点需要报告中间进度，可按以下方式处理：
- **通过 stderr 输出日志**：引擎独立读取 stderr 并通过 `tracing::warn!(target: "nexus::node::stderr")` 上报
- **通过文件输出**：节点将中间结果写入临时文件，最终 JSON 的 `content` 字段引用文件路径
- **使用 `--verbose` 模式**：引擎在节点执行过程中打印"等待节点完成"的状态日志

### 5.2 模板插值

引擎在执行前渲染 prompt 和 command 中的模板变量。支持以下语法：

| 语法 | 来源 | 说明 |
|------|------|------|
| `{{datarouter.<alias>.content}}` | `DataRouter.outputs[].content` | 上游节点的输出内容。`<alias>` = dataflow 中 `from` 的值或 `alias` |
| `{{datarouter.<alias>.route}}` | `DataRouter.outputs[].route` | 上游节点的 route 值 |
| `{{metadata.run_count}}` | `NodeMetadata.run_count` | 当前节点执行轮次（1-based） |
| `{{metadata.timed_out}}` | `NodeMetadata.timed_out` | 上次执行是否超时（`true` / `false`） |
| `{{node_dir}}` | `scripts_dir`（已解析） | 当前节点的脚本目录绝对路径。用于引用配套工具：`python {{node_dir}}/llm_sdk.py` |
| `{{prompt}}` | ProviderDef 的 `prompt` 字段（已渲染） | LLM 节点的完整 prompt 文本。仅用于 `type: "llm"` 的 `command` 字段 |

```json
{
  "nodes": [
    { "id": "config", "providers": [...], "process_timeout_secs": 10 },
    { "id": "review", "providers": [{
      "type": "shell",
      "command": "echo \"Received: {{datarouter.config.content}}\""
    }], "process_timeout_secs": 30 }
  ],
  "edges": [
    { "from": "config", "to": "review", "trigger": "all", "event": "complete" }
  ],
  "dataflows": [
    { "from": "config", "to": "review" }
  ]
}
```

**注意：** opencode CLI 的 `--format json` 输出的是 NDJSON event stream，而不是 Nexus 协议的单行 `{"route":"...","content":"..."}`。要对接 Nexus，需要用包装脚本把 NDJSON 转为 Nexus 格式。如果没有包装脚本，也可以直接用纯文本工具（python、echo）输出合规 JSON。

**规则：**
- 模板语法：`{{datarouter.<alias>.<field>}}`、`{{metadata.<field>}}`
- `<alias>` 必须对应 `dataflows` 中某个 `from` 的值，或该 dataflow 的 `alias`（如果设置了 `alias`，则模板中必须使用 alias 值）
- `<field>` 仅限 `route` 和 `content`（datarouter）或 `run_count` 和 `timed_out`（metadata）。此外 `{{node_dir}}`（无点号）为合法的独立模板变量


- 替换发生在 spawn 之前
- 不存在的 key 被替换为空字符串
- 普通文本原样透传。未识别的模板变量（如 `{{foo.bar}}`）在 `--validate-only` 时触发 `UnrecognizedTemplate` 错误，阻止执行

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

review 节点输出 `{"route":"approved","content":"..."}` 触发 deploy，输出 `{"route":"rejected","content":"..."}` 触发 fix。

```json
{
  "nodes": [
    {
      "id": "review",
      "providers": [{"type": "subprocess", "command": "python reviewer.py"}],
      "process_timeout_secs": 60
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
    { "from": "review", "to": "deploy", "trigger": "any", "event": "complete", "exit_reason": "approved" },
    { "from": "review", "to": "fix", "trigger": "any", "event": "complete", "exit_reason": "rejected" }
  ],
  "dataflows": []
}
```

注意 review 是入口节点（edges 中没有入边）。如果 review 需要前置节点提供数据，则在 edges 中添加前置依赖并在 dataflows 中声明数据传递。

### 6.4 有向环 + 上限保护（Directed Cycle with Route）

引擎支持通过**多节点有向环**实现重复执行，配合 `route_policy` 在上限到达后出环。

**示例：review → fix 循环 3 次后进入复盘**

review 节点配置 `route_policy.max_runs`，在达到 3 次后 NodeShell 将 DataRouter.route 改写为 "approved"，边匹配出环到 retro。review 节点自身输出 `{"route":"rejected","content":"..."}`，前两轮 route_policy 不干涉，DataRouter.route 保持为"rejected"维持循环；第 3 轮 route_policy 覆盖为"approved"出环。

```json
{
  "nodes": [
    {
      "id": "start",
      "providers": [{"type": "subprocess", "command": "echo start"}],
      "process_timeout_secs": 10
    },
    {
      "id": "review",
      "providers": [{"type": "subprocess", "command": "python review.py"}],
      "process_timeout_secs": 60,
      "route_policy": {
        "type": "max_runs",
        "max": 3,
        "then_route": "approved"
      }
    },
    {
      "id": "fix",
      "providers": [{"type": "subprocess", "command": "python fix.py"}],
      "process_timeout_secs": 120
    },
    {
      "id": "retro",
      "providers": [{"type": "subprocess", "command": "python retro.py"}],
      "process_timeout_secs": 60
    }
  ],
  "edges": [
    { "from": "start", "to": "review", "trigger": "any", "event": "complete" },
    { "from": "review", "to": "fix", "trigger": "any", "event": "complete", "exit_reason": "rejected" },
    { "from": "fix", "to": "review", "trigger": "any", "event": "complete" },
    { "from": "review", "to": "retro", "trigger": "any", "event": "complete", "exit_reason": "approved" }
  ],
  "dataflows": [
    { "from": "fix", "to": "review" }
  ]
}
```

**执行过程：**

| 轮次 | run_count | 节点输出 route | NodeShell 是否覆盖 | DataRouter.route 最终值 | 匹配边 |
|------|-----------|----------------|-------------------|----------------------|--------|
| 1 | 1 | "rejected" | ❌ 未达上限 | "rejected"（节点写入） | fix（exit_reason="rejected"） |
| 2 | 2 | "rejected" | ❌ 未达上限 | "rejected"（节点写入） | fix |
| 3 | 3 | "rejected" | ✅ 覆盖为 "approved" | "approved" | retro |

**关键要点：**
- review 节点正常输出 `{"route":"rejected","content":"..."}`，前 2 轮 route_policy 未触发，DataRouter.route 保持"rejected"，边匹配 fix 维持循环
- `route_policy` 配置了上限保护，NodeShell 在第 3 次运行时将 DataRouter.route 覆盖为 "approved"
- 两条出边：一条匹配 `exit_reason: "rejected"`（正常循环），一条匹配 `exit_reason: "approved"`（出环）
- review 节点不关心自己第几次运行——`run_count` 由引擎追踪，route_policy 在 NodeShell 侧评估
- **route_policy 的真正价值**：节点可以正常写 route 驱动循环，NodeShell 在上限到达时截断并改写，节点不需要知道上限的存在

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

## 7. LLM Agent 节点（`type: "llm"` / `type: "llm_sdk"`）

两种执行方式：**CLI 模式**（`type: "llm"`）通过命令行工具调用 LLM；**SDK 模式**（`type: "llm_sdk"`）通过 Anthropic Python SDK 直调，支持 token 级流式。两者共享相同的 stdin/stdout JSON 协议，可以互换使用。字段定义见 §1.3。

`type: "llm"`：指定 CLI 命令和 prompt，引擎自动处理：跨平台 CLI 发现、子进程管理、实时流式输出写 log、结果解析与路由。

**支持的 CLI：** `claude`、`opencode`、`nga`、`codeagent` 等任何可通过命令行非交互调用的 LLM 工具。

**关键模板变量：**
- `{{prompt}}` — 引擎渲染 prompt 模板后替换到 command 中
- `{{datarouter.X.content}}` — 上游节点 X 的输出内容
- `{{datarouter.X.route}}` — 上游节点 X 的 route 值
- `{{metadata.run_count}}` — 当前节点执行轮次
- `{{metadata.timed_out}}` — 上次执行是否超时
- `{{node_dir}}` — 当前节点的 scripts 目录路径

**llm_sdk 类型**：通过 Anthropic Python SDK 直调，支持 token 级流式输出（stderr）、原生 tool calling、structured output。

**环境要求**：`pip install anthropic`。引擎本身无 SDK 依赖——`llm_sdk.py` 是独立 Python 脚本，通过 `scripts_dir` 定位。

**凭证自动检测**（`llm_sdk.py` 启动时按序查找）：
1. `ANTHROPIC_API_KEY` 环境变量（SDK 标准）
2. `ANTHROPIC_AUTH_TOKEN` 环境变量（Claude Code 约定）
3. `~/.claude/settings.json` → `env` 字段（Claude Code 配置兜底）
4. `api_key_env` 指定的自定义环境变量名

`ANTHROPIC_BASE_URL` 同样自动从环境变量或 settings.json 读取——使用 DeepSeek 等非 Anthropic 端点时必须设置。

详见 §1.3。

**内部机制：**

**`llm` (CLI) — Agent 进程自闭环：**
1. 渲染模板 → 完整命令
2. 调用内置 Python wrapper（`llm_node.py`），跨平台解析 CLI 路径（Windows: .exe > .cmd > cmd /c；Linux: PATH 查找）
3. CLI 二进制自己完成 tool-use 循环（调 API → API 返回 tool_use → CLI 执行工具 → 回传 tool_result → 继续，直到产出最终文本）
4. 引擎 **只看到最终结果**：stdout 逐行转发到 log，进程退出后解析 `{route, content}`

**`llm_sdk` (SDK) — 裸 API 调用，wrapper 自己做 tool loop：**
1. 渲染模板 prompt
2. `llm_sdk.py` 调用 Anthropic SDK `client.messages.create()`
3. API 返回 `stop_reason == "tool_use"` → `llm_sdk.py` **必须自己**执行工具（`open()` / `os.rename()` / `os.makedirs()` 等）→ 构造 `tool_result` → 再次调 API
4. 重复直到 API 返回 `stop_reason == "end_turn"` → 提取文本 → 输出 `{route, content}`

**关键差异：谁负责执行工具？**

```
CLI 模式:  API ⇄ Claude CLI (内置 tool loop)  → 引擎只拿最终文本
SDK 模式:  API ⇄ llm_sdk.py (自己写 tool loop) → 引擎只拿最终文本
```

| | `type: "llm"` (CLI) | `type: "llm_sdk"` (SDK) |
|---|---|---|
| 谁调 API | Claude CLI 二进制 | `llm_sdk.py` (Python) |
| 谁执行 tool_use | CLI 内置，自动 | **必须自己在 wrapper 里实现 tool loop** |
| 文件系统 | CLI 自带 Read/Write/Edit 工具 | wrapper 用 Python `open()` / `os.rename()` 实现 |
| 引擎关心 tool 调用吗 | 不，完全透明 | 不，完全透明 |
| 适合场景 | 已有 CLI 工具，开箱即用 | 需要自定义工具、减少 CLI 依赖 |

> **一句话：CLI 模式是自带工具的 Agent 进程；SDK 模式是裸 API client，工具执行循环必须自己在 `llm_sdk.py` 里写。**

### 7.1 单节点 Claude 调用

```json
{
  "nodes": [{
    "id": "ask",
    "providers": [{
      "type": "llm",
      "command": "claude -p \"{{prompt}}\" --output-format json --verbose",
      "prompt": "test"
    }],
    "process_timeout_secs": 120
  }],
  "edges": []
}
```

### 7.2 单节点 OpenCode 调用

```json
{
  "nodes": [{
    "id": "ask",
    "providers": [{
      "type": "llm",
      "command": "opencode run --format json --auto -- \"{{prompt}}\"",
      "prompt": "test"
    }],
    "process_timeout_secs": 120
  }],
  "edges": []
}
```

### 7.3 代码审查（Review-Fix-Retro 循环）

```json
{
  "nodes": [
    {
      "id": "seed",
      "providers": [{"type": "shell", "command": "echo fn divide(a:i32,b:i32)->i32{a/b}"}],
      "process_timeout_secs": 10
    },
    {
      "id": "review",
      "providers": [{
        "type": "llm",
        "command": "claude -p \"{{prompt}}\" --output-format json --verbose",
        "prompt": "审查代码找bug: {{datarouter.seed.content}}。回复JSON: {\"route\":\"approved|rejected\",\"content\":\"审查意见\"}",
        "routes": ["approved", "rejected"]
      }],
      "process_timeout_secs": 120,
      "returns": ["approved", "rejected"]
    },
    {
      "id": "fix",
      "providers": [{
        "type": "llm",
        "command": "claude -p \"{{prompt}}\" --output-format json --verbose",
        "prompt": "修复代码: {{datarouter.review.content}}"
      }],
      "process_timeout_secs": 120
    },
    {
      "id": "retro",
      "providers": [{"type": "shell", "command": "echo done"}],
      "process_timeout_secs": 10
    }
  ],
  "edges": [
    {"from": "seed",   "to": "review", "trigger": "any", "event": "complete"},
    {"from": "review", "to": "fix",    "trigger": "any", "event": "complete", "exit_reason": "rejected"},
    {"from": "fix",    "to": "review", "trigger": "any", "event": "complete"},
    {"from": "review", "to": "retro",  "trigger": "any", "event": "complete", "exit_reason": "approved"}
  ],
  "dataflows": [
    {"from": "seed",   "to": "review"},
    {"from": "review", "to": "fix"},
    {"from": "fix",    "to": "review", "alias": "fixed_code"}
  ]
}
```

### 7.4 任意新 CLI（nga, codeagent 等）

```json
{
  "nodes": [{
    "id": "agent",
    "providers": [{
      "type": "llm",
      "command": "nga run --json \"{{prompt}}\"",
      "prompt": "分析: {{datarouter.data.content}}"
    }],
    "process_timeout_secs": 120
  }]
}
```

无需修改引擎代码，只需改 `command` 字段即可适配任何 CLI。

完整链式示例（config → LLM agent → report）：

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
        "type": "llm",
        "command": "opencode run --format json --auto -- \"{{prompt}}\"",
        "prompt": "检查代码: {{datarouter.config.content}}"
      }],
      "process_timeout_secs": 300
    },
    {
      "id": "report",
      "providers": [{"type": "subprocess", "command": "python save_report.py --input \"{{datarouter.llm_review.content}}\""}],
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

### 7.5 逐级上报审批流程

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
      "process_timeout_secs": 3600
    },
    {
      "id": "manager_review",
      "providers": [{"type": "subprocess", "command": "python manager_approve.py"}],
      "process_timeout_secs": 7200
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

### 7.6 单节点 llm_sdk 直调

```json
{
  "nodes": [{
    "id": "ask",
    "providers": [{
      "type": "llm_sdk",
      "model": "claude-sonnet-5-20251001",
      "prompt": "Say hello. Output ONLY: {\"route\":\"ok\",\"content\":\"Hello!\"}",
      "routes": ["ok"],
      "max_tokens": 256
    }],
    "process_timeout_secs": 60
  }],
  "edges": []
}
```

无需 CLI 或 command 字段。`llm_sdk.py` 通过 Anthropic SDK 直调，token 流式输出到 stderr（引擎 log），最终 stdout 输出 `{"route":"ok","content":"Hello!"}`。

### 7.7 审查循环 llm_sdk

将 §7.3 的审查循环改写为 SDK 模式——review 和 fix 节点都使用 `llm_sdk`，其余结构完全相同：

```json
{
  "nodes": [
    {
      "id": "seed",
      "providers": [{"type": "shell", "command": "echo fn divide(a:i32,b:i32)->i32{a/b}"}],
      "process_timeout_secs": 10
    },
    {
      "id": "review",
      "providers": [{
        "type": "llm_sdk",
        "model": "claude-sonnet-5-20251001",
        "system_prompt": "You are a code reviewer. Find bugs and security issues.",
        "prompt": "Review this code: {{datarouter.seed.content}}. Output ONLY: {\"route\":\"approved|rejected\",\"content\":\"review notes\"}",
        "routes": ["approved", "rejected"],
        "max_tokens": 1024
      }],
      "process_timeout_secs": 120
    },
    {
      "id": "fix",
      "providers": [{
        "type": "llm_sdk",
        "model": "claude-sonnet-5-20251001",
        "prompt": "Fix the issues found: {{datarouter.review.content}}. Output ONLY: {\"route\":\"done\",\"content\":\"fixed code\"}",
        "routes": ["done"],
        "max_tokens": 2048
      }],
      "process_timeout_secs": 120
    },
    {
      "id": "retro",
      "providers": [{"type": "shell", "command": "echo done"}],
      "process_timeout_secs": 10
    }
  ],
  "edges": [
    {"from": "seed",   "to": "review", "trigger": "any", "event": "complete"},
    {"from": "review", "to": "fix",    "trigger": "any", "event": "complete", "exit_reason": "rejected"},
    {"from": "fix",    "to": "review", "trigger": "any", "event": "complete"},
    {"from": "review", "to": "retro",  "trigger": "any", "event": "complete", "exit_reason": "approved"}
  ],
  "dataflows": [
    {"from": "seed",   "to": "review"},
    {"from": "review", "to": "fix"},
    {"from": "fix",    "to": "review", "alias": "fixed_code"}
  ]
}
```

与 CLI 版本（§7.3）的区别：`type: "llm_sdk"` 替代 `type: "llm"`，`model` + `system_prompt` + `max_tokens` 替代 `command`。edges 和 dataflows 完全相同——两种 provider 在同一套拓扑定义下可互换。

---

## 8. 边界情况与限制

### 8.1 空图

零节点的工作流是合法的，引擎立即收敛。

### 8.2 孤立节点

多个相互之间没有边连接的节点是合法的——它们同时作为入口节点被触发。

### 8.3 自环边

节点可以声明指向自己的边（自环）。自环边也需要遵循阈值的收敛约束：
- 如果自环边 `threshold = 1` 且节点没有其他出边 → Validator 报 `cycle without entry: deadlock detected`
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

exit_reason 来自节点 stdout JSON 的 `route` 字段，与边的 `exit_reason` 精确字符串匹配。尾随空格、大小写都影响匹配结果。`"approved"` ≠ `"approved "`。

**注意：** `route` 为空字符串时视为无 exit_reason。若节点输出了 JSON 但 `route` 为空，则只匹配 `exit_reason: null` 的边。若节点输出了纯文本（非 JSON），引擎报 `SpawnError`，节点标记为 failed。

### 8.7 引擎保障 vs 用户责任

引擎和用户之间有一条清晰的边界：**引擎保障执行安全，用户负责拓扑正确。**

#### 引擎保障（Engine's Contract）

引擎在运行时提供以下保证：

| 保障 | 行为 |
|------|------|
| **不并发重复执行同一节点** | 节点正在 Running 时，引擎拒绝新到达的 `NodeReady`，防止同一节点同时运行两份 |
| **状态机合法** | 节点状态仅沿 `Pending → Running → Completed/Failed/TimedOut` 流转。重入时（循环、重试）从终态回到 Running |
| **All 策略** | `fan_in_pending` 计数器保证所有上游到齐后才触发一次，触发后自动重置供下一轮使用 |
| **Any 策略** | 每条匹配的边独立触发——引擎忠实执行用户声明的拓扑 |
| **收敛** | 节点全部终态 + ready_queue 为空 → 正常收敛。连续 30s 无事件且无 Running 节点 → watchdog 将 Pending 节点标记 Skipped 并强制收敛 |

#### 用户责任（Workflow Author's Contract）

工作流作者对自己声明的拓扑语义负责：

| 场景                          | 须知                                                                                                                             |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| **多条 Any 边指向同一 target**     | 如果 A→C (Any) 且 B→C (Any)，A 和 B 都完成时，C 会被触发 N 次（N = 匹配边数）。引擎不会去重，因为这是用户声明的拓扑。"任一即触发一次"的正确写法是用一条边即可（A→C 即可实现 C 在 A 完成后执行），不需要两条边 |
| **exit_reason 分支路由**        | 确保每条分支声明了不同的 `exit_reason` 过滤条件。如果多条边声明了相同的匹配条件，它们会同时触发——这是拓扑声明问题，不是引擎 bug                                                     |
| **环 + Any**                 | 环中节点每轮可被触发一次。必须用 `route_policy.max_runs` 或 `threshold > 1` 终圈，否则无限循环。引擎不主动检测无限循环                                               |
| **dataflow 不替代 scheduling** | `dataflows` 声明数据流向，`edges` 声明调度依赖。仅声明 dataflow 而没有对应的 scheduling edge → 下游可能在上游完成前执行，收到空数据                                     |
| **dataflow 不保证"同轮"数据**      | 环场景中同一节点多次执行会覆盖输出，下游通过 dataflow 收到的是最新一次的值。需区分轮次时用 `metadata.run_count`                                                        |

> **一句话：** 引擎说"你声明什么拓扑，我忠实执行，且保证不并发重跑"。用户说"我声明了正确的拓扑——如果 A 和 B 都 Any→C 导致 C 跑两次，那是我声明的问题"。

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
  3  Node timeout
```

### 9.2 验证错误速查

`nexus-cli run --validate-only` 在 JSON 无效时打印以下错误：

| 错误信息 | 原因 |
| --- | --- |
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
| `node 'X' has process_timeout_secs = 0` | `process_timeout_secs` 设为 0，节点会立即超时。设为一个正数 |
| `datarouter ref 'Y' in node 'X' has no matching dataflow` | 模板引用 `{{datarouter.Y.*}}` 但未声明 `{"from":"Y","to":"X"}` 的 dataflow |
| `unrecognized template '{{...}}' in node 'X'` | 模板变量不在允许列表中。仅 `datarouter.*.*`、`metadata.*`、`node_dir` 有效 |
| `dataflow 'Y→X' has no scheduling edge → X will start before Y produces data` | 有 dataflow 但没有对应 scheduling edge。此为警告，不阻塞执行 |
| `route 'Z' of node 'X' has no matching edge` | LLM 节点声明的 route 值没有对应 `exit_reason` 的边 |

### 9.3 通用调试步骤

1. **验证 JSON 结构**：`nexus-cli run workflow.json --validate-only`，修复所有错误
2. **单节点测试**：先用一个 echo 节点验证基础流程 `nexus-cli run --validate-only`
3. **查看节点输出**：`nexus-cli run workflow.json --verbose` 查看节点 JSON 输出及状态变化
4. **查看最终状态**：`nexus-cli run workflow.json --dump-state` 查看所有节点终态
5. **检查日志**：运行日志写入 `log/run-{timestamp}.log`

### 9.4 Dashboard 集成

Dashboard 是 HTTP REST API + WebSocket 服务端（默认 `http://127.0.0.1:48080`），支持工作流的 CRUD 和远程执行。

#### 启动

```bash
# Windows
./bin/nexus-dashboard.exe
# Linux
./bin/linux/nexus-dashboard
```

环境变量 `NEXUS_HOST` / `NEXUS_PORT` 可自定义监听地址和端口。

#### 加载工作流

**API 格式要求（关键）：** 必须使用 `{"name":"...", "definition":{...}}` 包装格式。直接传裸工作流 JSON 会导致 `definition` 缺失，Dashboard 存入空对象 `{}`，View 界面为空。

```bash
# ❌ 错误：裸 JSON
curl -X POST http://127.0.0.1:48080/api/workflows \
  -H "Content-Type: application/json" -d @workflow.json

# ✅ 正确：包装格式
curl -X POST http://127.0.0.1:48080/api/workflows \
  -H "Content-Type: application/json" \
  -d '{"name":"My Workflow","definition":{"nodes":[...],"edges":[...],"dataflows":[...]}}'
```

`definition` 字段可以是 JSON object 或 JSON string，Dashboard 内部 auto-serialize 为 string 存储。

**UTF-8 编码：** 当 workflow JSON 含 CJK 字符或 emoji 时，curl/bash 管道可能损坏编码，导致 `invalid unicode code point`。此时用 Python 直接 POST：

```python
import json, urllib.request
with open('workflow.json', 'r', encoding='utf-8') as f:
    definition = json.load(f)
body = json.dumps(
    {'name': 'My Workflow', 'definition': definition},
    ensure_ascii=False
).encode('utf-8')
req = urllib.request.Request(
    'http://127.0.0.1:48080/api/workflows',
    data=body, method='POST'
)
req.add_header('Content-Type', 'application/json; charset=utf-8')
resp = urllib.request.urlopen(req)
print(resp.read().decode('utf-8'))  # → {"id":"...","status":"created"}
```

#### API 端点

| 方法 | 路径 | 说明 |
|------|------|------|
| `GET` | `/api/workflows` | 列出所有工作流 |
| `POST` | `/api/workflows` | 创建工作流 `{"name":"...","definition":{...}}` |
| `GET` | `/api/workflows/{id}` | 获取工作流详情（definition 已反序列化） |
| `PUT` | `/api/workflows/{id}` | 更新工作流 |
| `DELETE` | `/api/workflows/{id}` | 删除工作流 |
| `POST` | `/api/workflows/{id}/run` | 触发运行 → `{run_id, dashboard_url}` |
| `GET` | `/api/runs` | 列出所有运行记录 |
| `GET` | `/api/runs/{id}` | 获取运行详情 |
| `WS` | `/ws/runs/{run_id}` | WebSocket 实时状态推送 |

#### 触发运行

```bash
curl -X POST http://127.0.0.1:48080/api/workflows/{id}/run
# → {"run_id":"...","dashboard_url":"http://127.0.0.1:48080"}
```

运行状态通过 WebSocket `ws://127.0.0.1:48080/ws/runs/{run_id}` 实时推送。

---

## 10. 相关文档

- [QUICKSTART.md](./QUICKSTART.md) — 5 分钟入门
- [README.md](./README.md) — 快速开始和 API 参考
- [NEXUS_WORKFLOW_SKILL.md](./NEXUS_WORKFLOW_SKILL.md) — Claude Code skill 参考
- [DESIGN_PRINCIPLES.md](./DESIGN_PRINCIPLES.md) — 设计原则与工程洞见（脱离具体项目的通用经验）
