# Nexus

> 有向图驱动的子进程编排引擎。节点 = 子进程，边 = 触发条件，引擎机械执行。

**分类**：manual

---

写一个 JSON 定义工作流，每个节点跑一个子进程，引擎按边触发条件调度执行。

## 目录结构

```
engine/     Rust 引擎端 — 核心引擎、CLI、MCP Server、Dashboard 后端
ui/         前端端 — React Dashboard（待建）
```

## 快速开始

```bash
# 引擎开发
cd engine
cargo build --release

# 前端开发（待建）
cd ui
npm install
npm run dev
```

## 文档

- **[theory/](./theory/)** — 理论基础：局部闭包定理、收敛证明、Exec 模型等价性
- **[design/](./design/)** — 软件设计：三层架构、节点协议、设计决策
- **[manual/](./manual/)** — 操作手册：工作流定义参考、CLI 用法、Agent 指南
