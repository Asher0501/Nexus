# Audit Review Notes (internal scratch)

## Comparison baseline: ARCHITECTURE.md vs OPTIMIZED.md

### ARCHITECTURE.md original design:
- question_id: str(uuid.uuid4())[:8] — truncated 8-char
- Polling: 500ms loop, time.sleep()
- No auth on /human_answer endpoint
- No input validation on question_id
- Default dir: system temp dir
- No atomic operations
- No audit logs
- No cleanup/TTL

### OPTIMIZED.md claimed fixes:
1. ✅ question_id → full 36-char UUID v4
2. ✅ JWT Bearer auth on API
3. ✅ question_id regex /^[a-f0-9-]{36}$/
4. ✅ Atomic write: tmp + rename
5. ✅ Secure dir ~/.nexus/human_answers (0700)
6. ✅ Structured audit logs
7. ✅ Async event-driven (inotify)
8. ✅ TTL cleanup + signal handler
9. ✅ Degradation strategies
10. ✅ pending-questions endpoint
11. ✅ WebSocket notifications
12. ✅ Honest conclusion (removed false claim)

## New issues discovered

### P0: asyncio.run() inside sync context — BROKEN
- _handle_ask_human_async() calls asyncio.run()
- If llm_sdk.py call_api() runs in an existing event loop (typical for async SDKs), this raises RuntimeError
- Cannot nest asyncio.run() inside running loop
- Makes the entire async pathway non-functional

### P0: os.rename() NOT atomic on Windows
- Target deployment includes Windows (path C:/Users/...)
- POSIX rename() is atomic; Windows rename() is NOT (fails if dest exists, or replaces)
- No Windows-specific handling documented
- Cross-device rename() also fails silently

### P1: Signal handler incompatible with asyncio
- signal.signal(SIGTERM, cleanup) conflicts with asyncio event loop
- Should use loop.add_signal_handler() — but Windows doesn't support that either
- No Windows signal handling strategy

### P1: No JWT issuance mechanism
- API requires Authorization: Bearer <jwt_token> but no login/signup endpoint
- No /api/auth/login defined
- No token refresh or rotation strategy
- Users cannot obtain tokens to use the API

### P1: inotify → polling fallback broken on Windows
- inotify: Linux only
- kqueue: macOS/BSD only  
- Windows: neither available, always falls back to polling
- No Windows-native alternative (ReadDirectoryChangesW, watchdog)
- Polling fallback: 864,000 I/O ops/day — significant on Windows

### P1: async integration gap — no adapter between sync ARCHITECTURE.md flow and new async code
- ARCHITECTURE.md _handle_ask_human() is synchronous (time.sleep loop)
- OPTIMIZED.md defines _handle_ask_human_async() but never shows how it replaces the sync version
- Main flow diagram (steps ①-⑨) references inotify but doesn't resolve this contradiction
- Implementation checklist says "改为异步事件驱动" but no migration path

### P1: question_path.write_text() not atomic
- Step ② writes question file with direct write_text() — no atomic write
- If crash mid-write, other components read partial JSON → parse errors
- Only answer writes use atomic pattern; question writes don't

### P1: TTL glob matches both question AND answer files
- glob("human_*.json") catches both human_question_*.json and human_answer_*.json
- Could delete unconsumed answer files (orphaning the question)
- No separation of cleanup logic per file type

### P2: Concurrency safety of O_EXCL not actually implemented
- Code shows tmp+rename approach (方案 A)
- O_EXCL (方案 B) is commented out with a `#` comment, never executed
- No locking mechanism for concurrent answer submissions

### P2: No rate limiting on API endpoints
- POST /human_answer and GET /pending-questions have no rate limiting
- Attackers could flood answers or exhaust directory scanning

### P2: asyncio.run() called per-invocation is inefficient
- Each ask_human call creates/destroys a new event loop
- Should use a persistent loop or properly integrate with existing one
- Heavy overhead for high-frequency ask_human calls

### P2: Response format inconsistency
- 200: {"status": "accepted", ...}
- 401: {"error": "unauthorized", ...}
- 422: {"error": "validation_error", ...}
- Mixing "status" vs "error" top-level keys — non-standard API design

### P2: No CORS policy mentioned
- Dashboard frontend likely on different origin than API
- No CORS headers or preflight handling documented
- Browser-based Dashboard will be blocked by CORS

### P2: Audit log file permissions not explicitly set
- Directory: 0700, files in dir: 0600
- But audit log files under audit/ subdirectory — are they 0600 too?
- No explicit chmod on audit log files after creation
