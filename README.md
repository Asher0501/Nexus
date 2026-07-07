# Nexus

> **有向图驱动的插件编排引擎** — 任何子进程都是节点，退出码决定完成，stdout 即结果。

---

Nexus 是一个低层级的工作流编排引擎。你将工作流定义为一张有向图：节点就是子进程，边就是触发条件。没有中心化的流程控制器——每个节点只带自己的计数器和出边规则，引擎反复问每个节点「你的条件满足了吗」，直到所有人都说「不满足了」。

```mermaid
graph LR
    A["JSON Workflow Def"] --> B["serde_json parse"]
    B --> C["WorkflowDef"]
    C --> D["DiGraphBuilder"]
    D --> E["GraphDef"]
    E --> F{"Validator"}
    F -->|pass| G["Runtime Engine"]
    F -->|fail| H["ValidationError"]
```

## Nexus vs LangGraph / 编排框架

编排框架通常分为两类：**图运行时**和**agent 框架**。Nexus 属于前者，并且做了与主流方向不同的设计取舍。

### Agent 框架的根本局限

LangGraph 和同类 agent 框架的核心抽象是 **「LLM 自主路由」**——框架维护一个状态图，每个节点由 LLM 决定下一步去哪。这在原型和 demo 阶段非常高效，但在生产部署中暴露出一系列结构性问题：

| 问题 | 表现 |
|------|------|
| **不可预测的执行路径** | LLM 每次调用可能选择不同的下游节点。同一个输入可能走完全不同的执行路径，测试覆盖无法保证。 |
| **拓扑路径不透明** | 多 agent 协作时，协作拓扑由 LLM 的即时决策动态生成。没有人在设计时定义过这张图——它是运行时涌现的。调试时你只能看到「这次走了哪条路」，不能断言「所有可能的路」。 |
| **难以用于稳定的业务生产** | 业务系统需要确定性的编排——审批流必须走 manager_approve，不走 auto_escalate；支付流水线必须先 validate 再 charge，不能反过来。当路由决策权交给 LLM 时，这些约束变成了 prompt 工程问题，而非架构保证。 |
| **复杂 skill 任务靠 LLM 自觉** | 工具调用顺序、上下文传递、错误恢复——全部依赖 LLM 在 prompt 中「理解该怎么做」。没有编译期检查，没有静态分析。如果 LLM 选错了工具，框架不会阻止，只会忠实地执行错误决定。 |

### Nexus 的选择：确定性图 + 子进程节点

Nexus 走另一条路：**执行拓扑在设计时声明，由引擎机械地执行，不受运行时 LLM 决策的影响。**

```
LangGraph:  LLM → 选边 → 执行节点 → LLM → 选边 → ...
Nexus:       声明式 JSON 图 → 引擎机械计数 → 触发下游 → ...
```

| 维度 | LangGraph / Agent 框架 | Nexus |
|------|----------------------|-------|
| 执行路径 | LLM 运行时决定 | 设计时声明，引擎机械执行 |
| 节点类型 | Python 函数 / LLM Call | **任何子进程**（Python、Shell、OpenCode、Claude Code） |
| 状态传递 | 共享 TypedDict，框架管理 | **stdout 即结果**，进程间通过 DataRouter 传递 |
| 路由决策 | LLM 选边 / 条件边函数 | **边触发算法**（事件类型 × exit_reason × All/Any × threshold） |
| 可测试性 | Mock LLM、限制工具集 | **编译期验证** + 确定性执行路径 |
| 并发控制 | 框架内部调度 | **Semaphore**，用户可见可配置 |
| 持久化 | Checkpointer 快照 | **Snapshot semantics**——不保证同轮，保证最新 |
| 节点语言 | Python / TypeScript | **任何语言**——pidgin 无关 |

### 什么时候用 Nexus

Nexus 不适合需要 LLM 自主探索、动态规划路径的场景（那是 agent 框架的领域）。Nexus 适合：

- **你预先知道工作流的拓扑结构**——审批流、CI/CD 流水线、数据处理管道、多步 AI 审查链
- **你需要编译期保证**——验证器在运行前检查图结构、可达性、声明完备性
- **你的节点是异构的**——Python 脚本、Shell 命令、OpenCode、Claude Code、自定义二进制，混排在一个工作流中
- **你需要稳定、可复现的生产行为**——同一个输入总是走同一条路径

## 为什么用 Nexus？

Nexus 提供的是基础设施，不是抽象——它不封装你的业务逻辑，也不猜测你的节点类型：

- **进程即节点** — 任何 stdin/stdout 的子进程都可以成为节点。Python、PowerShell、Rust、Node.js、OpenCode、Claude Code——不改引擎代码。
- **退出码即完成** — exit 0 = 完成，非 0 = 失败。这是判断进程结束的唯一方式。不依赖超时、不依赖心跳、不依赖最后一行 JSON。
- **逐行流式输出** — 每行 stdout 在节点运行时实时上报（`--verbose` 可见）。进程不需要等退出——中间结果实时可观测。
- **无节点类型耦合** — 没有 AI executor、没有 HTTP executor。所有节点统一为 `type: "subprocess"`。引擎不感知节点类型。
- **边驱动调度** — 边由四个正交维度定义：事件类型（Complete/Failed/Timeout）、返回值（exit_reason）、组合逻辑（All/Any）、阈值（threshold）。不需要第五种维度。
- **调度拓扑 ≠ 数据拓扑** — `edges` 决定谁完成后谁开始，`dataflows` 决定谁的数据传给谁。两张独立的图。

