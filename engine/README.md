# Nexus Engine

> Rust 引擎端 — 核心引擎、CLI、MCP Server

## 快速开始

```bash
cargo build --release
# 示例工作流在 ../release/examples/
./target/release/nexus-cli run ../release/examples/branch-routing-e2e.json --verbose
```

## 目录结构

```
crates/
├── engine/         核心引擎库（nexus-engine）
├── cli/            CLI 工具（nexus-cli）
├── mcp-server/     MCP 服务器（nexus-mcp-server）
└── dashboard/      Dashboard 后端（REST + WebSocket）
scripts/            构建与 LLM wrapper 脚本
../release/         发布包（二进制 + 示例 + 文档）
```

## 构建

```bash
cargo build --workspace --release
```
