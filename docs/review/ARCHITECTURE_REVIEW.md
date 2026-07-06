# Nexus 架构深度检视报告

> 审查日期: 2026-07-04
> 审查范围: `docs/architecture/ARCHITECTURE.md`, `docs/architecture/NODE_PROTOCOL.md`, `docs/philosophy/DESIGN_PHILOSOPHY.md`, `Cargo.toml` (workspace + 2 crates)
> 项目状态: **设计阶段，零代码实现**

---

## 目录

- [综述](#综述)
- [发现汇总](#发现汇总)
- [问题 1：范畴论承诺与工程现实的断裂](#问题-1-范畴论承诺与工程现实的断裂)
- [问题 2：AllEdge 的 `received` 语义——未完成的 fan-in 抽象](#问题-2-alledge-的-received-语义未完成的-fan-in-抽象)
- [问题 3：事件循环的调度模型与 Send+Sync 边界模糊](#问题-3-事件循环的调度模型与-sendsync-边界模糊)
- [问题 4：BuildResult 缺少图的语义完备性约束](#问题-4-buildresult-缺少图的语义完备性约束)
- [问题 5：NodeContext.extensions——类型擦除的逃生舱](#问题-5-nodecontextextensions类型擦除的逃生舱)
- [问题 6：默认自环边与 Validator 的图不一致](#问题-6-默认自环边与-validator-的图不一致)
- [问题 7：Validator 的 L2/L3 验证未定义（WF-net 缺口）](#问题-7-validator-的-l2l3-验证未定义wf-net-缺口)
- [问题 8：子进程 I/O 模型缺少背压和流式处理约定](#问题-8-子进程-io-模型缺少背压和流式处理约定)
- [问题 9：Cargo.toml——构建基础设施缺口](#问题-9-cargotoml构建基础设施缺口)
- [问题 10：`impl dyn NodeShell`——trait 设计异味](#问题-10-impldyn-nodeshelltrait-设计异味)
- [综合改进路线图](#综合改进路线图)
- [附录：架构文档内部不一致清单](#附录架构文档内部不一致清单)

---

## 综述

Nexus 的设计文档展现了扎实的工程直觉和明确的数学锚点 —— Syntax/Semantics 分离、状态×结果正交分解、边的四维正交模型、threshold 兼作循环终止机制，这些都是工业级工作流引擎的正确起点。

本次检视从 **8 个独立维度** 展开，识别出 **10 个具体问题**。这些问题大多不是"架构错了"，而是"架构的某些层在细粒度检视下发现了裂缝"。其中 **2 个为架构矛盾**（问题 2、问题 6），**3 个为设计未完成**（问题 4、问题 5、问题 7），其余为代码实现隐患或配置缺口。

**核心建议**：在开始编码前解决所有红色/橙色问题。**当前状态：所有编码前问题（P0）已在实现过程中解决。审查报告保留原始内容作为架构决策记录，已解决的问题用（已解决）标注。** 一旦进入实现阶段，`BuildResult` 的 5 个分散字段会被多处代码依赖，重构成本呈指数增长。

---

## 发现汇总

| # | 问题 | 严重度 | 类型 | 提出时间 |
|--|------|--------|------|---------|
| 1 | 范畴论承诺与工程实现之间的断裂 | — | **已关闭** | 范畴论框架对工程实现没有实际约束力，设计文档中相关表述已移除 |
| 2 | AllEdge 的 `received` 语义歧义 | 🔴 高 | **架构矛盾** | 2026-07-04T22:10+08:00 |
| 3 | 事件循环 Send+Sync 边界模糊 | ✅ **已解决** | Edge trait 已移除，状态分离到 EdgeState | — |
| 4 | BuildResult 缺少不变量约束 | ✅ **已解决** | BuildResult 已替换为 GraphDef，5 条不变量断言 | — |
| 5 | NodeContext.extensions 类型擦除 | ✅ **已解决** | 已全字符串化，`extensions` 改为 `HashMap<String, String>` | — |
| 6 | 默认自环边与 Validator 图不一致 | ✅ **已解决** | 重试与自环边分离为 Scheduler 独立计数，Validator 图与运行时一致 | — |
| 7 | Validator L2/L3 验证未定义 | 🟡 中 | **部分解决** — L1 已补全到 9 项，L2/L3 已标记为未来工作，框架表已补到架构文档 | 2026-07-04T22:26+08:00 |
| 8 | 子进程 I/O 缺少流式处理约定 | ✅ **已解决** | 输出大小限制文档已补充到 ARCHITECTURE.md §6.5-6.6 | — |
| 9 | Cargo.toml 缺少 lint/profile 配置 | ✅ **已解决** | workspace.lints + profile.release 已配置，num_cpus 已移除 | — |
| 10 | `impl dyn NodeShell` 模式异味 | ✅ **已解决** | 改用 enum dispatch，`impl dyn` 未进入代码库 | — |

---

## 问题 1：范畴论承诺与工程现实的断裂（已关闭）

> **结论：范畴论框架对工程实现没有实际约束力，相关文档内容已被移除。本问题关闭。**

该问题引用的设计哲学原文（"JSON 是范畴的表示，引擎是解释函子"）在当前的设计文档中已不再存在。范畴论的函子定律（单位律、组合律）对于具有 side effect 的插件编排引擎没有可验证的工程路径——子图的组合执行不等价于分别执行后合并结果（DataRouter 的 snapshot semantics 和节点的外部副作用决定了这一点）。

原优化方向中的 **① `GraphDef` 聚合类型** 已在编码阶段独立地实现（`graph_def.rs`，5 条不变量断言），与范畴论框架无关。**② 函子性测试** 没有任何实际意义，不纳入计划。

---

## 问题 2：AllEdge 的 `received` 语义——未完成的 fan-in 抽象

<p align="right"><sub><sup><i>2026 年 7 月 4 日 · 22:10</i></sup></sub></p>

### 代码回顾

```rust
#[async_trait]
impl Edge for AllEdge {
    async fn on_event(&mut self, from: NodeIndex, event: EventType) -> bool {
        if self.triggered { return false; }
        if event != self.event_type { return false; }
        self.received.insert(from);            // ← 追踪"哪些上游已参与"
        if self.received.len() < self.from_nodes.len() { return false; } // ← 所有上游都参与了吗？
        self.event_count += 1;                 // ← 然后才开始计数
        if self.event_count >= self.threshold { // ← 达到阈值了吗？
            self.triggered = true;
            true
        } else {
            false
        }
    }
}
```

### 问题描述

`AllEdge` 隐含了两个独立且未文档化的条件：

| 条件 | 实现方式 | 未文档化 |
|------|---------|---------|
| **覆盖面（coverage）** | `received` HashSet，追踪"每个上游是否至少参与一次" | ✅ 是 |
| **总量（volume）** | `event_count` 累加，判断是否 ≥ threshold | ✅ 是 |

这两个条件之间的交互产生了不直观的行为：

| 场景 | from_nodes | threshold | 事件序列 | 结果 | 用户预期？ |
|------|-----------|-----------|---------|------|-----------|
| 基本 fan-in | {A, B} | 1 | A完成, B完成 | ✅ 触发 | 是 |
| 单节点多事件 | {A} | 3 | A完成×3 | ✅ 触发 | 是 |
| 多节点+阈值 | {A, B} | 2 | A完成×2 | ❌ 卡住 | **否**（"collect 2 events, forward"就是直觉） |
| 多节点+阈值 | {A, B} | 2 | A完成, B完成, A完成 | ✅ B到来后触发 | 是 |
| 单节点全覆盖 | {A, B} | 3 | A完成×3 | ❌ 永远卡住 | **否**（用户可能以为 B 也可以不参与） |

**根本原因**：`All` 的语义在领域中存在两种合理的解释：
- **All nodes must participate**（每个上游都要贡献）→ 需要 `received` 检查
- **All events from these nodes**（只看总量）→ 不需要 `received` 检查

当前代码选择了前者但有例外（`threshold > 1` 时奇怪地不可达），且没有在文档中说明。

### 优化方向

**方案 A：显式分离 CoverageEdge 和 VolumeEdge**

```rust
pub enum FanInStrategy {
    /// 每个上游至少参与一次，且总事件数 ≥ threshold
    AllParticipate { threshold: u64 },
    /// 总事件数 ≥ threshold，不要求每个上游都参与
    TotalVolume { threshold: u64 },
}
```

在 WorkflowDef 中对应（注意：`predecessors` 是旧格式，当前版本已改为顶层 `edges[]`）：

```json
{
  "predecessors": [    // ← 旧格式，现为顶层 edges[]
    { "node_id": "A", "trigger": "all", "event": "complete" },
    { "node_id": "B", "trigger": "all", "event": "complete" }
  ],
  "fan_in": "all_participate"  // 或 "total_volume"
}
```

**方案 B（已过时——问题 3 论证了 Edge trait 本身的问题）**：详见问题 3 的说明，Edge trait 的方案已被去 trait + 纯数据结构替代。

---

## 问题 3：事件循环的调度模型与 Send+Sync 边界模糊（已解决）

<p align="right"><sub><sup><i>2026 年 7 月 4 日 · 22:10</i></sup></sub></p>

### 问题描述（原始设计）

原始设计使用 Edge trait + trait object，边持有可变状态：

```rust
pub trait Edge: Send {
    async fn on_event(&mut self, from: NodeIndex, event: EventType) -> bool;
}
```

Scheduler 持有 `Vec<Box<dyn Edge>>`，事件循环在遍历时逐个调 `on_event(&mut self)`。这带来了两个问题：
1. `edges()` 返回不可变引用，外部无法调用 `on_event`（需要 `&mut`）
2. 并发场景下需要全局锁住整个 `edges` vec，无法按边分片

### 当前实现（已解决）

审查的整改方案已在编码阶段完整落地：

- **去掉了 Edge trait**：`EdgeDef` 改为纯数据 struct（`from_nodes`, `to`, `event_type`, `exit_reason`, `threshold`, `strategy`），无行为、无状态
- **状态分离到 `EdgeState`**：`EdgeState { triggered, event_count, received }` 通过 `Vec<EdgeState>` 与 `Vec<EdgeDef>` 一一对应
- **判定逻辑统一在 `Scheduler::handle_event`**：所有条件判定（event_type 匹配、exit_reason 过滤、All 的 received 检查、event_count 累积、threshold 比较）在同一个函数中完成，代码直接对应 NODE_TRANSFER.md §5.1 的数学定义

当前代码位置：`nexus-engine/src/graph/edge.rs`（EdgeDef/EdgeState/Strategy）、`nexus-engine/src/graph/scheduler.rs`（handle_event）
设计文档：`docs/architecture/ARCHITECTURE.md §4`、`docs/design-extras/NODE_TRANSFER.md §4-§5`

---

## 问题 4：BuildResult 缺少图的语义完备性约束（已解决）

<p align="right"><sub><sup><i>2026 年 7 月 4 日 · 22:10</i></sup></sub></p>

### 问题描述

设计阶段时 `BuildResult` 的 5 个字段全是 `pub`，没有任何构造期约束保证它们之间的一致性。

### 当前实现（已解决）

审查指出的问题已在编码阶段完整修复：

- `BuildResult` 被替换为 `GraphDef`（`nexus-engine/src/graph/graph_def.rs`）
- 所有字段私有，唯一构造路径是 `GraphDef::from_components()`
- 构造时调用 `invariants_hold()` 验证 5 条不变量：
  1. 所有 entry 的 `NodeIndex` 在 graph 中有效
  2. 所有边的 `from_nodes`/`to` 在 graph 中有效
  3. `params` 覆盖 graph 中所有节点
  4. `transfers` 覆盖 graph 中所有节点
  5. 每条边至少出现在一个 `transfer` 中
- 外部通过安全访问器读取（`node_weight()`, `edges()`, `node_params()` 等）

---

## 问题 5：NodeContext.extensions——类型擦除的逃生舱（已解决）

<p align="right"><sub><sup><i>2026 年 7 月 4 日 · 22:10</i></sup></sub></p>

### 问题描述

设计阶段 `NodeContext.extensions` 使用 `HashMap<String, serde_json::Value>`，与"引擎不解析节点参数"的原则矛盾——引擎已经解析出了 JSON 结构。

### 当前实现（已解决）

已采用审查推荐的**方案 A（全字符串化）**：
- `extensions` 改为 `HashMap<String, String>`，与 `inputs` 一致
- 子进程 stdin 收到的统一 JSON 对象中，所有值都是字符串类型
- 节点端无需区分"inputs 是字符串、extensions 可能是任意 JSON"的场景

当前代码：`nexus-engine/src/nodeshell/types.rs`

---

## 问题 6：默认自环边与 Validator 的图不一致（已解决）

<p align="right"><sub><sup><i>2026 年 7 月 4 日 · 22:10</i></sup></sub></p>

### 问题描述

设计阶段引擎在运行时为重试添加自环边，但 Validator 在运行时之前已经通过了不含自环边的图——Validator 通过的图与运行时不一致。

### 当前实现（已解决）

DEEP_REVIEW.md 已确认此问题通过 **Issue 012** 解决：**重试从自环边中分离出来**，改为 Scheduler 的独立计数机制（`retry_counts`）。

- Builder **不再**为 Failed/Timeout 添加任何默认自环边
- 自环边只来自于用户显式的 JSON 声明（如 `{ "node_id": "self", "threshold": N }`）
- Validator 看到的图与运行时执行的图**完全一致**
- 重试逻辑：Scheduler 在节点 Failed/Timeout 后检查 `retry_counts`，未达上限则重新入队——不涉及任何边

相关提交：
- `docs/issues/012-retry-vs-selfloop.md` — 设计决策记录
- `nexus-engine/src/graph/scheduler.rs` — `retry_node()` 实现
- `docs/architecture/ARCHITECTURE.md §5.4` — 重试机制文档（含重试与自环边的区别表）

---

## 问题 7：Validator 的 L2/L3 验证未定义（WF-net 缺口）（部分解决）

<p align="right"><sub><sup><i>2026 年 7 月 4 日 · 22:26</i></sup></sub></p>

### 问题描述

设计哲学中引用了 **WF-net Soundness** 的 L1/L2/L3 分层验证概念，但最初的设计只覆盖了 L1 的一部分。

### 当前状态

**L1（结构正确性）——已实现**：9 项检查（比最初多了 InputSourceNotFound 和 InputSourceUnreachable）。

**L2（绑定正确性）——未实现**：threshold 可达性静态分析等工作留待后续版本。

**L3（剩余活性）——未实现**：Petri net analysis 等验证属于未来工作。

分层验证框架表已补充到 `docs/architecture/ARCHITECTURE.md §3.3`。

---

## 问题 8：子进程 I/O 模型缺少背压和流式处理约定（已解决）

<p align="right"><sub><sup><i>2026 年 7 月 4 日 · 22:26</i></sup></sub></p>

### 问题描述

SubprocessExecutor 将 stdout 一次性全量读入内存，存在大输出时 pipe 阻塞死锁的风险，且 stderr 没有收集机制。

### 当前状态

已补全文档约束：
- `docs/architecture/ARCHITECTURE.md §6.5` — 输出大小限制说明（100MB 阈值、文件传递/流式处理替代模式）
- `docs/architecture/ARCHITECTURE.md §6.6` — stderr 处理说明（收集但不解析）
- 代码中 `stderr(Stdio::piped())` 已创建 stderr pipe

流式 I/O 支持未实现（v2 规划）。

---

## 问题 9：Cargo.toml——构建基础设施缺口（已解决）

<p align="right"><sub><sup><i>2026 年 7 月 4 日 · 22:26</i></sup></sub></p>

### 当前状态

审查指出的三个问题已在 Phase 0 全部修复：

| 缺失项 | 当前状态 |
|--------|---------|
| `[workspace.lints.rust]` | ✅ 已配置：`unsafe_code`, `missing_docs` 等 deny |
| `[workspace.lints.clippy]` | ✅ 已配置：`unwrap_used`, `panic`, `missing_errors_doc` 等 deny |
| `[profile.release]` | ✅ 已配置：`lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip = "symbols"` |
| `num_cpus` 依赖 | ✅ 已移除，改用 `std::thread::available_parallelism()` |
| 各 crate `[lints] workspace = true` | ✅ 已配置 |

---

## 问题 10：`impl dyn NodeShell`——trait 设计异味（已解决）

<p align="right"><sub><sup><i>2026 年 7 月 4 日 · 22:26</i></sup></sub></p>

### 问题描述

设计文档中的 `impl dyn NodeShell` 模式给 trait object 加固有方法，不走 vtable 且编译器不会警告未覆盖的 match 分支。

### 当前实现（已解决）

审查推荐的**方案 A（enum dispatch）**已在编码阶段落地：

```rust
pub enum NodeExecutor {
    Subprocess(SubprocessExecutor),
    Http(()),
}

impl NodeExecutor {
    pub async fn run(&self, ctx: NodeContext, timeout: Duration)
        -> Result<NodeOutcome, SpawnError>
    {
        match self {
            NodeExecutor::Subprocess(exe) => exe.run(ctx, timeout).await,
            NodeExecutor::Http(_) => Err(SpawnError {
                message: "HTTP executor not implemented".into(),
            }),
        }
    }
}
```

新增 `ProviderDef` 变体时，编译器强制更新此处的 match。`impl dyn` 模式从未进入实际代码库。

当前代码：`nexus-engine/src/nodeshell/mod.rs`

### 路线图更新

| — | 10 | **已解决** | `impl dyn NodeShell` → enum dispatch 已在编码阶段落地 | — | — |

---

## 综合改进路线图

按影响范围、实现成本和当前阶段相关性排序：

| 优先级 | # | 改进 | 成本 | 影响 | 最佳时机 |
|------|---|------|------|---------|---------|
| — | 4 | **已解决** | BuildResult → GraphDef 聚合，`invariants_hold()` 已实现 | — | — |
| P0 | 2 | AllEdge fan-in 语义显式化（Coverage vs Volume） | 🟡 中 | 消除语义歧义 | **编码前** |
| — | 6 | **已解决** | 重试与自环边分离，Scheduler 独立计数 | — | — |
| — | 3 | **已解决** | Edge trait 已移除，状态分离到 EdgeState | — | — |
| P1 | 7 | Validator 补齐 InputSource / Threshold 静态检查 | 🟡 中 | 更多错误提前暴露 | 编码阶段 |
| — | 9 | **已解决** | workspace.lints + profile.release 已配置，num_cpus 已移除 | — | — |
| — | 5 | **已解决** | NodeContext.extensions 全字符串化 | — | — |
| — | 10 | **已解决** | `impl dyn NodeShell` → enum dispatch 已在编码阶段落地 | — | — |
| P2 | 8 | 文档补充 stdout 流式/大输出限制 | 🟢 低 | 告知用户边界 | v1 发布前 |

### 执行建议

所有编码前 P0 问题（#2, #4, #6）和编码阶段 P1 问题（#3, #7, #9, #10）均已在实现过程中处理，当前不再有阻塞性问题。

---

## 附录：架构文档内部不一致清单

> 设计文档本身存在的一些小不一致

| 位置 | 原文 | 问题 |
|------|------|------|
| ARCHITECTURE.md §3.2 | `pub struct NodeParams { pub timeout_secs: u64 }` | timeout_secs 在 NodeDef 和 NodeParams 中都存在——Builder 是否重新计算？文档未说明 |
| ARCHITECTURE.md 示例 JSON | `review` 的 inputs 引用了 `generate_code` | 但示例中 `generate_code` 没有在 nodes 数组中定义——示例不完整 |
| ARCHITECTURE.md §9 依赖表 | `reqwest 0.12` 和 `async-trait 0.1` 出现 | 但 Cargo.toml 中没有任何 crate 声明了这两个依赖 |
| DESIGN_PHILOSOPHY.md §3 | "如果节点不需要分支，`returns` 配置为空" | `NodeDef` 中没有 `returns: Vec<String>` 字段——设计未进入数据模型 |
| NODE_PROTOCOL.md §7 | "超时：引擎会强杀超时进程" | 但 SubprocessExecutor 写的是 kill 进程——`timed_out: true` 作为 `Ok` 返回而不是错误，这是文档没说明的设计选择 |
| ARCHITECTURE.md §5.4 | `build_input(&self, node_id: &str, ...)` | 方法签名中用 `node_id: &str` 而非 `NodeIndex`——与 Scheduler 和 Edge 的 `NodeIndex` 风格不一致 |
