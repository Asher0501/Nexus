# Human-in-the-Loop — 实现方案 v2

> 基于优化后的 ToolBridge 架构 + Claude Code / LangGraph 成熟实践

## 1. 总览

```
LLM 不确定 → tool_use: ask_human
    │
    ▼
ToolBridge._run_ask_human()
    │
    ├── stderr: [HUMAN_QUESTION]{"id":"uuid","question":"...","options":[...]}
    │       │
    │       ▼ engine chunk → WebSocket → Dashboard UI 弹输入框
    │       │                        → CLI stderr 打印 + stdin 读取
    │
    ├── 写文件: {HUMAN_DIR}/{run_id}/{qid}/question.json
    │
    ├── 轮询: {HUMAN_DIR}/{run_id}/{qid}/answer.json (200ms)
    │
    └── 拿到答案 → tool_result → LLM 继续
```

**核心原则：零引擎改动。** 全部在 ToolBridge + Dashboard API + 前端。

## 2. 新增文件/改动

| 文件 | 改动 | 行数 |
|------|------|------|
| `tool_bridge.py` | 新增 `ask_human` 内置工具 + `_run_ask_human()` | ~50 |
| `dashboard/api/runs.rs` | 新增 `POST /api/runs/{id}/human_answer` | ~25 |
| `static/index.html` | 检测 `[HUMAN_QUESTION]` → 弹输入框 → POST | ~50 |
| CLI `main.rs` | NodeChunk 检测 → 格式化打印 + stdin 线程写答案 | ~60 (可选) |

**总计 ~125 行核心代码，~185 行含 CLI。**

## 3. ToolBridge — ask_human 工具

### 3.1 工具 Schema

```python
{
    "name": "ask_human",
    "description": (
        "Ask a human for input when you are uncertain about a decision, "
        "need clarification, or require domain knowledge. Use this to "
        "resolve ambiguity before proceeding. The human will see your "
        "question and options, then respond."
    ),
    "input_schema": {
        "type": "object",
        "properties": {
            "question": {
                "type": "string",
                "description": "The question to ask. Be specific and clear."
            },
            "context": {
                "type": "string",
                "description": "Brief context: why you need input, what depends on it."
            },
            "options": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Suggested options for the human to choose from. Helps avoid free-text ambiguity."
            }
        },
        "required": ["question"]
    }
}
```

### 3.2 执行逻辑

```python
def _run_ask_human(tool_input: dict) -> str:
    qid = str(uuid.uuid4())
    question = tool_input.get("question", "")
    options = tool_input.get("options", [])
    context = tool_input.get("context", "")

    # 1. 信号发射 → stderr → engine chunk → WS/CLI
    payload = json.dumps({
        "id": qid,
        "question": question,
        "options": options,
        "context": context,
    })
    sys.stderr.write(f"[HUMAN_QUESTION]{payload}\n")
    sys.stderr.flush()

    # 2. 写问题文件
    q_dir = Path(HUMAN_DIR) / (run_id or "unknown") / qid
    q_dir.mkdir(parents=True, exist_ok=True)
    (q_dir / "question.json").write_text(json.dumps({
        "id": qid, "question": question, "options": options,
        "context": context, "status": "waiting",
        "ts": time.time()
    }))

    # 3. 轮询答案文件 (200ms 间隔)
    answer_file = q_dir / "answer.json"
    deadline = time.time() + HUMAN_TIMEOUT
    while time.time() < deadline:
        if answer_file.exists():
            try:
                answer_data = json.loads(answer_file.read_text())
                answer = answer_data.get("answer", str(answer_data))
                # 反馈：答案已收到
                sys.stderr.write(f"[HUMAN_ANSWERED]{json.dumps({'id': qid})}\n")
                sys.stderr.flush()
                # 清理
                answer_file.unlink(missing_ok=True)
                (q_dir / "question.json").unlink(missing_ok=True)
                q_dir.rmdir()  # 忽略非空错误
                return answer
            except (json.JSONDecodeError, OSError):
                pass
        time.sleep(0.2)

    # 超时
    sys.stderr.write(f"[HUMAN_TIMEOUT]{json.dumps({'id': qid})}\n")
    sys.stderr.flush()
    return "ERROR: No human response received within time limit."

# 配置
HUMAN_DIR = os.environ.get("NEXUS_HUMAN_DIR", os.path.join(os.getcwd(), "tmp", "human_io"))
HUMAN_TIMEOUT = int(os.environ.get("NEXUS_HUMAN_TIMEOUT", "86400"))  # 24h
```

