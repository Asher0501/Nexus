# 节点转移函数（NodeTransfer）——局部闭包定理的实现桥梁

> 设计补充，作为 DESIGN_PHILOSOPHY（局部闭包定理）到工程实现之间的映射层。

**分类**：theory

---

## 一、转移函数的核心分解

### 1.1 边的本质：h + g

每条边是由两个正交函数构成的组合：

```
边 e = (source, target, h_e, g_e)
```

| 符号 | 名称 | 定义 | 职责 |
|------|------|------|------|
| `source` | 源节点 | `v ∈ V` | 谁的事件触发这条边 |
| `target` | 目标节点 | `w ∈ V` | 谁被触发 |
| `h_e` | 分支匹配函数 | `(exit_code, exit_reason) → bool` | 判断本次事件是否匹配这条边 |
| `g_e` | 策略聚合函数 | `{source 的就绪状态} → bool` | 判断所有源节点是否已就绪 |

### 1.2 h：分支匹配

`h_e` 是纯函数——不记忆、不计数、不依赖历史状态。同样的输入永远产生同样的输出。

```
h_e(event_type, exit_reason) =
    edge.event_type = event_type                ← 事件类型匹配
    ∧ (edge.exit_reason = None                  ← 未设过滤，全部匹配
        ∨ edge.exit_reason = exit_reason)       ← 精确字符串匹配
    ∧ event_count + 1 ≥ edge.threshold          ← 阈值满足
```

`h_e` 中唯一的状态是 `event_count`，但它是**阈值计数器**而非记忆——它只回答"事件到达次数够了吗"，不回答"这条边以前触发过吗"。

**h_e 不包含 `triggered`。** 因为"这条边是否触发过"不属于 h_e 的职责。

### 1.3 g：策略聚合

`g_e` 把一组源节点的就绪状态聚合为一个布尔值：

```
g_e({source1, source2, ..., sourceN}) =
    Any ∨ All

Any:  g = true  (单 source，直接触发)
All:  g = ∀u ∈ sources: u.ready  (fan_in_pending 归零)
```

`g_e` 通过 `fan_in_pending[target]` 实现：每条 All 边触发时递减，归零时目标入队，然后重置为初始值。这样每轮都是一个全新的 g 评估周期。

### 1.4 完整的 f_v

```
f_v(v, event_type, exit_reason) =
    { w | e = (v, w, h_e, g_e) ∈ E
          ∧ h_e(event_type, exit_reason)
          ∧ g_e({ u.ready | u ∈ pred(w) }) }
```

其中 `pred(w)` 是所有指向 w 的边的 source 节点集。

### 1.5 关键设计原则

| 原则 | 含义 | 代码体现 |
|------|------|----------|
| **h 是无状态的** | h 不记忆边是否触发过 | 没有 `triggered`、没有 `fires_count` |
| **h 和 g 正交** | 分支匹配与策略聚合互不干扰 | h 在单边上判定，g 跨多边聚合 |
| **g 每轮重置** | 目标入队后，g 的就绪状态清空重来 | `fan_in_pending` 在下游入队后自动恢复 |
| **f_v 是纯函数** | 相同的输入产生相同的输出 | 不依赖时序、调用次数、历史状态 |

---

## 二、工程映射

### 2.1 边：EdgeDef (h) + EdgeState

```rust
/// 边的定义——纯数据，运行时只读。
///
/// 对应数学公式中的边 e = (source, target, h_e, g_e)：
/// - from, to 决定了 source 和 target
/// - event_type, exit_reason, threshold 是 h_e 的参数
/// - strategy 是 g_e 的策略选择
pub struct EdgeDef {
    pub from: NodeIndex,        // 源节点
    pub to: NodeIndex,          // 目标节点
    pub event_type: EventType,  // h_e：匹配的事件类型
    pub exit_reason: Option<String>, // h_e：匹配的 exit_reason 过滤
    pub threshold: u64,         // h_e：阈值
    pub strategy: Strategy,     // g_e：Any 或 All
}

/// 边的运行时状态。
///
/// 只有 h_e 的阈值计数器（event_count）和 g_e 的聚合状态（received）。
/// 没有 triggered——h_e 是纯函数。
pub struct EdgeState {
    pub event_count: u64,             // h_e：阈值计数器
    pub received: HashSet<NodeIndex>,  // g_e：All 策略下已就绪的 source 集合
}
```

### 2.2 节点转移函数：NodeTransfer

```rust
/// 节点 v 的局部转移函数 f_v。
///
/// 对应局部闭包定理中的 f_v: State_v → 2^V。
/// 聚合了该节点的所有出边 e = (v, _, h_e, g_e)。
pub struct NodeTransfer {
    pub from: NodeIndex,
    pub out_edge_indices: Vec<usize>,  // 指向 edges[] 的索引
}
```

