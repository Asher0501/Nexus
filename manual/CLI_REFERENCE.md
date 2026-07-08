# CLI 参考

> Nexus CLI 的完整用法说明、参数、退出码和调试方法。

**分类**：manual

---

## 用法

```bash
nexus-cli run <workflow.json> [OPTIONS]
```

## 参数

| 参数 | 默认值 | 说明 |
| --- | --- | --- |
| `--max-concurrency N` | CPU 核心数 | 最大并发节点数 |
| `--node-timeout S` | 3600 | 节点默认超时秒数，被节点级 `process_timeout_secs` 覆盖 |
| `--max-timeout-retries N` | 3 | 超时和 spawn 失败的重试次数。exit-code 失败（exit_code ≠ 0）不自动重试 |
| `--verbose` | — | 启用详细日志（显示流式 chunk 输出和 stderr） |
| `--validate-only` | — | 只做验证，不执行 |
| `--dump-state` | — | 完成后输出节点状态快照 |

## 退出码

| 退出码 | 含义 |
| --- | --- |
| 0 | Success |
| 1 | Validation error |
| 2 | Runtime error |
| 3 | Node timeout |

## 验证错误速查

`nexus-cli run --validate-only` 在 JSON 无效时打印以下错误：

| 错误信息 | 原因 |
| --- | --- |
| `empty graph: no nodes defined` | nodes 数组为空 |
| `duplicate node ID: 'X'` | 两个节点 ID 相同 |
| `no entry node: all nodes have predecessors` | 所有节点都有入边，没有入口节点 |
| `unreachable node 'X': not reachable from any entry node` | 节点在调度图中不可达 |
| `cycle without entry: deadlock detected` | 存在无入口的环（可能自环 threshold=1，或多个节点形成环路且环内无入口节点） |
| `exit not reachable from node 'X'` | 节点 X 没有路径到达任何出口节点（无出边的节点） |
| `node 'X' has no valid provider` | providers 数组为空 |
| `node 'X' references non-existent predecessor 'Y'` | edges 中 from/to 引用了不存在的节点 ID |
| `input source 'Y' for node 'X' is not reachable from any entry` | dataflows 中 from 节点不可达 |
| `build invariant failure: ...` | 内部图构造不变量检查失败。排查节点 ID 引用和 dataflows 声明是否完整 |

## 通用调试步骤

1. **验证 JSON 结构**：`nexus-cli run workflow.json --validate-only`，修复所有错误
2. **单节点测试**：先用一个 echo 节点验证基础流程 `nexus-cli run --validate-only`
3. **查看流式输出**：`nexus-cli run workflow.json --verbose` 查看每行 stdout 实时输出
4. **查看最终状态**：`nexus-cli run workflow.json --dump-state` 查看所有节点终态
5. **检查日志**：运行日志写入 `log/run-{timestamp}.log`
