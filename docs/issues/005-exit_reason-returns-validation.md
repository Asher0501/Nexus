# Issue 005: Validator L2 验证项补充

**类型**: 工程实现  
**来源**: 架构审查问题 7  
**状态**: 待实现

---

## 问题描述

当前 Validator 只覆盖 L1（结构正确性），缺少 L2（绑定正确性）的验证项。

需要补充以下三类验证：

### 1. exit_reason / returns 相关

1. **边引用的 exit_reason 不在节点的 returns 中**：如果节点定义 `returns: ["approved", "rejected"]`，但某条 Complete 边配置了 `exit_reason: "pending"`，运行时会永远不触发
2. **exit_reason 声明了但没配置匹配的边**：节点声明了 `returns: ["approved"]`，但没有 Complete 边匹配 `exit_reason: "approved"`——节点返回这个值时不会触发任何下游
3. **边配置了 exit_reason 但节点没有 returns**：用户在某条 Complete 边上写了 `exit_reason: "ok"`，但节点没有声明 `returns` 字段——`exit_reason` 永远不会被填，这条边永远不触发
4. **没有默认出口**：某事件类型下的所有出边都配置了 exit_reason，但节点的 returns 和 exit_reason 可能覆盖不全。建议 Warning 级别的检查——如果该事件类型下没有 exit_reason=None 的边，且可声明的 exit_reason 值没有被完全覆盖，发出警告

### 2. InputSource 相关

4. **inputs 引用的节点不存在**：节点声明了 `inputs: ["node_X"]`，但 `node_X` 不在 `nodes` 数组中。运行时 DataRouter 返回空数据，但没有任何地方报错
5. **inputs 引用的节点从入口不可达**：`node_X` 存在但从入口节点无法到达它。运行时可能一直等到超时

### 3. Threshold 可达性（未来扩展）

6. **threshold 超过可能的事件上限**：如果一条 Complete 边的 threshold=100，但所有能触发这条边的上游节点的 Complete 事件总数上限是 50——这条边永远达不到 threshold

## 解决方案

在 Validator 中补充以下验证项：

| 验证项 | 说明 |
|--------|------|
| ExitReasonMismatch | 边引用的 exit_reason 不在对应节点的 returns 中 |
| ExitReasonUnused | 节点的 returns 中的值，没有对应的 Complete 出边使用 |
| ExitReasonWithoutReturns | 边配置了 exit_reason 但节点没有声明 returns |
| NoDefaultExitReasonEdge | 某事件类型下所有出边都配置了 exit_reason，没有兜底边 |
| InputSourceNotFound | inputs 引用的节点 ID 在 nodes 中不存在 |
| InputSourceUnreachable | inputs 引用的节点从入口不可达 |

```rust
pub enum ValidationError {
    // ... 已有项 ...

    ExitReasonMismatch { node_id: String, reason: String },
    ExitReasonUnused { node_id: String, returns: String },
    ExitReasonWithoutReturns { node_id: String, reason: String },
    InputSourceNotFound { node_id: String, source: String },
    InputSourceUnreachable { node_id: String, source: String },
}
```

## 影响范围

- Validator 新增 5 个验证项
- 不影响运行时逻辑

## 优先级

P1 — 编码阶段纳入

