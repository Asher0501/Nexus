# Issue 003: 子进程输出大小限制与流式处理约定

**类型**: 工程实现 / 文档补充  
**来源**: 架构审查问题 8  
**状态**: 待实现

---

## 问题描述

当前 SubprocessExecutor 的 I/O 模型将在子进程退出后一次性读取全部 stdout：

```
spawn(command)
写入 stdin (ctx JSON)
关闭 stdin
等待退出（带超时）
读取 stdout ← 一次性读完整数据
```

在以下场景存在问题：

1. **大输出**：节点产生数百 MB stdout，进程因 pipe buffer 满阻塞，引擎也在阻塞等退出 → 潜在死锁
2. **流式输出**：LLM 生成等场景，当前模型要求"等全部完成再读"，无法边出边转发
3. **stderr 收集**：文档说"引擎会收集但不解析"，但架构中没有 stderr 收集的机制

## 解决方案

v1 阶段不实现流式 I/O，但在文档中明确输出大小限制。

### ARCHITECTURE.md 补充

在 SubprocessExecutor 章节末尾添加：

> **输出大小限制**：当前 SubprocessExecutor 将子进程 stdout 一次性读入内存。单个节点输出不应超过 MAX_NODE_OUTPUT（默认 100MB）。超过此限制可能导致 OOM 或进程阻塞。

### NODE_PROTOCOL.md 补充

在"你需要知道的事"表中补充：

> | 输出大小 | stdout 不应超过 100MB，超出部分可能被丢弃或导致进程阻塞 |

## 未来规划

v2 可考虑支持：
1. **文件传递**：节点将输出写入临时文件，stdout 输出文件路径
2. **流式处理**：通过命名管道或 TCP socket 逐步传递数据

## 影响范围

- 仅文档变更，不涉及代码改动
- ARCHITECTURE.md + NODE_PROTOCOL.md

## 优先级

P2 — v1 发布前