### 2.3 f_v 的实现

```rust
/// 实现 f_v：对节点 v 的每条出边 e = (v, w, h_e, g_e)，依次：
/// 1. 调用 h_e——匹配事件类型、exit_reason、threshold
/// 2. 调用 g_e——Any 立即触发，All 等待所有 source 就绪
///
/// h_e 是纯函数，不依赖 triggered。
/// g_e 每轮重置（fan_in_pending 在下游入队后恢复）。
pub fn handle_event(
    &mut self,
    node: NodeIndex,
    event: EventType,
    exit_reason: Option<&str>,
) -> Vec<NodeIndex> {
    let mut ready = Vec::new();
    let Some(transfer) = self.transfers.get(&node) else {
        return ready;
    };

    for &edge_idx in &transfer.out_edge_indices {
        let edge = &self.edges[edge_idx];
        let state = &mut self.edge_states[edge_idx];

        // ── h_e：分支匹配 ──
        if edge.event_type != event { continue; }
        if let Some(reason) = &edge.exit_reason {
            if exit_reason != Some(reason.as_str()) { continue; }
        }
        state.event_count += 1;
        if state.event_count < edge.threshold { continue; }

        // ── g_e：策略聚合 ──
        match edge.strategy {
            Strategy::Any => {
                self.state.ready_queue.push_back(edge.to);
                ready.push(edge.to);
            }
            Strategy::All => {
                if let Some(pending) = self.state.fan_in_pending.get_mut(&edge.to) {
                    if *pending > 0 { *pending -= 1; }
                }
                let all_ready = self.state.fan_in_pending
                    .get(&edge.to).map(|&p| p == 0).unwrap_or(true);
                if all_ready {
                    self.state.ready_queue.push_back(edge.to);
                    ready.push(edge.to);
                    // g_e 重置
                    if let Some(pending) = self.state.fan_in_pending.get_mut(&edge.to) {
                        *pending = self.graph.edges()
                            .iter().filter(|e| e.to == edge.to && e.strategy == Strategy::All).count();
                    }
                }
            }
        }
    }
    ready
}
```

---

## 三、与设计哲学的一致性检查

### 3.1 当前代码状态

| 检查项 | 状态 | 说明 |
|--------|------|------|
| **h 是无状态的** — 没有 triggered | ✅ | `EdgeState` 已无 `triggered` 字段 |
| **h 和 g 正交** | ✅ | `handle_event` 中 h 判定完毕后才进入 g 聚合 |
| **g 每轮重置** | ✅ | `fan_in_pending` 在下游入队后自动恢复初始值 |
| **没有 fires_count / max_fires** | ✅ | 已从所有代码中移除 |
| **边不记忆触发历史** | ✅ | `retry_node` 只重置 `event_count`，不恢复 `triggered` |
| **循环退出靠 engine 层** | ❓ | `max_runs` 相关代码已被移除，当前无循环退出机制 |
| **exit_reason 路由** | ✅ | 通过 `EdgeDef.exit_reason` 精确匹配 |
| **All 多轮支持** | ✅ | `fan_in_pending` 每轮重置，测试覆盖 |

### 3.2 发现的问题

**循环退出机制缺失：** `max_runs` 相关的代码（`NodeParams.max_runs`、`RuntimeState.run_counts`、engine 中的 max_runs 检查）在前面某次重构中被完全移除了。当前没有循环退出机制——有向环会无限转下去，直到永远。

这是一个独立的 feature，不在本次去 triggered 的范围内。如果需要加回来，需要在：
- `NodeParams` 加回 `max_runs: Option<u64>`
- `RuntimeState` 加回 `run_counts: HashMap<NodeIndex, u64>`
- `engine.rs` 加回 `NodeReady` 中的 max_runs 检查

---

## 四、形式化定义

### 4.1 h_e 的数学形式

```
h_e(event, exit_reason) =
    edge.event_type = event
    ∧ (edge.exit_reason = None ∨ edge.exit_reason = exit_reason)
    ∧ event_count + 1 ≥ edge.threshold
```

### 4.2 g_e 的数学形式

```
g_e({sources}) =
    Any:  true  (单 source, 直接触发)
    All:  fan_in_pending[target] = 0
          （重置：fan_in_pending[target] = count of All edges → target）
```

### 4.3 f_v 的完整形式

```
f_v(v, event, exit_reason) =
    { w | e = (v, w, h_e, g_e) ∈ E
          ∧ h_e(event, exit_reason)
          ∧ g_e({ u.ready | u ∈ pred(w) }) }
```

---

> **哲学总结：** 边 = (h, g)。h 回答"这次事件是否匹配"，g 回答"现在可以触发下游吗"。两者正交，都是纯函数。f_v 是它们的组合。没有 `triggered`、没有记忆、没有状态污染。
