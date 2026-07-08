# Issue 011: exit_reason 通过 stdout 协议头传递

**类型**: 工程实现 / 文档更新  
**来源**: Deep Review 问题 9  
**状态**: 已更新文档

---

## 问题描述

exit_reason 需要通过独立于业务输出的通道传递。不能在 stdout 中混入 exit_reason（引擎不解析 stdout），也不能用环境变量（子→父不通）。

## 解决方案

stdout 第一行以 `__nexus_exit_reason: <value>` 格式作为 exit_reason 协议头。NodeShell 读取第一行提取 exit_reason，剩余部分作为业务输出。

```
stdout:
__nexus_exit_reason: approved       ← NodeShell 提取，不传给下游
{"result": "ok", "data": [...]}      ← 业务输出，传给 DataRouter
```

## 设计哲学

NodeShell 读取 stdout 第一行的固定前缀格式是协议约定，不是业务解析。NodeShell 不理解 `approved` 的含义，只是提取字符串。引擎仍然只做字符串匹配，不理解含义。

## 影响范围

- NODE_PROTOCOL.md 已更新
- ARCHITECTURE.md 不需要改（NodeOutcome.exit_reason 字段已存在）
- NodeShell/SubprocessExecutor 需要实现 stdout 协议头解析

## 优先级

P1 — 编码阶段纳入
