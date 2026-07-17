"""
Human Pool Server — in-memory question/answer pool with HTTP API.

Started by the first wrapper that needs human input (port 19876).
All wrappers share this single pool. A separate consumer process
reads pending questions, displays them on the terminal, and posts
answers.

Endpoints:
  POST /q          Register a question → {qid, question, options, context}
                   Returns {"qid": "uuid"}
  GET  /a/{qid}    Block until answer is available (threading.Event)
                   Returns {"answer": "..."}
  GET  /pending    List all unanswered questions (used by consumer)
                   Returns [{"qid":"...","question":"...","options":[...]}, ...]
  POST /a/{qid}    Write an answer (used by consumer)
                   Body: {"answer": "..."}
  GET  /health     Health check
"""

import json
import sys
import threading
import uuid
from http.server import HTTPServer, BaseHTTPRequestHandler
from socketserver import ThreadingMixIn

class ThreadingHTTPServer(ThreadingMixIn, HTTPServer):
    daemon_threads = True
from urllib.parse import urlparse

# ── In-memory pool ──────────────────────────────────────────

_lock = threading.Lock()
_pool: dict = {}  # qid → {"question", "options", "context", "answer", "event"}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass  # silence HTTP logs

    def _json(self, code: int, data: dict):
        body = json.dumps(data, ensure_ascii=True).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        path = urlparse(self.path).path
        length = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(length) if length else b"{}"
        try:
            body = json.loads(raw)
        except json.JSONDecodeError:
            return self._json(400, {"error": "invalid json"})

        if path == "/q":
            qid = str(uuid.uuid4())
            question = body.get("question", "No question")
            options = body.get("options", [])
            context = body.get("context", "")
            ev = threading.Event()
            with _lock:
                _pool[qid] = {
                    "question": question,
                    "options": options,
                    "context": context,
                    "answer": None,
                    "event": ev,
                }
            return self._json(200, {"qid": qid})

        if path.startswith("/a/"):
            qid = path.split("/a/", 1)[1]
            with _lock:
                entry = _pool.get(qid)
            if not entry:
                return self._json(404, {"error": "qid not found"})
            answer = body.get("answer", "")
            entry["answer"] = answer
            entry["event"].set()  # wake blocked GET
            return self._json(200, {"status": "accepted"})

        return self._json(404, {"error": "not found"})

    def do_GET(self):
        path = urlparse(self.path).path.strip("/")

        if path == "health":
            return self._json(200, {"status": "ok", "pool_size": len(_pool)})

        if path == "pending":
            with _lock:
                items = [
                    {"qid": qid, "question": e["question"],
                     "options": e["options"], "context": e["context"]}
                    for qid, e in _pool.items()
                    if e["answer"] is None
                ]
            return self._json(200, {"questions": items})

        if path.startswith("a/"):
            qid = path[2:]
            with _lock:
                entry = _pool.get(qid)
            if not entry:
                return self._json(404, {"error": "qid not found"})
            # Block until consumer writes answer
            entry["event"].wait()
            answer = entry.pop("answer")
            with _lock:
                _pool.pop(qid, None)
            return self._json(200, {"answer": answer})

        return self._json(404, {"error": "not found"})


def start_server(port: int = 19876):
    srv = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    srv.allow_reuse_address = True
    t = threading.Thread(target=srv.serve_forever, daemon=True)
    t.start()
    return srv, t


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 19876
    srv, _ = start_server(port)
    print(f"Human pool server on http://127.0.0.1:{srv.server_port}")
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        srv.shutdown()