## 快速开始

```bash
# 验证工作流
nexus-cli run test_workflow.json --validate-only

# 运行一个工作流
nexus-cli run test_workflow.json

# 查看流式输出
nexus-cli run test_workflow.json --verbose

# 完成后输出节点状态
nexus-cli run test_workflow.json --dump-state
```

## 工作流定义

工作流由三部分组成：节点（nodes）、调度边（edges）、数据流（dataflows）。

```json
{
  "nodes": [
    {
      "id": "fetch_data",
      "providers": [{"type": "subprocess", "command": "python fetcher.py"}],
      "process_timeout_secs": 30
    },
    {
      "id": "validate",
      "providers": [{"type": "subprocess", "command": "python validator.py"}],
      "process_timeout_secs": 10
    }
  ],
  "edges": [
    { "from": "fetch_data", "to": "validate", "trigger": "all", "event": "complete" }
  ],
  "dataflows": [
    { "from": "fetch_data", "to": "validate" }
  ]
}
```

所有工具统一通过 `type: "subprocess"` 接入：

**OpenCode：**
```json
{
  "type": "subprocess",
  "command": "opencode run --format json --dangerously-skip-permissions -- \"review this code\""
}
```

**Claude Code：**
```json
{
  "type": "subprocess",
  "command": "claude -p \"review this code\" --output-format json --model claude-sonnet-4"
}
```

**链式传递上游数据（模板插值）：**
```json
{
  "type": "subprocess",
  "command": "opencode run --format json --dangerously-skip-permissions -- \"{{inputs.config}}\""
}
```

## 工作原理

### 三层架构

```mermaid
graph TD
    subgraph "引擎 Engine"
        E1["事件循环"]
        E2["调度 · 并发控制"]
        E3["边: 触发判定"]
        E4["机械计数 · 数据路由"]
    end
    subgraph "NodeShell"
        N1["根据 command 启动进程"]
        N2["逐行流式读取 stdout"]
        N3["exit_code → 事件类型"]
    end
    subgraph "节点 Node"
        P1["读 stdin → 计算 → 写 stdout"]
        P2["exit 0"]
    end

    E1 -->|NodeShell::run| N1
    N1 -->|stdin / stdout| P1
```

### 执行流程

```
Scheduler → 决定谁 ready
    ↓ NodeReady
Semaphore → acquire() 等空位
    ↓ spawn
SubprocessExecutor → stdin 写 JSON + 逐行读 stdout（实时 chunk）+ wait
    ↓ stdout（逐行实时）
DataRouter → 存储输出，传递给下游
```

### 边触发算法

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

fan-in 场景（A 和 B 完成后触发 C）通过两条独立边实现。

### 重试机制

重试仅对 **Timeout** 和 **SpawnError**（子进程启动失败）生效。exit-code 失败（exit_code ≠ 0）不自动重试，直接走 Failed 出边。

```
节点 Timeout / SpawnError
  → retry_count < max_retries (默认 3)?
    → retry_count++ → 重新执行节点
  → 否则触发 Timeout/Failed 出边
```

## CLI

```
nexus-cli run <workflow.json>

  --max-concurrency N    最大并发节点数（默认：CPU 核数）
  --node-timeout S       节点默认超时秒数（默认：3600），被节点级
                         process_timeout_secs 覆盖
  --max-timeout-retries N
                         超时和 spawn 失败的重试次数（默认：3）
  --verbose              详细日志（含流式 chunk 输出）
  --validate-only        仅验证，不执行
  --dump-state           输出节点状态快照

退出码：
  0  Success
  1  Validation error
  2  Runtime error
  3  Idle timeout
```

## 模式

| 模式 | 说明 |
|------|------|
| 链式 | A → B → C，严格顺序执行 |
| 扇出/扇入 | A → B, A → C → D（All 等待全部完成） |
| 条件分支 | review → approved/deploy, rejected/fix（exit_reason 路由） |
| 带阈值循环 | collector 自环 3 次后触发 aggregator（threshold: 3） |
| 错误处理 | A(Complete → B, Failed → error_handler) |
| 并行聚合 | A, B, C 同时执行 → M 等待全部完成（All） |

详细模式模板见 [`references/WORKFLOW_REFERENCE.md`](./references/WORKFLOW_REFERENCE.md)。

## crate 划分

```
nexus-engine/    ← 核心引擎库（lib crate）
nexus-cli/       ← 命令行工具（binary crate，1.5MB 静态链接）
nexus-mcp-server/← MCP 服务器（JSON-RPC stdio 协议）
```

## 构建

```bash
# 构建
cargo build --release

# 打包（独立分发）
powershell -File scripts\package.ps1
```

产物在 `nexus-dist/` 目录——单个 `nexus-cli.exe`（1.5MB），零运行时依赖。

## 系统要求

- 编译：Rust 1.83+ (edition 2024)
- 运行：Windows (GNU/MSVC) 或 Linux

---

**Nexus 不封装你的逻辑。它只提供图的骨架。你把骨架填上子进程，它就跑。**
