# Issue 006: timeout_secs → process_timeout_secs（明确超时边界）

**类型**: 工程实现  
**来源**: Deep Review 问题 1  
**状态**: 待实现

---

## 问题描述

当前 `NodeParams` 中只有一个 `timeout_secs: u64` 字段，但实际存在两种不同性质的超时：

| 超时类型 | 语义 | 谁管理 | 超时后的行为 |
|---------|------|--------|-------------|
| **进程存活超时** | 子进程最多活多久 | **引擎** | 强杀进程，标记 Failed |
| **业务计算超时** | 节点内部调用外部服务的等待时间 | **节点自己** | 节点自行决定重试或降级 |

这两个超时混用会导致：
- 设短了：节点内部重试还没完成就被引擎强杀
- 设长了：失去安全网的保护意义

## 设计哲学边界

> 引擎只做机械计数和转发，不做业务判断。— 原则 6

进程存活超时是"引擎安全网"，属于引擎该做的事。业务计算超时是"节点内部策略"，属于节点自己的事。引擎不应理解或管理业务计算超时。

当前设计哲学的原则 6 已覆盖此边界——引擎只做机械的事。此 issue 仅将其明确到超时层面。

## 解决方案

`timeout_secs` 改名为 `process_timeout_secs`，明确语义为"进程存活超时"。不新增业务超时字段——业务超时由节点自己管理，引擎不关心。

```rust
pub struct NodeParams {
    /// 进程存活超时（秒）。超过此时间引擎强杀子进程，标记为 Failed。
    pub process_timeout_secs: u64,
}
```

JSON 配置对应更新：

```json
{
  "id": "fetch_data",
  "providers": [...],
  "process_timeout_secs": 30
  // 入口节点：旧版用 "predecessors": []，新版在顶层声明 edges，未出现在 edges[].from 的节点即为入口
}
```

## 影响范围

- `NodeDef.timeout_secs` → `NodeDef.process_timeout_secs`
- `NodeParams` 对应字段改名
- ARCHITECTURE.md 中所有引用 `timeout_secs` 的地方同步改名
- 不影响运行时逻辑

## 优先级

P1 — 编码阶段纳入
