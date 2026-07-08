# 节点转移函数（NodeTransfer）——局部闭包定理的实现桥梁

> 设计补充，作为 DESIGN_PHILOSOPHY（局部闭包定理）到工程实现之间的映射层。
> 与 ARCHITECTURE.md 并列阅读。

**分类**：theory

---

## 一、问题

### 1.1 命题与实现的映射缺口

**局部闭包定理（DESIGN_PHILOSOPHY.md §〇）：**

> 对于任意有限有向图 G = (V, E)，若每个顶点 v ∈ V 附带一个局部转移函数 f_v : State_v → 2^V（将 v 的局部状态映射到需要触发的下游顶点集），则全局执行轨迹 T(G) 等价于所有 f_v 在状态空间上的最小不动点。

**差距分析：**

| 维度 | 命题声称 | 当前实现 | 问题 |
| --- | --- | --- | --- |
| 转移函数的主体 | 每个节点 v 有一个 f_v | 每条边有一个 `on_event`，一维遍历 | 代码中找不到"节点 → 它的转移函数"的映射 |
| 检索范围 | f_v 只访问 State_v（局部信息） | 引擎遍历 `scheduler.edges` 全量 | 每次节点完成都要 `O(|E|)` 扫描 |
| 抽象层级 | f_v: State_v → 2^V 是高层语义 | 边是工程单元，粒度更细 | 命题的数学结构不直接投影到代码 |

### 1.2 性能问题叠加

| 场景 | 图规模 | 每次事件触发的边扫描量 |
| --- | --- | --- |
| 小图 | 10 节点, 30 边 | 30 条——可忽略 |
| 中等 | 100 节点, 400 边 | 400 条——轻微 |
| 大图 | 500 节点, 2000 边 | 2000 条——显著 |
| 高频迭代 | 环状子图, 500 次迭代 × 2000 边 | 1,000,000 次扫描——不可忽略 |

---

## 二、NodeTransfer 定义

### 2.1 核心结构

```rust
/// 节点 v 的局部转移函数 f_v。
///
/// 对应局部闭包定理中的 f_v: State_v → 2^V。
/// 每个节点恰好有一个 NodeTransfer，聚合了从该节点出发的所有出边。
pub struct NodeTransfer {
    /// 所属节点的索引
    pub from: NodeIndex,

    /// 出边索引列表（指向 Scheduler.edges[] 中的位置）。
    /// 每条出边已经按 from_nodes 预过滤好——所有 edge.from_nodes 都包含 from。
    pub out_edge_indices: Vec<usize>,
}
```

### 2.2 为什么是索引而不是 &Edge

- 索引是 `Copy`，不需要生命周期标注
- Scheduler 持有 `Vec<EdgeDef>` 的所有权，索引不会产生借用冲突
- 索引访问是 `scheduler.edges[idx]`，与遍历所有边相比只多一次间接寻址
- 边缘状态也在 `Vec<EdgeState>` 中通过同一索引访问

### 2.3 不可变性保证

`NodeTransfer` 在 Builder 阶段构造，运行时**只读**。`from` 和 `out_edge_indices` 在整个运行时生命周期中不变。

### 2.4 结构位置

```rust
pub struct Scheduler {
    states: HashMap<NodeIndex, NodeState>,
    counters: HashMap<NodeIndex, NodeCounters>,
    params: HashMap<NodeIndex, NodeParams>,

    // 边（定义 + 运行时状态分离）
    edges: Vec<EdgeDef>,              // 边定义——不可变
    edge_states: Vec<EdgeState>,      // 边运行时状态——与 edges 等长一一对应

    // 节点转移函数索引
    transfers: HashMap<NodeIndex, NodeTransfer>,

    ready_queue: VecDeque<NodeIndex>,
}
```

---

## 三、Builder 变更：构建 NodeTransfer

### 3.1 Builder 现有流程

Builder 的流程（ARCHITECTURE.md §3.2）不变，新增步骤 5：

1. 为每个 `NodeDef` 创建图节点（`NodeData`）
2. 将 `PredecessorDef` 按 `(to, trigger, event, exit_reason, threshold)` 分组聚合，每组生成一条边实例
3. 对每个节点的 Failed 和 Timeout，添加默认自环边（threshold=1），即重试
4. 识别入口节点
5. **构建 NodeTransfer 索引**：遍历所有边，按 `from_nodes` 分组聚合，为每个节点生成其局部转移函数

### 3.2 build_transfers 方法

```rust
impl Builder {
    fn build_transfers(
        edges: &[EdgeDef],
        graph: &DiGraph<NodeData, ()>,
    ) -> Result<HashMap<NodeIndex, NodeTransfer>, BuildError> {
        let mut transfers: HashMap<NodeIndex, Vec<usize>> = HashMap::new();

        for (idx, edge) in edges.iter().enumerate() {
            for &from in &edge.from_nodes {
                if graph.node_weight(from).is_none() {
                    return Err(BuildError::InvalidNodeIndex(from));
                }
                transfers.entry(from).or_default().push(idx);
            }
        }

        // 为没有出边的节点创建空的 NodeTransfer
        for node in graph.node_indices() {
            transfers.entry(node).or_default();
        }

        Ok(transfers
            .into_iter()
            .map(|(from, indices)| NodeTransfer { from, out_edge_indices: indices })
            .collect())
    }
}
```

