# Human-in-the-Loop Architecture

## 1. Overview

Allow an LLM node (`type: "llm_sdk"`) to ask a human for input mid-execution and resume after receiving the answer. The interaction surface can be Dashboard (browser) or CLI (terminal).

**Key principle: zero engine changes.** The entire mechanism is implemented in the SDK Python wrapper + Dashboard frontend + one new API endpoint. The engine correctly handles long-running nodes already.

## 2. Data Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│                         NEXUS ENGINE (unchanged)                     │
│                                                                     │
│  NodeShell::LlmSdk                                                  │
│       │                                                             │
│       │ spawn                                                        │
│       ▼                                                             │
│  ┌──────────────────────────────────────────────┐                   │
│  │            llm_sdk.py (Python)               │                   │
│  │                                              │                   │
│  │  call_api()                                  │                   │
│  │    │                                         │                   │
│  │    ├─ Anthropic Messages API ──→ LLM         │                   │
│  │    │        │                                │                   │
│  │    │        ├─ stop_reason: "end_turn" → return text              │
│  │    │        │                                │                   │
│  │    │        └─ stop_reason: "tool_use"       │                   │
│  │    │              │                          │                   │
│  │    │              ├─ tool: read_file    → execute, loop            │
│  │    │              ├─ tool: write_file   → execute, loop            │
│  │    │              ├─ tool: ask_human    → execute, loop    ◄── NEW│
│  │    │              └─ (max 20 turns)         │                   │
│  │    │                                        │                   │
│  │    └─ parse_output() → {route, content}     │                   │
│  │    └─ try_route_correction()                │                   │
│  │    └─ write_output() → stdout               │                   │
│  └──────────────────────────────────────────────┘                   │
│       │ stdout (JSON) / stderr (chunks)                             │
│       ▼                                                             │
│  Engine captures stdout → DataRouter → downstream nodes             │
│  Engine captures stderr → NodeChunk → WebSocket / CLI callback      │
└─────────────────────────────────────────────────────────────────────┘
                              │
                    stderr: "[HUMAN_QUESTION]{...}"
                              │
         ┌────────────────────┼────────────────────┐
         ▼                    ▼                    ▼
   Dashboard SPA         CLI (terminal)       MCP Server
   WebSocket → UI        stderr → print       (unsupported)
   User types answer     User types answer
         │                    │
         ▼                    ▼
   POST /api/runs/{id}   echo '{"answer":"B"}' 
   /human_answer         > tmp/human_answer_{id}.json
         │                    │
         └────────────────────┘
                    │
                    ▼
         tmp/human_answer_{id}.json  ← llm_sdk.py polls this file
                    │
                    ▼
         tool_result → Anthropic API → LLM resumes
```

## 3. ask_human Tool Schema

Declared alongside `read_file` / `write_file` in `FILE_TOOLS`:

```json
{
    "name": "ask_human",
    "description": "Ask a human for input when you are uncertain about a decision, need clarification, or require domain knowledge. Use this to resolve ambiguity before proceeding. The human will see your question and options, then respond.",
    "input_schema": {
        "type": "object",
        "properties": {
            "question": {
                "type": "string",
                "description": "The question to ask the human. Be specific."
            },
            "context": {
                "type": "string",
                "description": "Brief context explaining why you need human input and what decision depends on it."
            },
            "options": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Suggested options for the human to choose from. Helps avoid free-text ambiguity. Leave empty if open-ended."
            }
        },
        "required": ["question"]
    }
}
```

## 4. ask_human Execution Flow (in llm_sdk.py)

```python
def _execute_tool(tool_name, tool_input):
    ...
    if tool_name == "ask_human":
        return _handle_ask_human(tool_input)


def _handle_ask_human(tool_input):
    question_id = str(uuid.uuid4())[:8]
    question = tool_input.get("question", "No question provided")
    options = tool_input.get("options", [])
    context = tool_input.get("context", "")

    # 1. Signal to the interaction surface via stderr (engine → chunk → WS/CLI)
    payload = json.dumps({
        "id": question_id,
        "question": question,
        "options": options,
        "context": context,
    })
    log(f"[HUMAN_QUESTION]{payload}")

    # 2. Write question file for polling-based answer delivery
    answer_path = HUMAN_ANSWER_DIR / f"human_answer_{question_id}.json"
    question_path = HUMAN_ANSWER_DIR / f"human_question_{question_id}.json"
    question_path.write_text(json.dumps({
        "question": question,
        "options": options,
        "context": context,
        "status": "waiting",
        "node_id": os.environ.get("NEXUS_NODE_ID", ""),
    }))

    # 3. Poll for answer (file-based IPC, works across all interaction surfaces)
    deadline = time.time() + HUMAN_TIMEOUT
    while time.time() < deadline:
        if answer_path.exists():
            try:
                answer = json.loads(answer_path.read_text())
                answer_path.unlink()  # Clean up
                question_path.unlink()
                return answer.get("answer", answer.get("selection", str(answer)))
            except (json.JSONDecodeError, OSError):
                pass
        time.sleep(HUMAN_POLL_INTERVAL)  # 500ms

    # Timeout
    question_path.unlink(missing_ok=True)
    return "ERROR: Human did not respond within the timeout period."


