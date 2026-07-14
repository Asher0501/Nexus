"""
LLM Node — universal Nexus wrapper for any LLM CLI.

Supports any CLI via --cmd (engine passes the full rendered command):
  claude:   claude -p "prompt" --output-format stream-json --include-partial-messages
  opencode: opencode run --format json --auto -- "prompt"
  nga:      nga run --json "prompt"
  codeagent: codeagent -p "prompt"

Protocol:
  stdin   ← Nexus NodeContext JSON (inputs, extensions, metadata)
  stdout  → real-time forwarded to stderr (engine captures → log chunks)
  stdout  → final line parsed as Nexus NodeOutput for routing
  stderr  → also forwarded to engine log
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time


# ═══════════════════════════════════════════════════════════════════
# CLI args
# ═══════════════════════════════════════════════════════════════════

def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Nexus universal LLM node")
    p.add_argument("--cmd", default="",
                   help="Full rendered CLI command to execute")
    p.add_argument("--route", default="",
                   help="Comma-separated expected route values")
    p.add_argument("--timeout", type=int, default=0,
                   help="Internal timeout seconds (0 = rely on engine)")
    return p.parse_args()


# ═══════════════════════════════════════════════════════════════════
# Nexus protocol — stdin
# ═══════════════════════════════════════════════════════════════════

def read_nexus_context() -> dict:
    raw = sys.stdin.read()
    if not raw.strip():
        return {"inputs": {}, "extensions": {}, "metadata": {"run_count": 1, "timed_out": False}}
    return json.loads(raw)


# ═══════════════════════════════════════════════════════════════════
# CLI resolution — cross-platform (Windows + Linux)
# ═══════════════════════════════════════════════════════════════════

def _resolve_program(name: str) -> str:
    """Resolve CLI name to executable path."""
    # Prefer .exe on Windows (bypasses .cmd wrapper)
    if sys.platform == "win32":
        exe = shutil.which(name + ".exe")
        if exe:
            return exe
    return shutil.which(name) or name


# ═══════════════════════════════════════════════════════════════════
# Subprocess execution
# ═══════════════════════════════════════════════════════════════════

def emit_stderr(line: str):
    print(line, file=sys.stderr, flush=True)


def run_cmd(cmd_str: str, stdin_text: str | None = None) -> subprocess.Popen:
    """Spawn CLI. Prompt goes via stdin (not -p), avoiding command-line limits."""
    program = cmd_str.split(" ", 1)[0]
    exe = _resolve_program(program)
    if exe != program:
        cmd_str = exe + cmd_str[len(program):]

    emit_stderr(f"[llm_node] {os.path.basename(exe)}")
    proc = subprocess.Popen(
        cmd_str,
        stdin=subprocess.PIPE if stdin_text else subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        shell=False,
    )
    if stdin_text and proc.stdin:
        proc.stdin.write(stdin_text)
        proc.stdin.close()
    return proc


# ═══════════════════════════════════════════════════════════════════
# Output streaming → engine log (not routing)
# ═══════════════════════════════════════════════════════════════════

def stream_stdout_and_stderr(proc: subprocess.Popen, timeout: int) -> str:
    """Read stdout line-by-line, forward each line to stderr (engine log).
    Returns complete stdout for final parsing. Stderr read concurrently.
    Sends periodic heartbeats to flush pipe buffers for real-time streaming."""
    deadline = time.time() + timeout if timeout > 0 else None
    stdout_lines: list[str] = []
    stderr_done = threading.Event()
    heartbeat_stop = threading.Event()

    def _read_stderr():
        try:
            for line in proc.stderr:
                line = line.rstrip("\n\r")
                if line:
                    emit_stderr(line)
        except Exception:
            pass
        finally:
            stderr_done.set()

    def _heartbeat():
        while not heartbeat_stop.is_set():
            time.sleep(0.3)
            if not heartbeat_stop.is_set():
                emit_stderr(" ")

    threading.Thread(target=_read_stderr, daemon=True).start()
    threading.Thread(target=_heartbeat, daemon=True).start()

    try:
        for line in proc.stdout:
            line = line.rstrip("\n\r")
            if line:
                stdout_lines.append(line)
                emit_stderr(line)
            if deadline and time.time() > deadline:
                proc.kill()
                emit_stderr("[llm_node] timeout")
                break
    except Exception:
        pass
    finally:
        heartbeat_stop.set()

    stderr_done.wait(timeout=5)
    return "\n".join(stdout_lines)


# ═══════════════════════════════════════════════════════════════════
# Output parsing — find route+content JSON in any output
# ═══════════════════════════════════════════════════════════════════

ROUTE_RE = re.compile(
    r'"route"\s*:\s*"([^"\\]*(?:\\.[^"\\]*)*)"\s*,\s*"content"\s*:\s*"((?:[^"\\]|\\.)*)"',
    re.DOTALL,
)

def parse_text_output(stdout: str) -> dict:
    """Search ALL output for route+content JSON, anywhere."""
    # 1. Try direct JSON parse of the entire output
    stripped = stdout.strip()
    if stripped.startswith("{"):
        try:
            obj = json.loads(stripped)
            if "route" in obj:
                return {"route": str(obj["route"]), "content": str(obj.get("content", ""))}
        except json.JSONDecodeError:
            pass

    # 2. Normalize: unescape JSON string escapes.  In NDJSON, the route JSON
    # appears as {\"route\":\"again\",...}.  Turn \" into plain " so the regex matches.
    unescaped = stdout.replace('\\"', '"').replace('\\\\', '\\')
    for m in ROUTE_RE.finditer(unescaped):
        route = m.group(1)
        # Unescape JSON string escapes (\", \\, \n, etc.)
        content_raw = m.group(2)
        try:
            content = json.loads('"' + content_raw + '"')
        except json.JSONDecodeError:
            content = content_raw
        return {"route": route, "content": content}

    # 3. Fallback
    return {"route": "", "content": stripped}


# ═══════════════════════════════════════════════════════════════════
# Main
# ═══════════════════════════════════════════════════════════════════

def main():
    args = parse_args()
    ctx = read_nexus_context()

    cmd_str = args.cmd or ctx.get("extensions", {}).get("_cmd", "")
    if not cmd_str:
        sys.stdout.write(json.dumps({"route": "error", "content": "no --cmd provided"}))
        sys.exit(1)

    # Get prompt from stdin context (engine sends it there).
    prompt = ctx.get("extensions", {}).get("prompt", "")

    # If command has {{prompt}}, replace it (backward compat, -p mode).
    if prompt and "{{prompt}}" in cmd_str:
        safe_prompt = prompt.replace("\\", "\\\\").replace("\"", "\\\"")
        cmd_str = cmd_str.replace("{{prompt}}", safe_prompt)
        use_stdin = False
    else:
        # No {{prompt}} → pass prompt via CLI stdin instead of -p
        use_stdin = bool(prompt)

    try:
        proc = run_cmd(cmd_str, stdin_text=prompt if use_stdin else None)
    except FileNotFoundError as e:
        sys.stdout.write(json.dumps({"route": "error", "content": f"not found: {e}"}))
        sys.exit(1)

    stdout_text = stream_stdout_and_stderr(proc, args.timeout)
    emit_stderr(f"[llm_node] captured {len(stdout_text)} chars from stdout")
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    output = parse_text_output(stdout_text)
    sys.stdout.write(json.dumps(output, ensure_ascii=False))


if __name__ == "__main__":
    main()
