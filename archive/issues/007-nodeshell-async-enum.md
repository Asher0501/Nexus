# Issue 007: NodeShell 改为 enum dispatch + async fn

**类型**: 工程实现  
**来源**: Deep Review 问题 2 + ARCHITECTURE_REVIEW 问题 10  
**状态**: ✅ **已实现** — NodeExecutor 采用 enum dispatch（Subprocess / Http 两个变体），`impl dyn` 从未进入代码库

---

## 问题描述

当前 `NodeShell` trait 声明为同步签名：

```rust
pub trait NodeShell: Send + Sync {
    fn run(&self, config: &ProviderDef, ctx: NodeContext, timeout: Duration)
        -> Result<NodeOutcome, SpawnError>;
}
```

但所有具体实现（SubprocessExecutor、HttpExecutor）必然需要异步 I/O：

| 操作 | 依赖 |
|------|------|
| 启动子进程 | `tokio::process::Command` |
| 写入 stdin | `tokio::io::AsyncWrite` |
| 等待退出 | `Child::wait()` 返回 `impl Future` |
| 超时处理 | `tokio::time::timeout` |

如果调用方在同步上下文中 `block_on`，会占用 tokio 工作线程，可能导致线程池耗尽。

## 设计哲学影响

异步与否不影响去中心化命题——信息所有权不变（边判触发、节点算数据），引擎仍是闭包求值器。异步只是调度机制的优化，不是信息所有权的改变。

## 解决方案

用 `NodeExecutor` enum 替代 `NodeShell` trait，天然支持 `async fn`，零堆分配：

```rust
pub enum NodeExecutor {
    Subprocess(SubprocessExecutor),
    Http(HttpExecutor),
}

impl NodeExecutor {
    pub async fn run(&self, ctx: NodeContext, process_timeout: Duration)
        -> Result<NodeOutcome, SpawnError>
    {
        match self {
            NodeExecutor::Subprocess(exe) => exe.run(ctx, process_timeout).await,
            NodeExecutor::Http(exe) => exe.run(ctx, process_timeout).await,
        }
    }
}
```

优势：
- enum dispatch，编译器保证新变体必须被处理
- `async fn` 直接在 enum 上，不需要 `#[async_trait]` 的堆分配
- ProviderDef 可以映射到 enum 变体，与 JSON schema 对应

## 影响范围

- 删除 `NodeShell` trait
- 新增 `NodeExecutor` enum
- `SubprocessExecutor`、`HttpExecutor` 各自实现 `async fn run`
- Executor 中调用 `NodeExecutor::run` 代替 `NodeShell::run`
- 不影响 Edge、Scheduler、Event Loop

## 优先级

P0 — 编码前必须处理
