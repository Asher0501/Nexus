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

def _find_exe(name: str) -> list[str]:
    """Resolve a CLI name to executable argv prefix.
    Windows: prefers real .exe behind npm .cmd wrappers.
    Linux: uses bare name from PATH."""
    if sys.platform == "win32":
        exe = shutil.which(name + ".exe")
        if exe:
            return [exe]
        cmd_path = shutil.which(name + ".cmd")
        if cmd_path:
            real = _parse_cmd_for_exe(cmd_path)
            if real and os.path.isfile(real):
                return [real]
            return ["cmd.exe", "/c", name]
    found = shutil.which(name) or name
    return [found]


def _parse_cmd_for_exe(cmd_path: str) -> str | None:
    """Parse a Windows .cmd wrapper to find the real .exe."""
    try:
        with open(cmd_path) as f:
            for line in f:
                m = re.search(r'"([^"]*\.exe)"', line)
                if m:
                    rel = m.group(1).replace("%dp0%", "").lstrip("\\/")
                    return os.path.normpath(
                        os.path.join(os.path.dirname(cmd_path), rel))
    except OSError:
        pass
    return None


# ═══════════════════════════════════════════════════════════════════
# Subprocess execution — any CLI, any platform
# ═══════════════════════════════════════════════════════════════════

def emit_stderr(line: str):
    print(line, file=sys.stderr, flush=True)


def run_cmd(cmd_str: str) -> subprocess.Popen:
    """Spawn the CLI command. Resolves the program intelligently on
    Windows (preferring .exe) and falls back to shell on both platforms."""
    program = cmd_str.split(" ", 1)[0]
    prefix = _find_exe(program)

    use_shell = False
    if len(prefix) == 1 and prefix[0] != program:
        # Found a better path (e.g. claude.exe) — replace program in command
        cmd_str = prefix[0] + cmd_str[len(program):]
    elif prefix == [program] or prefix[0] == program:
        pass  # use as-is
    else:
        use_shell = True  # cmd.exe /c wrapper

    emit_stderr(f"[llm_node] {os.path.basename(prefix[0])}")
    return subprocess.Popen(
        cmd_str,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        shell=use_shell,
    )


# ═══════════════════════════════════════════════════════════════════
# Output streaming → engine log (not routing)
# ═══════════════════════════════════════════════════════════════════

def stream_stdout_and_stderr(proc: subprocess.Popen, timeout: int) -> str:
    """Read stdout line-by-line, forward each line to stderr (engine log).
    Returns complete stdout for final parsing. Stderr read concurrently."""
    deadline = time.time() + timeout if timeout > 0 else None
    stdout_lines: list[str] = []
    stderr_done = threading.Event()

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

    threading.Thread(target=_read_stderr, daemon=True).start()

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

    stderr_done.wait(timeout=5)
    return "\n".join(stdout_lines)


# ═══════════════════════════════════════════════════════════════════
# Output parsing — extract route + content from any format
# ═══════════════════════════════════════════════════════════════════

def extract_json(text: str) -> dict | None:
    if not text.strip():
        return None
    try:
        return json.loads(text.strip())
    except json.JSONDecodeError:
        pass
    m = re.search(r'\{[^{}]*"route"\s*:\s*"[^"]*"[^{}]*\}', text, re.DOTALL)
    if m:
        try:
            return json.loads(m.group(0))
        except json.JSONDecodeError:
            pass
    start, end = text.find("{"), text.rfind("}")
    if start >= 0 and end > start:
        try:
            return json.loads(text[start:end + 1])
        except json.JSONDecodeError:
            pass
    return None


def parse_text_output(stdout: str) -> dict:
    """Parse stdout into {route, content}. Handles:
    - Claude envelope: {"type":"result","result":"{...}"}
    - Direct JSON: {"route":"...","content":"..."}
    - OpenCode NDJSON: {"type":"text","part":{"text":"..."}}
    - Raw text"""
    parsed = extract_json(stdout)

    # Handle JSON arrays (e.g. claude --verbose NDJSON): take last object
    if isinstance(parsed, list):
        parsed = parsed[-1] if parsed else {}

    # Claude result envelope
    if isinstance(parsed, dict) and parsed.get("type") == "result":
        inner = parsed.get("result", "")
        if isinstance(inner, str):
            ip = extract_json(inner)
            if ip and "route" in ip:
                return {"route": str(ip["route"]), "content": str(ip.get("content", ""))}
            return {"route": "", "content": inner}
        return {"route": "", "content": json.dumps(parsed)}

    # Direct JSON
    if parsed and "route" in parsed:
        return {"route": str(parsed["route"]), "content": str(parsed.get("content", ""))}

    # Alternative keys
    if parsed:
        for k in ("status", "verdict", "result", "output", "decision"):
            if k in parsed:
                return {"route": str(parsed[k]), "content": json.dumps(parsed)}
        return {"route": "", "content": json.dumps(parsed)}

    # OpenCode NDJSON
    if '"type":"text"' in stdout or '"type":"step_start"' in stdout:
        last_text = ""
        for line in stdout.splitlines():
            try:
                ev = json.loads(line.strip())
            except json.JSONDecodeError:
                continue
            if ev.get("type") == "text":
                t = ev.get("part", {}).get("text", "")
                if t:
                    last_text = t
        if last_text:
            inner = extract_json(last_text)
            if inner and "route" in inner:
                return {"route": str(inner["route"]), "content": str(inner.get("content", ""))}
            return {"route": "", "content": last_text}

    return {"route": "", "content": stdout.strip()}


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

    try:
        proc = run_cmd(cmd_str)
    except FileNotFoundError as e:
        sys.stdout.write(json.dumps({"route": "error", "content": f"not found: {e}"}))
        sys.exit(1)

    stdout_text = stream_stdout_and_stderr(proc, args.timeout)
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    output = parse_text_output(stdout_text)
    sys.stdout.write(json.dumps(output, ensure_ascii=False))


if __name__ == "__main__":
    main()