### 3.3 BuildResult 变更

```rust
pub struct BuildResult {
    pub graph: DiGraph<NodeData, ()>,
    pub index_map: HashMap<String, NodeIndex>,
    pub edges: Vec<EdgeDef>,
    pub transfers: HashMap<NodeIndex, NodeTransfer>,  // [新增]
    pub node_params: HashMap<NodeIndex, NodeParams>,
    pub entry_nodes: Vec<NodeIndex>,
}
```

### 3.4 新增的不变量

| 不变量 | 验证方式 | 违反后果 |
| --- | --- | --- |
| **Transfer coverage** | 图中每个节点在 `transfers` 中都有条目 | 运行时 `transfers[node]` panic |
| **Index validity** | 所有 `out_edge_indices` 在 `edges` 范围内 | index out of bounds |
| **From consistency** | `transfers[n]` 中每条边的 `from_nodes` 包含 n | 触发了不属于自己的边 |
| **No orphan edges** | 每条边至少出现在一个 transfer 中 | 边被忽略 |

---

## 四、事件循环变更：从遍历所有边到按节点索引

### 4.1 变更要点

| 差异 | 旧版 | 新版 |
| --- | --- | --- |
| 检索范围 | 遍历 `scheduler.edges` 全量 | 通过 `transfers[node]` 索引取出边 |
| 条件匹配 | 遍历后做 `from_nodes().contains(&node)` | 不需要——Builder 已经预匹配好了 |
| 边状态管理 | Edge trait 内部 `&mut self` | Scheduler 统一管理 `edge_states` |
| 边类型 | `Vec<Box<dyn Edge>>` + trait + vtable | `Vec<EdgeDef>` + 字段直接访问 |
| 复杂度 | `O(|E|)` 每次节点完成 | `O(out_degree(v))` 每次节点完成 |

### 4.2 边退化为纯数据（EdgeDef）

结合 All/Any 完备性结论（DESIGN_PHILOSOPHY.md §〇 推论），边的最终形态确定为纯数据结构，无 trait、无多态：

```rust
/// 条件组合策略。All/Any 是完备的原语——不需要扩展第三种策略。
pub enum Strategy {
    All,
    Any,
}

/// 边定义——纯数据，运行时只读。
pub struct EdgeDef {
    pub from_nodes: Vec<NodeIndex>,
    pub to: NodeIndex,
    pub event_type: EventType,
    pub exit_reason: Option<String>,
    pub threshold: u64,
    pub strategy: Strategy,
}

/// 边的运行时状态——由 Scheduler 管理，与 edges 等长一一对应。
#[derive(Default)]
pub struct EdgeState {
    pub triggered: bool,
    pub event_count: u64,
    pub received: HashSet<NodeIndex>,  // 仅 strategy == All 时使用
}
```

**为什么不需要 trait 和多态**：

1. **All/Any 是完备的**（DESIGN_PHILOSOPHY.md §〇 推论）：任意布尔逻辑 `∧` / `∨` 表达式都可以用 All/Any + 图拓扑等价实现，不需要扩展第三种策略
2. **边只有数据，没有行为**：所有判定逻辑在 Scheduler::handle_event 中统一管理，不通过 vtable dispatch
3. **`Vec<EdgeDef>` 比 `Vec<Box<dyn Edge>>` 更简单**：没有堆分配，没有虚函数调用，没有 `Send + Sync` 的隐式约束

**好处**：
- 判定逻辑全部在 `handle_event` 一个函数中，代码结构直接对应数学结构 F_v
- 状态突变集中在 Scheduler 中，便于审计和测试
- 消除堆分配和 vtable 开销

---

## 五、Scheduler 变更：持有 NodeTransfer

### 5.1 完整的 Scheduler 结构

```rust
pub struct Scheduler {
    states: HashMap<NodeIndex, NodeState>,
    counters: HashMap<NodeIndex, NodeCounters>,
    params: HashMap<NodeIndex, NodeParams>,

    edges: Vec<EdgeDef>,
    edge_states: Vec<EdgeState>,

    transfers: HashMap<NodeIndex, NodeTransfer>,

    ready_queue: VecDeque<NodeIndex>,
}

impl Scheduler {
    pub fn new(
        params: HashMap<NodeIndex, NodeParams>,
        edges: Vec<EdgeDef>,
        edge_states: Vec<EdgeState>,
        transfers: HashMap<NodeIndex, NodeTransfer>,
        entries: &[NodeIndex],
    ) -> Self { /* ... */ }

    /// 事件处理——使用 NodeTransfer 索引 + EdgeDef 字段直接访问
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

            if state.triggered { continue; }
            if edge.event_type != event { continue; }
            if let Some(reason) = &edge.exit_reason {
                if exit_reason != Some(reason.as_str()) { continue; }
            }

            // All 策略：先到齐再计数
            if matches!(edge.strategy, Strategy::All) {
                state.received.insert(node);
                if state.received.len() < edge.from_nodes.len() { continue; }
            }

            state.event_count += 1;
            if state.event_count >= edge.threshold {
                state.triggered = true;
                ready.push(edge.to);
            }
        }

        ready
    }
}
```

