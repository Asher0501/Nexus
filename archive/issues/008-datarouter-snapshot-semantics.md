# Issue 008: DataRouter snapshot semantics 文档化

**类型**: 工程实现 / 文档补充  
**来源**: Deep Review 问题 3  
**状态**: 已解决

---

## 问题描述

DataRouter 的"最新输出"语义缺乏版本概念。多轮次场景下，目标节点拿到的是多个上游各自的"最新"输出，但这些最新输出的时间点可能不一致。

三种方案评估：

| 方案 | 描述 | v1 适用性 |
|------|------|-----------|
| A: snapshot semantics | 只保留最新，明确文档化 | ✅ 适合 v1 |
| B: 输出版本号 | 按 (node_id, version) 索引 | ❌ 过度设计 |
| C: edge 级别策略 | 不同边用不同策略 | ❌ 过度设计 |

## 解决方案

明确文档化为 **snapshot semantics（最新快照语义）**，DataRouter 只保留每个节点最新一次的输出。

## 设计哲学

DataRouter 是引擎的机械路由组件，不涉及业务理解。snapshot semantics 是最简模型——引擎不做"对齐版本"等业务判断。

## 影响范围

- ARCHITECTURE.md §5.5 文档已更新
- 不影响运行时逻辑

## 优先级

P2 — v1 发布前
