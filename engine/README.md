# Nexus Engine

> Rust 引擎端 — 核心引擎、CLI、MCP Server

## 快速开始

```bash
cargo build --release
cd crates/cli && cargo run -- run ../examples/workflows/parallel-review.json --verbose
```

## 目录结构

```
crates/
├── engine/         核心引擎库（nexus-engine）
├── cli/            CLI 工具（nexus-cli）
├── mcp-server/     MCP 服务器（nexus-mcp-server）
└── dashboard/      Dashboard 后端（待建）
examples/
└── workflows/      示例工作流
scripts/            构建脚本
```

## 构建

```bash
cargo build --workspace --release
```