### 5.2 事件循环中使用

```rust
let ready_nodes = scheduler.handle_event(node, event, exit_reason);
for target in ready_nodes {
    send(NodeReady(target));
}
```

---

## 六、不变性 & 正确性证明

### 6.1 构造期不变量

```rust
impl BuildResult {
    fn invariants_hold(&self) -> bool {
        let n = self.graph.node_count();
        let edge_count = self.edges.len();

        self.graph.node_indices().all(|i| self.transfers.contains_key(&i))
            && self.transfers.values().all(|t| {
                t.out_edge_indices.iter().all(|&idx| idx < edge_count)
            })
            && self.transfers.iter().all(|(&node, t)| {
                t.out_edge_indices.iter().all(|&idx| {
                    self.edges[idx].from_nodes.contains(&node)
                })
            })
            && (0..edge_count).all(|idx| {
                self.transfers.values().any(|t| t.out_edge_indices.contains(&idx))
            })
            && self.entry_nodes.iter().all(|i| i.index() < n)
            && self.params.len() == n
    }
}
```

### 6.2 运行时不变性

```
1. events[node][event]++ 后，transfers[node] 一定存在（构造期保证 coverage）
2. transfers[node].out_edge_indices 中所有索引在 edges 范围内
3. edge_states 与 edges 等长，同一索引访问语义正确
```

### 6.3 与旧版的行为等价性

**引理**：对于任意图 G 和任意事件序列 S，旧版和本设计产生完全相同的 `NodeReady` 序列。

证明概要：
1. **覆盖相同**：旧版遍历 `scheduler.edges` 通过 `from_nodes().contains(&node)` 筛选。新版通过 `transfers[node]` 直接访问——后者是前者在 Builder 阶段的预计算。
2. **判定相同**：旧版 AllEdge::on_event 与新版 All 分支的条件序列和突变动作完全一致（triggered、event_type、received、event_count、threshold）。
3. **归纳**：假设前 k-1 个事件后两个系统状态一致，第 k 个事件后仍然一致。

---

## 七、与局部闭包定理的对应关系

### 7.1 映射表

| 定理符号 | 工程设计 |
| --- | --- |
| G = (V, E) | `graph: DiGraph<NodeData, ()>` + `edges: Vec<EdgeDef>` |
| v ∈ V | `NodeIndex` |
| State_v | `counters[v]: NodeCounters { complete, failed, timeout }` |
| **f_v: State_v → 2^V** | **`transfers[v]: NodeTransfer { out_edge_indices }`** |
| f_v(State_v) | `handle_event(v, event, exit_reason) → Vec<NodeIndex>` |
| 2^V（幂集） | `Vec<NodeIndex>`（触发下游列表） |
| T(G)（全局执行轨迹） | 事件循环产生的 NodeReady 序列 |
| 最小不动点 | 所有 event_count 达到 threshold 后停止 |

### 7.2 形式化的 f_v

```
f_v(event, exit_reason) = {
    edge.to | edge ∈ transfers[v].out_edge_indices
              ∧ edge.event_type = event
              ∧ (edge.exit_reason = None ∨ edge.exit_reason = exit_reason)
              ∧ (edge.strategy = Any
                  ∨ (edge.strategy = All
                      ∧ edge_states[idx].received ⊇ edge.from_nodes))
              ∧ edge_states[idx].event_count + 1 ≥ edge.threshold
}
```

---

## 八、性能影响

| 操作 | 旧版 | 新版 |
| --- | --- | --- |
| 查找出边 | `O(|E|)` 遍历 | `O(1)` HashMap 查找 |
| 条件匹配 | `O(|E|)` 线性扫描 | `O(out_degree(v))` 索引遍历 |
| 状态突变 | `O(1)` on_event 内部 | `O(1)` edge_states[idx] |

对于 500 节点、2000 边的图，`build_transfers` 额外占用约 32KB 内存，运行时每次事件触发从 2000 次检查降为 1-10 次检查。

---

> **哲学总结：** NodeTransfer + EdgeDef + 去多态之后，代码结构和数学结构不再有隔阂——"每个节点自己带一个局部转移函数"在代码中不是隐喻，是字面意思。
>
> **当前代码状态**：上述设计已全部实现。`Scheduler` 包含 `transfers` 索引，`handle_event` 通过 `NodeTransfer.out_edge_indices` 索引遍历出边，复杂度 `O(out_degree(v))`。
