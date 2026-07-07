# Nexus

JSON 定义一张有向图 → 每个节点跑一个子进程 → 退出码决定流向。

不依赖 LLM 的即时决策。不封装你的业务逻辑。不新增一种编程语言让你学。

## 痛点

**LangGraph 类框架**的节点是 Python 函数/LLM Call，路由由 LLM 运行时决定。这意味着：

- 同一个输入可能走不同路径 → 测试覆盖无法保证
- 拓扑是运行时涌现的 → 没人能在设计时断言所有可能路径
- 约束靠 prompt 维护 → 不是架构保证

**传统工作流引擎**（Temporal、Airflow）太重。数据库、持久化、调度器、Worker 集群——你只想编排几个进程，不是部署一个基础设施。

## Nexus 的做法

**节点 = 子进程。** Python、Shell、OpenCode、Claude Code、Rust 二进制——任何 stdin/stdout 的进程。不区分 AI executor、HTTP executor、函数 executor。所有节点统一 `type: "subprocess"`。

**边 = 四维触发条件。** 事件类型（Complete/Failed/Timeout）× exit_reason × All/Any × threshold。引擎机械计数，不猜测。

**调度拓扑 ≠ 数据拓扑。** `edges` 决定谁完成后谁开始，`dataflows` 决定谁的数据传给谁。两张独立的图，你不会遇到「为了传数据而画多余边」的窘境。

## 一个工作流长这样

```json
{
  "nodes": [
    { "id": "fetch", "providers": [{"type": "subprocess", "command": "python fetcher.py"}] },
    { "id": "validate", "providers": [{"type": "subprocess", "command": "python validator.py"}] },
    { "id": "notify", "providers": [{"type": "subprocess", "command": "python notifier.py"}] }
  ],
  "edges": [
    { "from": "fetch", "to": "validate", "trigger": "all", "event": "complete" },
    { "from": "validate", "to": "notify", "trigger": "all", "event": "complete" }
  ],
  "dataflows": [
    { "from": "fetch", "to": "validate" }
  ]
}
```

OpenCode 和 Claude Code 也是子进程，不改引擎：

```json
{ "type": "subprocess", "command": "opencode run --format json --dangerously-skip-permissions -- \"review this code\"" }
```

上游数据用模板插值传递：

```json
{ "type": "subprocess", "command": "opencode run --format json --dangerously-skip-permissions -- \"{{inputs.result}}\"" }
```

## 用 CLI 跑

```bash
nexus-cli run workflow.json                  # 跑
nexus-cli run workflow.json --validate-only   # 只验证
nexus-cli run workflow.json --verbose         # 看每行流式输出
nexus-cli run workflow.json --dump-state      # 看节点最终状态
```

单个 1.5MB 静态链接 exe，零运行时依赖。`cargo build --release` 搞定。

## 什么时候用

- 你预先知道工作流的拓扑结构——审批、CI/CD、数据处理、多步 AI 审查链
- 节点是异构的——Python、Shell、OpenCode、Claude Code 混排
- 你需要稳定、可复现的生产行为——同一个输入始终走同一条路

**不适合**需要 LLM 自主探索、动态规划路径的场景——那是 agent 框架的领域。

## 支持的模式

| 模式 | 怎么做 |
|------|--------|
| 链式 | A → B → C |
| 扇出/扇入 | A → B, A → C → D（All 等待全部完成） |
| 条件分支 | review: Complete → deploy, Failed → fix |
| 自环/阈值 | collector 触发自身 3 次后触发 aggregator |
| 错误处理 | A: Complete → B, Failed → error_handler |
| 并行聚合 | A, B, C 同时执行 → M 等全部完成 |

## 构建

```bash
cargo build --release
```

产物 `target/release/nexus-cli.exe`（1.5MB），丢到任何 Windows/Linux 机器就能跑。

---

**Nexus 不封装你的逻辑。它只提供图的骨架。你把骨架填上子进程，它就跑。**
