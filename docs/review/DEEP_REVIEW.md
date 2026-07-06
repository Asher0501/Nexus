# Nexus 架构深度审视报告

> 审查日期: 2026-07-05（第五轮——更新追踪 + 剩余问题精检）
> 审视视角: **排除已解决项，聚焦剩余真实问题**。
> 项目状态: **编码阶段，v1 已实现（90 测试通过，设计文档与代码一致）**

---

## 目录

- [演进追踪——本轮审查前的重要改动](#演进追踪本轮审查前的重要改动)
- [已闭环的问题清单](#已闭环的问题清单)
- [剩余真实问题](#剩余真实问题)
  - [问题 A：声明完备性原则的理论正确性与工程实践之间的不可验证性](#问题-a声明完备性原则的理论正确性与工程实践之间的不可验证性)
  - [问题 B：event_count 与 retry_count 的重叠计数——Failed/Timeout 被计两次](#问题-bevent_count-与-retry_count-的重叠计数failedtimeout-被计两次)
  - [问题 C：NodeData/NodeParams 中的 timeout_secs 未同步为 process_timeout_secs](#问题-cnodedatanodeparams-中的-timeout_secs-未同步为-process_timeout_secs)
  - [问题 D：输入侧（inputs）缺少引用验证——节点定义了 inputs 但源节点可能不存在](#问题-d输入侧-inputs-缺少引用验证节点定义了-inputs-但源节点可能不存在)
  - [问题 E：三个"完成"概念的语义重叠](#问题-e三个完成概念的语义重叠)
  - [问题 F：等价性定理中的事件序列一致性在并发下没有保证](#问题-f等价性定理中的事件序列一致性在并发下没有保证)
- [全局状态一览](#全局状态一览)
- [最终建议](#最终建议)

---

## 演进追踪——本轮审查前的重要改动

基于与上一轮审查时的文档快照对比，以下改动已实施：

| 改动 | 位置 | 说明 |
|------|------|------|
| `NodeDef.process_timeout_secs` 改名 | ARCHITECTURE.md §2 | `timeout_secs` → `process_timeout_secs` |
| `NodeDef.max_retries` 新增字段 | ARCHITECTURE.md §2 | 节点级重试配置，继承全局默认值 3 |
| Builder 步骤 3 移除"添加默认自环边" | ARCHITECTURE.md §3.2 | 重试不再依赖自环边 |
| 事件循环增加重试检查 | ARCHITECTURE.md §5.2 | Failed/Timeout 后先检查 retry_count，不超过上限才触发边 |
| RuntimeState 新增 `retry_counts` | ARCHITECTURE.md §5.4 | 每节点重试计数 |
| 重试机制完整文档 | ARCHITECTURE.md §5.4（新增子节） | 含重试配置表、重试与自环边的区别表 |
| Issue 012 创建 | docs/issues/ | 记录重试与自环边分离的设计决策 |
| §5.2 新增 timed_out 与 exit_code 正交性说明 | ARCHITECTURE.md §5.2 第 398-400 行 | "timed_out 与 exit_code 是正交的" |

### 特别关注：timed_out 与 exit_code 正交性说明

第 398-400 行是一个重要的新增声明：

> "timed_out 与 exit_code 是正交的：timed_out=true 表示引擎强杀，此时 exit_code 是 kill 信号（如 -9），不是节点自愿退出的码。timed_out 让用户能区分'节点自己报错'和'引擎超时强杀'。"

这个声明直接承认了我之前在第 3 轮审查中指出的"事件类型与 exit_code 之间信息不对称"问题（问题 #11），并用保留两个正交字段而不是压缩为单一事件类型的方式解决。

**评价**：这是正确的决策。保留正交信息比扩展事件枚举更好。

---

## 已闭环的问题清单

以下问题在本次更新后已不再构成有效问题：

| 原问题 | 来源 | 闭环原因 |
|--------|------|---------|
| **自环边相关**（重试语义、triggered 复位、无限重试） | 第 2/3 轮 | ✅ 重试已从自环边分离为 Scheduler 独立计数。自环边恢复为纯计数边 |
| **"引擎不解析"矛盾** | 第 2 轮 | ✅ DESIGN_PHILOSOPHY.md 更新为"结构化组装" |
| **终止条件** | 第 2 轮 | ✅ `running_count==0 && queue.is_empty()` |
| **DataRouter 语义** | 第 2 轮 | ✅ snapshot semantics 文档化 |
| **exit_reason 协议** | 第 2 轮 | ✅ NODE_PROTOCOL 更新 |
| **事件信息不对称（#11）** | 第 3 轮 | ✅ §5.2 第 398-400 行明确 timed_out 与 exit_code 正交 |
| **时间超时模型改名** | 第 2 轮 | ✅ `process_timeout_secs` 在 `NodeDef` 中已完成 |
| **重试机制架构定义缺失** | 第 4 轮 | ✅ ARCHITECTURE.md 新增完整重试机制文档 |
| **自环边/重试分离** | 第 4 轮 | ✅ Issue 012 已创建，ARCHITECTURE.md 已更新 |

---

## 剩余真实问题

经过五轮审查，项目已解决大部分问题。以下是 **仍然真实存在** 的剩余问题，按严重度排列。

---

### 问题 A：声明完备性原则的理论正确性与工程实践之间的不可验证性

**严重度**: 🟡 中  
**影响范围**: 运行时可靠性  
**状态**: **已解决（设计决策更新）**

#### 问题

DESIGN_PHILOSOPHY.md 原来说"运行时遇到声明未覆盖的行为 → 引擎立即崩溃退出"，但在字符串类型的 exit_reason 面前，"声明完备性"是不可验证的——exit_reason 值域无限，Validator 无法检查所有可能的运行时值。

#### 解决方案

已更新 DESIGN_PHILOSOPHY.md 中的声明完备性原则，将"崩溃退出"改为"记录 warning 日志并跳过不匹配的出边"。核心原因：字符串类型的无限值域使得崩溃不能解决完备性问题，可诊断的日志比不可恢复的崩溃更有实际价值。Validator 在运行前通过 Warning 检查声明完备性，运行时遇到的未覆盖 exit_reason 通过日志排查。

---

### 问题 B：event_count 与 retry_count 的重叠计数——Failed/Timeout 被计两次

**严重度**: 🟡 中  
**影响范围**: 运行时语义正确性  
**状态**: **已解决（文档已补充）**

#### 问题

counters[Failed/Timeout] 只记录超出重试次数的事件，被重试消耗掉的事件不计入。这在 ARCHITECTURE.md 的事件循环说明中没有说明。

#### 解决方案

已在 ARCHITECTURE.md §5.2 的事件循环代码中增加了注释说明：

> 注意：counters[node][Failed|Timeout] 只记录超出重试次数的事件，被重试消耗掉的事件不计入。如果需要监控节点实际失败/超时次数，应使用 scheduler.state.retry_counts[node]。

---

### 问题 C：NodeData/NodeParams 中的 timeout_secs 未同步为 process_timeout_secs

**严重度**: 🟢 低（文档不一致）  
**影响范围**: 文档正确性  
**状态**: **已解决**

#### 问题

`NodeData.timeout_secs` 和 `NodeParams.timeout_secs` 在 ARCHITECTURE.md 中未同步改为 `process_timeout_secs`。

#### 解决方案

已通过 grep 确认 ARCHITECTURE.md 中所有位置（NodeData、NodeParams、JSON 示例、测试表）均已使用 `process_timeout_secs`。

---

### 问题 D：输入侧（inputs）缺少引用验证——节点定义了 inputs 但源节点可能不存在

**严重度**: 🟡 中  
**影响范围**: 运行时健壮性  
**状态**: **已解决**

#### 问题

节点 inputs 可能引用不存在的节点 ID，运行时 DataRouter 无法解析。

#### 解决方案

已在 Validator 中实现 `InputSourceNotFound`（引用的节点 ID 不存在）和 `InputSourceUnreachable`（引用的节点从入口不可达）两项检查。ARCHITECTURE.md §3.3 的验证项表已补充这两项。

---

### 问题 E：三个"完成"概念的语义重叠

**严重度**: 🟢 低  
**影响范围**: 概念清晰度  
**状态**: **已解决**

#### 问题

文档中"完成"一词被混用于"节点完成"、"工作流完成"、"边完成"三个不同的概念。

#### 解决方案

已在 ARCHITECTURE.md 开头增加**术语表**，明确定义：
- **节点完成** = 子进程退出产生 `NodeCompleted` 事件
- **边触发** = 边达到 threshold，`triggered = true`
- **工作流收敛** = `running_count == 0 && ready_queue.is_empty()`

文档中"完成"单独出现时默认指"节点完成"。

---

### 问题 F：等价性定理中的事件序列一致性在并发下没有保证

**严重度**: 🟡 中（理论层面）  
**影响范围**: 确定性保证  
**状态**: **已解决（结论修正 + 文档优化）**

#### 问题

审查认为等价性证明中"事件序列由状态迭代自然确定"的声明只对单线程成立。但实际上 F_v 的签名中不包含时间参数，交换互不相交的 F_v 的调用顺序不影响最终状态——等价性在并发下也成立。

原文档的表述（"事件序列是由状态迭代自然确定的"）不够清晰，容易引起误解。

#### 解决方案

已更新 DESIGN_PHILOSOPHY.md 中的相关表述，明确说明 F_v 不含时、交换互不相交的 F_v 的顺序不影响结果。

---

## 全局状态一览

### 文档演进状态

| 文档 | 状态 | 备注 |
|------|------|------|
| ARCHITECTURE.md | ✅ 已更新（740 行） | 重试机制、process_timeout、timed_out 正交性 |
| DESIGN_PHILOSOPHY.md | ✅ 已更新（669 行） | 声明完备性、等价性定理、形式化定义 |
| NODE_PROTOCOL.md | ✅ 已更新（208 行） | `__nexus_exit_reason` 协议 |
| NODE_TRANSFER.md | 未变化 | 与当前实现一致 |
| Issues 目录 | 12 个 | 001-012 |
| DEEP_REVIEW.md | 本文 | 五轮审查汇总 |

### 三轮审查后的问题收敛情况

| 轮次 | 发现数 | 已解决 | 待解决 | 备注 |
|------|-------|--------|--------|------|
| 第 1 轮（ARCHITECTURE_REVIEW） | 10 | 9 | 1* | 未解决的涉及 Cargo.toml 构建基础设施 |
| 第 2 轮（DEEP_REVIEW 语言无关） | 9 | 6 | 3 | 未解决：声明完备性、#D、#F |
| 第 3 轮（深层 + ISSUE 追踪） | 3 | 2 | 1 | 未解决：事件序列 #F |
| 第 4 轮（第一性原理） | 5 | 4+ | 0 | 重试机制已在本次更新中解决 |
| **第 5 轮（本次）** | — | — | **6 个剩余** | 见上 A-F |

> *ARCHITECTURE_REVIEW 的 Cargo.toml 问题（workspace 根、依赖 mismatch、lint 配置）属于构建基础设施，不是设计问题，在此表中不计入待解决。

### 最终剩余问题——全部已解决

| 问题 | 严重度 | 解决方式 |
|------|--------|---------|
| **A** 声明完备性崩溃诊断 | 🟡 中 | ✅ 设计决策更新：无限值域下崩溃不可行，改为 warning 日志 + 跳过 |
| **B** event_count vs retry_count 计数说明 | 🟡 中 | ✅ 已补充注释说明 counters 只记录超出重试次数的事件 |
| **C** timeout_secs 改名同步 | 🟢 低 | ✅ 已确认 ARCHITECTURE.md 全部使用 process_timeout_secs |
| **D** InputSource 验证 | 🟡 中 | ✅ InputSourceNotFound + InputSourceUnreachable 已在 Validator 中实现 |
| **E** "完成"概念语义重叠 | 🟢 低 | ✅ 已增加术语表（节点完成 / 边触发 / 工作流收敛） |
| **F** 等价性证明的并发假设 | 🟡 中 | ✅ 结论修正：F_v 不含时，交换互不相交的 F_v 调用顺序不影响结果，文档已优化表述 |

---

## 最终建议

### 建议一：编码前只需修复 C（3 行改名）

如果现在就要开始编码，**唯一需要在编码前处理的文档不一致**是问题 C——`NodeData.timeout_secs` 和 `NodeParams.timeout_secs` 的改名。3 行文档修改，不涉及任何设计决策。**当前状态：已确认 ARCHITECTURE.md 全部使用 process_timeout_secs。**

### 建议二：编码后仍需处理的 5 个问题

这 5 个问题已在编码阶段全部解决。

### 建议三：编码策略建议

```
编码路径（从设计到实现）：
  1. 工作流模型（WorkflowDef / NodeDef / PredecessorDef）  ← 数据层
  2. Builder + GraphDef + 不变量验证                         ← 语法层
  3. Validator                                                ← 验证层
  4. EdgeDef + EdgeState + Strategy                           ← 边模型
  5. Scheduler + RuntimeState + handle_event                  ← 调度层
  6. DataRouter                                               ← 数据路由
  7. Event Loop                                               ← 主循环
  8. NodeExecutor / SubprocessExecutor                        ← 执行层
  9. CLI                                                       ← 入口
  10. 模式测试                                                ← 验证
```

前 5 步不涉及子进程/异步 I/O，可以纯单元测试驱动。第 6/7 步开始需要 token 集成测试。

---

> **本报告为第五轮审查总结。所有指出的 6 个剩余问题（A-F）已在编码阶段全部解决。报告中描述的问题、分析和解决方案保留为架构决策记录。当前项目状态：代码实现完整（90 测试通过）、设计文档与实现一致、审查意见已全部闭环。**
