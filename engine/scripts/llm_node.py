"""
LLM Node — Nexus wrapper for any LLM CLI (claude, opencode, nga, codeagent, ...).

Protocol layer (stdin, output parsing, route correction) is shared
in nexus_protocol.py.  This file only contains CLI-specific logic:
binary resolution, subprocess spawning, real-time streaming.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import threading
import time

from nexus_protocol import log, parse_output, read_context, try_route_correction, write_output

# ═══════════════════════════════════════════════════════════════════
# CLI resolution — cross-platform (Windows + Linux)
# ═══════════════════════════════════════════════════════════════════

def resolve_program(name: str) -> str:
    """Resolve CLI name to executable path.  Prefer .exe on Windows."""
    if sys.platform == "win32":
        exe = shutil.which(name + ".exe")
        if exe:
            return exe
    return shutil.which(name) or name


# ═══════════════════════════════════════════════════════════════════
# Subprocess execution
# ═══════════════════════════════════════════════════════════════════

def spawn(cmd_str: str, stdin_text: str | None = None) -> subprocess.Popen:
    """Spawn CLI process.  Prompt goes via stdin (not -p), avoiding
    command-line length limits on Windows."""
    program = cmd_str.split(" ", 1)[0]
    exe = resolve_program(program)
    if exe != program:
        cmd_str = exe + cmd_str[len(program):]

    real_exe = exe
    if sys.platform == "win32" and exe.lower().endswith(".cmd"):
        # .CMD wrappers add an extra cmd.exe layer that kills streaming.
        # Prefer the real node binary when available (Claude Code layout).
        node_path = os.path.join(os.path.dirname(exe), "..", "dist", "claude.js")
        if os.path.isfile(node_path):
            real_exe = "node"
            cmd_str = f'node "{node_path}" {cmd_str[len(program):].lstrip()}'
    elif sys.platform == "win32":
        # Try .exe next to .cmd (some npm packages ship both)
        exe_path = os.path.splitext(exe)[0] + ".exe"
        if os.path.isfile(exe_path):
            real_exe = exe_path
            cmd_str = real_exe + cmd_str[len(program):]

    log(f"[llm_node] {os.path.basename(real_exe)}")
    proc = subprocess.Popen(
        cmd_str,
        stdin=subprocess.PIPE if stdin_text else subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,       # line-buffered — critical for real-time streaming
        shell=False,
    )
    if stdin_text and proc.stdin:
        proc.stdin.write(stdin_text)
        proc.stdin.close()
    return proc


def stream(proc: subprocess.Popen, timeout: int = 0) -> str:
    """Read stdout line-by-line, forward each line to stderr (engine log).
    Returns complete stdout text for final parsing.  Sends periodic
    heartbeats to flush pipe buffers."""
    deadline = time.time() + timeout if timeout > 0 else None
    stdout_lines: list[str] = []
    stderr_done = threading.Event()
    heartbeat_stop = threading.Event()

    def _read_stderr():
        try:
            for line in proc.stderr:
                line = line.rstrip("\n\r")
                if line:
                    log(line)
        except Exception:
            pass
        finally:
            stderr_done.set()

    def _heartbeat():
        while not heartbeat_stop.is_set():
            time.sleep(0.3)
            if not heartbeat_stop.is_set():
                log(" ")

    threading.Thread(target=_read_stderr, daemon=True).start()
    threading.Thread(target=_heartbeat, daemon=True).start()

    try:
        for line in proc.stdout:
            line = line.rstrip("\n\r")
            if line:
                stdout_lines.append(line)
                log(line)
            if deadline and time.time() > deadline:
                proc.kill()
                log("[llm_node] timeout")
                break
    except Exception:
        pass
    finally:
        heartbeat_stop.set()

    stderr_done.wait(timeout=5)
    return "\n".join(stdout_lines)


def wait(proc: subprocess.Popen):
    """Wait for process with a 5s grace period, kill if stuck."""
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()


# ═══════════════════════════════════════════════════════════════════
# Main
# ═══════════════════════════════════════════════════════════════════

def main():
    p = argparse.ArgumentParser(description="Nexus universal LLM node")
    p.add_argument("--cmd", default="", help="Full rendered CLI command")
    p.add_argument("--timeout", type=int, default=0,
                   help="Internal timeout seconds (0 = rely on engine)")
    args = p.parse_args()

    ctx = read_context()

    cmd_str = args.cmd or ctx.get("extensions", {}).get("_cmd", "")
    if not cmd_str:
        write_output({"route": "error", "content": "no --cmd provided"})
        sys.exit(1)

    prompt = ctx.get("extensions", {}).get("prompt", "")

    # Backward compat: if command has {{prompt}}, inject it there.
    # Otherwise pass prompt via stdin (avoids command-line length limits).
    if prompt and "{{prompt}}" in cmd_str:
        safe = prompt.replace("\\", "\\\\").replace('"', '\\"')
        cmd_str = cmd_str.replace("{{prompt}}", safe)
        use_stdin = False
    else:
        use_stdin = bool(prompt)

    expected_routes = ctx.get("extensions", {}).get("route", "")

    def _run(prompt_text: str) -> str:
        proc = spawn(cmd_str, stdin_text=prompt_text)
        text = stream(proc, args.timeout)
        wait(proc)
        return text

    try:
        stdout_text = _run(prompt if use_stdin else None)
    except FileNotFoundError as e:
        write_output({"route": "error", "content": f"not found: {e}"})
        sys.exit(1)

    log(f"[llm_node] captured {len(stdout_text)} chars from stdout")
    output = parse_output(stdout_text)
    output = try_route_correction(output, expected_routes, _run)
    write_output(output)


if __name__ == "__main__":
    main()
