# Issue 012: 重试与自环边分离

**类型**: 设计修正  
**来源**: Deep Review 问题 3 深化  
**状态**: ✅ **已实现** — retry 从自环边分离为 Scheduler 独立计数（`retry_counts` + `retry_node()`），Builder 不再添加默认自环边。见 ARCHITECTURE.md §5.4

---

## 问题描述

当前 Builder 为节点的 Failed/Timeout 事件添加默认自环边作为重试机制。但自环边和重试是两种不同的语义：

| | 自环边 | 重试 |
|---|---|---|
| 触发条件 | 节点正常完成但阈值未到 | 节点 Failed/Timeout |
| 语义 | "再执行一次" | "刚才失败了，再试一次" |
| 配置方式 | 用户显式声明 | 引擎默认行为 |

混用导致自环边的 threshold 和重试次数概念混淆。

## 解决方案

重试从自环边中分离为 Scheduler 的独立计数机制。

### Builder 变更

Builder **不再**为 Failed/Timeout 添加默认自环边。

### Scheduler 变更

Scheduler 维护每个节点的 `retry_count`，节点 Failed/Timeout 时：

```
节点 Failed/Timeout
  → retry_count[node]++
  → if retry_count[node] < max_retries:
      → 发 NodeReady(node)  // 重试
  → else:
      → TerminatedFailed     // 超过重试次数
```

```rust
pub struct RuntimeState {
    pub states: HashMap<NodeIndex, NodeState>,
    pub counters: HashMap<NodeIndex, NodeCounters>,
    pub retry_counts: HashMap<NodeIndex, u64>,  // 每节点重试计数
    pub edge_states: Vec<EdgeState>,
    pub ready_queue: VecDeque<NodeIndex>,
}

pub struct EngineConfig {
    pub max_concurrency: usize,
    pub global_timeout: Duration,
    pub max_retries: u64,         // 全局默认重试次数
}
```

### 自环边保持现有行为

用户显式声明的自环边不受影响（旧版 `predecessors` 格式，等价于新版顶层 `edges`）：

```json
{
  "id": "review"
}
```

新版在 `WorkflowDef` 顶层声明：

```json
{
  "nodes": [{ "id": "review" }],
  "edges": [
    { "from": "generate", "to": "review", "trigger": "all", "event": "complete", "threshold": 5 }
  ]
}
```

threshold=5 表示"正常完成 5 次后才走到下游"——这是自环，不是重试。

## 影响范围

- Builder 移除默认自环边
- Scheduler 新增 retry_counts 和重试逻辑
- EdgeState 不需要 reset_on_trigger
- handle_event 保持当前逻辑不变

## 优先级

P0 — 编码前处理