### 3.3 run_id 注入

ToolBridge 需要知道当前 `run_id` 来做文件隔离。通过环境变量传递：

```python
# llm_sdk.py main() 中
os.environ["NEXUS_RUN_ID"] = ctx.get("metadata", {}).get("_run_id", "unknown")
```

或者从 engine 侧写入 NodeContext.extensions。

## 4. Dashboard API — 答案接收

### 4.1 端点

```
POST /api/runs/{run_id}/human_answer
Content-Type: application/json
Body: {"question_id": "uuid", "answer": "方案B"}
```

### 4.2 实现

```rust
pub async fn human_answer(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let qid = body.get("question_id").and_then(|v| v.as_str())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let answer = body.get("answer").and_then(|v| v.as_str())
        .unwrap_or("");

    let human_dir = std::env::var("NEXUS_HUMAN_DIR")
        .unwrap_or_else(|_| "tmp/human_io".to_string());
    let q_dir = PathBuf::from(&human_dir).join(&run_id).join(qid);

    if !q_dir.exists() {
        return Err(StatusCode::NOT_FOUND);
    }
    if q_dir.join("answer.json").exists() {
        return Err(StatusCode::CONFLICT);
    }

    let payload = json!({
        "answer": answer,
        "answered_by": "dashboard",
        "ts": /* unix timestamp */,
    });
    std::fs::write(q_dir.join("answer.json"), payload.to_string())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({"status": "accepted"})))
}
```

## 5. Dashboard 前端

```javascript
// WebSocket message handler 中新增
if (msg.type === "node_chunk") {
    const t = msg.data.text;
    if (t.startsWith("[HUMAN_QUESTION]")) {
        showHumanQuestion(msg.data.node_id, JSON.parse(t.slice(17)));
    } else if (t.startsWith("[HUMAN_ANSWERED]")) {
        showHumanAnswered(msg.data.node_id);
    } else if (t.startsWith("[HUMAN_TIMEOUT]")) {
        showHumanTimeout(msg.data.node_id);
    }
}

function showHumanQuestion(nodeId, payload) {
    // 在 node 的 log panel 中渲染：
    // ┌──────────────────────────────────┐
    // │ 🤔 LLM 需要你的输入               │
    // │ 问: payload.question              │
    // │ [按钮: payload.options[0]]        │
    // │ [按钮: payload.options[1]]        │
    // │ 或自定义: [____] [提交]           │
    // └──────────────────────────────────┘
}

function submitHumanAnswer(questionId, answer) {
    fetch(`/api/runs/${currentRunId}/human_answer`, {
        method: 'POST',
        headers: {'Content-Type': 'application/json'},
        body: JSON.stringify({question_id: questionId, answer})
    });
}
```

## 6. CLI (可选)

NodeChunk callback 检测前缀 + stdin 线程读答案：

```rust
// main.rs NodeEventCb
NodeEvent::NodeChunk { ref node_id, ref text } => {
    if text.starts_with("[HUMAN_QUESTION]") {
        // 格式化打印问题 + 选项
        // 设置 pending_question
        // 提示用户输入
    }
}

// 独立 stdin 线程
std::thread::spawn(move || {
    loop {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line);
        // 如果有 pending_question → 写 answer.json
    }
});
```

## 7. 时序

```
T=0     LLM 调 ask_human → stderr 信号发射
T=0.1   WebSocket 推送到 Dashboard → UI 弹输入框
T=5     人看到 → 思考 → 输入答案 → 点提交
T=5.1   Dashboard POST /human_answer → 写 answer.json
T=5.3   wrapper 轮询发现 answer.json → 读答案 → tool_result
T=5.3   LLM 收到答案 → 继续执行
T=10    LLM 输出最终结果 → 节点 Completed
```

## 8. 边界情况

| 场景 | 行为 |
|------|------|
| 人永远不回答 | 24h 后超时 → "ERROR: no human response" → LLM 自行处理 |
| 引擎被 Stop | cancel_flag → engine 退出。wrapper 进程继续轮询直到超时。答案 API 仍可工作 |
| 多个问题同时 | 各自独立 UUID → 独立文件目录 → 独立轮询 |
| Dashboard 刷新/重启 | 问题文件在磁盘 → 答案 API 仍可用 → WebSocket 断开但不影响回答 |
