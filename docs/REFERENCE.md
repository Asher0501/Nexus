# Nexus Reference

> Agent 工作参考。包含工作流定义、节点协议、运行时机制和 CLI 的全部 schema/结构说明。
> 不包含内部决策记录、检视意见和敏感信息。

---

## 工作流定义

### WorkflowDef — 工作流定义

顶层 JSON 结构：

```json
{
  "nodes": [
    {
      "id": "<string, 唯一节点 ID>",
      "providers": [{"type": "subprocess", "command": "<string>"}],
      "process_timeout_secs": 30,
      "max_concurrency": null,
      "returns": [],
      "max_retries": null
    }
  ],
  "edges": [
    {"from": "<上游 ID>", "to": "<当前节点 ID>", "trigger": "all|any", "event": "complete|failed|timeout"}
  ],
  "dataflows": [
    {"from": "<上游节点 ID>", "to": "<当前节点 ID>"}
  ]
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `id` | string | ✅ | 唯一标识符 |
| `providers` | array | ✅ | 执行方式。`{"type":"subprocess","command":"..."}` 唯一类型（所有工具统一） |
| `process_timeout_secs` | u64 | ✅ | 节点超时秒数 |
| `edges` | array | ❌ | 边数组，定义调度拓扑（from → to），详见下方 EdgesDef |
| `dataflows` | array | ❌ | 数据流声明，定义谁的数据传给谁，详见下方 DataFlowDef |
| `max_concurrency` | u64 or null | ❌ | 该节点的最大并发数，null = 继承引擎全局值 |
| `returns` | string[] | ❌ | 分支路由的可选值 |
| `max_retries` | u64 or null | ❌ | 最大重试次数，null = 继承引擎全局默认值 3 |

> **注意**：不再有 `type: "ai"` 变体。所有工具统一通过 `type: "subprocess"` 接入。
> AI CLI（OpenCode、Claude Code）和其他子进程一样，直接在 command 中写完整调用。

### DataFlowDef — 数据流定义

```json
{
  "from": "上游节点 ID",
  "to": "当前节点 ID"
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `from` | string | ✅ | 上游节点 ID（数据来源） |
| `to` | string | ✅ | 当前节点 ID（数据去向） |

### EdgesDef — 边定义

```json
{
  "from": "上游节点 ID",
  "to": "下游节点 ID",
  "trigger": "all",
  "event": "complete",
  "exit_reason": null,
  "threshold": 1
}
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `from` | string | ✅ | — | 上游节点 ID（起点） |
| `to` | string | ✅ | — | 下游节点 ID（终点） |
| `trigger` | "all" \| "any" | ✅ | — | All: 所有上游到齐后才计数。Any: 任一上游事件直接计数 |
| `event` | "complete" \| "failed" \| "timeout" | ✅ | — | 匹配的事件类型 |
| `exit_reason` | string or null | ❌ | null | 匹配的退出原因（字符串精确匹配） |
| `threshold` | u64 | ❌ | 1 | 触发所需的事件次数 |

---

## 节点协议（NODE_PROTOCOL）

任何进程只要遵循以下协议即可成为 Nexus 节点，**不需要改引擎代码**。

### 通信流程

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

### stdin 输入格式

```json
{
  "inputs": {
    "上游节点ID": "该节点输出的纯文本"
  },
  "extensions": {}
}
```

- `inputs`：上游节点的输出。key = 来源节点 ID，value = 纯文本
- `extensions`：节点特有的配置参数（保留字段）

### stdout 输出格式

**普通输出**（无分支路由）：直接写纯文本，每行实时上报

```
代码评审结果：
1. 风格良好
2. 建议添加类型注解
```

**流式事件**（实时进度）：行前缀 `__nexus_event:`

```
__nexus_event: 进度 2/5
__nexus_event: tool_use 分析代码结构
```

**中间日志**（不占输出）：行前缀 `__nexus_log:`

```
__nexus_log: 开始处理文件 main.ts
```

**分支路由输出**（有 exit_reason）：行前缀 `__nexus_exit_reason:`

```
__nexus_exit_reason: approved
{"result": "ok", "data": [...]}
```

NodeShell 提取 `exit_reason`，剩余内容作为业务输出。

### 退出码

| 码 | 含义 | 引擎行为 |
|----|------|---------|
| 0 | 正常完成 | 采用 stdout 内容作为节点输出 |
| 非0 | 异常 | 走失败处理路径 |
| 被信号杀死 | 异常终止 | 走失败处理路径 |

### 不需要节点关心的事

| 引擎负责 | 节点不需要关心 |
|---------|--------------|
| 超时控制 | 引擎强杀超时进程 |
| 重试 | 引擎决定是否重新启动节点 |
| 并发 | 引擎管理并发度 |
| 数据来源 | 引擎传入工作流定义中声明的数据 |
| exit_reason 提取 | 按 `__nexus_exit_reason:` 写 stdout 即可 |
| 流式输出 | stdout 逐行实时读取，节点不需要等退出 |

---

## 引擎运行时机制

### 边（Edge）语义

边由四个正交维度定义：

| 维度 | 取值 | 引擎是否理解含义 |
|------|------|----------------|
| 事件类型 | Complete / Failed / Timeout | ✅ 是——执行事实 |
| 返回值 | `exit_reason: Option<String>` | ❌ 否——纯字符串匹配 |
| 组合逻辑 | All / Any | ✅ 是——数学逻辑 |
| 阈值 | `threshold: u64` | ✅ 是——整数比较 |

**边触发算法**（`Scheduler::handle_event`）：

```
for each out_edge of node:
  1. 跳过已触发的边
  2. 跳过事件类型不匹配的边
  3. 跳过 exit_reason 不匹配的边
  4. All 策略：收集 received 集合，未到齐则跳过
  5. event_count++
  6. event_count >= threshold && !triggered → triggered=true, 入队目标节点
```

### 并发控制

引擎使用 Semaphore 限制同时执行的节点数。许可用尽时自动等待。

| 配置来源 | 字段 | 默认值 |
|---------|------|--------|
| 引擎全局 | `EngineConfig.max_concurrency` | CPU 核数 |

### 数据路由

DataRouter 采用 snapshot semantics（最新快照语义）：
- 每个节点只保留最新一次的输出
- 目标节点触发时获取所有上游的最新输出
- 不保证输出来自同一轮执行

### 流式输出（NodeChunk）

引擎为每个执行中的节点创建独立的 chunk channel。每行 stdout 在子进程运行时实时发送：

```rust
let (chunk_tx, mut chunk_rx) = mpsc::unbounded_channel::<NodeChunk>();

// 后台消费任务：实时 emit 到日志
tokio::spawn(async move {
    while let Some(chunk) = chunk_rx.recv().await {
        tracing::info!(target: "nexus::node::chunk", node_id, text = chunk.text);
    }
});

// 传给 executor
let outcome = executor.run(ctx, timeout, &nid, Some(chunk_tx)).await;
```

- `--verbose` 模式在终端可见
- 节点退出后，完整 output 进入 DataRouter
- `chunk_tx` 为 `Option`——不传则不发送，保持向后兼容

### 重试机制

节点 Failed 或 Timeout 后，引擎在触发下游边之前先检查重试计数：

```
节点 Failed/Timeout
  → retry_count < max_retries? (默认 3)
    → retry_count++ → 重新执行节点（不触发下游边）
  → 否则触发 Failed/Timeout 出边
```

| 级别 | 配置项 | 默认值 |
|------|--------|--------|
| 引擎全局 | `EngineConfig.max_retries` | 3 |
| 节点级 | `NodeDef.max_retries` | 继承全局 |

---

## CLI

```
nexus run <workflow.json>

  --max-concurrency N    最大并发节点数（默认：CPU 核数）
  --node-timeout S       节点默认超时秒数（默认：3600）
  --verbose              详细日志（含流式 chunk 输出）
  --validate-only        仅验证，不执行
  --dump-state           输出节点状态快照

退出码：
  0  Success
  1  Validation error
  2  Runtime error
  3  Idle timeout
```

---

## 依赖

| 依赖 | 用途 |
|------|------|
| petgraph 0.8 | 图数据结构 + SCC 算法 |
| tokio 1.x | 异步运行时 + 子进程管理 |
| serde + serde_json 1.x | JSON 序列化/反序列化 |
| thiserror 2.x | 错误类型推导 |
| clap 4.x | CLI 参数解析 |
| tracing 0.1 | 结构化日志 |

---

## 接入外部工具（不改引擎代码）

所有工具统一通过 `type: "subprocess"` 接入。直接在 command 中写完整调用。
子进程的 stdout 被逐行实时读取，进程退出码决定完成状态。

| 工具 | 示例 command |
|------|-------------|
| OpenCode | `opencode run --format json --dangerously-skip-permissions -- "prompt"` |
| Claude Code | `claude -p "prompt" --output-format json --model claude-sonnet-4` |
| ESLint | `npx eslint --format json src/` |
| pytest | `pytest --json-report` |
| 任何脚本 | 按 NODE_PROTOCOL 实现 |