# Configurable via environment variables
HUMAN_ANSWER_DIR = Path(os.environ.get("NEXUS_HUMAN_DIR", tempfile.gettempdir()))
HUMAN_TIMEOUT = int(os.environ.get("NEXUS_HUMAN_TIMEOUT", "86400"))  # 24h default
HUMAN_POLL_INTERVAL = float(os.environ.get("NEXUS_HUMAN_POLL_INTERVAL", "0.5"))
```

## 5. Interaction Surfaces

### 5.1 Dashboard (browser)

```
WebSocket receives NodeChunk
    │
    ▼
text.startsWith("[HUMAN_QUESTION]") ?
    │
    ▼ YES
parse JSON payload → render question card in node's log panel
    │
    ├─ Show context (why LLM is asking)
    ├─ Show question text
    ├─ If options provided → render as buttons
    │     [方案A: 微服务] [方案B: 单体]
    ├─ If no options → render text input + [Submit]
    │     [________________] [Submit]
    │
    ▼ User clicks button or types answer → 
    │
    POST /api/runs/{run_id}/human_answer
        {"question_id": "...", "answer": "方案B"}
    │
    ▼ API handler writes answer file → wrapper's polling loop finds it
```

**New API endpoint:**

```
POST /api/runs/{run_id}/human_answer
Content-Type: application/json
Body: {"question_id": "abc12345", "answer": "方案B"}

Response: 200 {"status": "accepted"}
```

Endpoint implementation (in `runs.rs`):
```rust
// Read question_id from body
// Compute answer file path: HUMAN_ANSWER_DIR / f"human_answer_{question_id}.json"
// Write {"answer": answer} to file
// Return 200
```
Zero engine interaction — just writes a file that llm_sdk.py is already polling.

### 5.2 CLI (terminal)

```
NodeChunk callback prints to stderr:
[ ] review: [HUMAN_QUESTION]{"question":"选方案A还是B？","options":["A","B"],"id":"a1b2"}

Terminal shows:
─────────────────────────────────────────
🤔 review 需要人工输入:
─────────────────────────────────────────
问: 选方案A还是B？
选项: [1] 方案A  [2] 方案B
─────────────────────────────────────────
输入编号或答案: _
─────────────────────────────────────────

User types "2" + Enter

CLI stdin thread → writes {"answer":"方案B"} to answer file
```

**CLI implementation** (in `main.rs`):
```rust
// Before engine.run():
// Spawn a stdin reader thread:
//   loop { read stdin line → check if there's a pending question file → write answer }
//
// NodeEventCb handler for NodeChunk:
//   if text starts with "[HUMAN_QUESTION]" → parse, print formatted question, 
//   set a flag that stdin thread checks
```

Actually, a simpler CLI approach: the CLI just prints the question and tells the user to write the answer file manually. Humans using CLI are power users.

```
[ ] review: [HUMAN_QUESTION] id=a1b2 | 选方案A还是B？选项:["A","B"]
→ To answer: echo '{"answer":"方案B"}' > /tmp/human_answer_a1b2.json
```

This works today with zero CLI changes. Interactive stdin reading can be added later.

## 6. File Layout for Answers

```
{HUMAN_ANSWER_DIR}/
├── human_question_{id}.json   ← wrapper writes question + status
└── human_answer_{id}.json     ← interaction surface writes answer
```

Default: system temp directory. Configurable via `NEXUS_HUMAN_DIR` env var.

## 7. Error Handling & Edge Cases

| Scenario | Behavior |
|----------|----------|
| Human never responds | llm_sdk.py times out after `HUMAN_TIMEOUT` (default 24h) → returns "ERROR: no response" → LLM sees error → continues or outputs route:"error" |
| Human responds with empty string | Treated as valid answer: "" |
| Multiple concurrent ask_human calls | Each gets unique question_id → separate answer files |
| Engine is cancelled mid-wait | Engine exits; wrapper process orphaned, continues polling; answer file still accepted; subprocess terminates after timeout |
| Wrapper process killed mid-wait | Engine detects exit → handles as Failed event |
| Malformed answer JSON | Polling loop retries until valid JSON or timeout |
| Dashboard reload during wait | Chunk not replayed; answer file still polled; interaction can resume via API directly |
| Answer file already exists from previous run | question_id is UUID-based, collision probability negligible |

## 8. Implementation Checklist

### Phase 1: Core (llm_sdk.py only)
- [ ] Add `ask_human` to `FILE_TOOLS`
- [ ] Add `_handle_ask_human()` function
- [ ] Add `HUMAN_ANSWER_DIR`, `HUMAN_TIMEOUT`, `HUMAN_POLL_INTERVAL` config
- [ ] Unit test: ask_human with simulated answer file

### Phase 2: Dashboard API
- [ ] `POST /api/runs/{run_id}/human_answer` endpoint
- [ ] Integration test: endpoint writes answer file, wrapper detects it

### Phase 3: Dashboard UI
- [ ] Detect `[HUMAN_QUESTION]` prefix in WebSocket chunk messages
- [ ] Render question card with options/input
- [ ] POST answer on submit
- [ ] Integration test: full flow Dashboard → answer → LLM resumes

### Phase 4: CLI (optional)
- [ ] Print formatted question on `[HUMAN_QUESTION]` chunk
- [ ] (Stretch) stdin thread for interactive input

## 9. Non-Goals (explicitly out of scope)

- No engine changes
- No checkpoint/resume mechanism
- No MCP Server support
- No multi-human approval workflows (one question = one answer)
- No conversation history across ask_human calls (each is independent)
