# Issue 001: BuildResult → GraphDef + RuntimeState 分离

**类型**: 工程实现  
**来源**: 架构审查问题 1b / 4（与 002 统一为 Spec / Status 模式）  
**状态**: ✅ **已实现**

---

## 问题描述

设计阶段 `BuildResult` 的 5 个字段之间存在 3 个隐式约束（不变量），但没有任何机制保证它们：

| 不变量 | 含义 | 违反后果 |
|--------|------|---------|
| Entry reachability | `entry_nodes` 中所有 `NodeIndex` 在 `graph` 中有效 | `graph.node_weight(entry)` panic |
| Edge validity | 所有 `Edge` 的 `from_nodes()` 和 `to()` 在 `graph` 中有效 | Scheduler 访问边时 index out of bounds |
| Param coverage | `node_params` 的 key 覆盖 `graph` 中所有节点 | 运行时 `params[node]` 找不到 key |

## 当前状态

✅ **已实现**。代码中将 `BuildResult` 替换为 `GraphDef`（`nexus-engine/src/graph/graph_def.rs`）：
- 所有字段私有，唯一构造路径 `GraphDef::from_components()`
- 构造时 `invariants_hold()` 验证 5 条不变量（比设计时多了 transfers 覆盖和 edge-in-transfer 两条）
- `NodeTransfer` 索引在 Builder 阶段构建，纳入 `GraphDef` 统一管理
- 运行时状态分离到 `RuntimeState`（Scheduler 持有）
- 外部通过安全访问器读取（`entry_nodes()`, `node_index()`, `edges()`, `transfers()`, `node_params()` 等）

## 与相关 issue 的关系

- **Issue 001**（本文件）：`BuildResult` → `GraphDef`，构造期断言 ✅ 已实现
- **Issue 002**：`Edge` 状态分离 → 纳入 `RuntimeState.edge_states` ✅ 已实现
