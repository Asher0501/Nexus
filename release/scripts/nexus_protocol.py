"""
Nexus Protocol — shared utilities for all Nexus node wrappers.

Every node wrapper follows the same protocol:
  stdin  ← NodeContext JSON
  stderr → log / stream chunks (engine captures as real-time output)
  stdout → final line: {"route":"...","content":"..."}

This module provides the common pieces so that each wrapper only
needs to implement its own execution strategy (CLI spawn, SDK call, etc.).
"""

import json
import re
import sys

# ── Surrogate sanitization ──────────────────────────────────
# Lone surrogates (U+D800–U+DFFF) can leak in via Windows console
# API transcoding or serde_json escapes. They are invalid Unicode
# and crash json.dumps / sys.stdout.write.

_SURROGATE_RE = re.compile(r'[\ud800-\udfff]')


def sanitize(s: str) -> str:
    """Replace lone surrogates with U+FFFD."""
    return _SURROGATE_RE.sub('�', s)


# ── stdin / stdout protocol ─────────────────────────────────

def read_context() -> dict:
    """Read Nexus NodeContext JSON from stdin."""
    raw = sys.stdin.read()
    if not raw.strip():
        return {
            "inputs": {},
            "extensions": {},
            "metadata": {"run_count": 1, "timed_out": False},
        }
    return json.loads(raw)


def write_output(output: dict):
    """Sanitize and write the final {route, content} to stdout."""
    output["route"] = sanitize(str(output.get("route", "")))
    output["content"] = sanitize(str(output.get("content", "")))
    sys.stdout.write(json.dumps(output, ensure_ascii=True))


def log(msg: str):
    """Write a line to stderr (engine captures → log chunks)."""
    print(msg, file=sys.stderr, flush=True)


# ── Output parsing ───────────────────────────────────────────
#
# LLM / subprocess output may be messy (code fences, NDJSON, mixed
# text).  We use a multi-stage fallback to find route+content JSON.

# Stage 3 regex: matches "route":"val","content":"escaped-str"
_ROUTE_RE = re.compile(
    r'"?route"?\s*:\s*"?([^",}\s]+)"?\s*,\s*"?content"?\s*:\s*"((?:[^"\\]|\\.)*)"',
    re.DOTALL,
)


def parse_output(text: str) -> dict:
    """Extract {route, content} from arbitrary text.

    Fallback stages:
      1. Try direct JSON parse of entire text.
      2. Scan line-by-line for any valid JSON line with a 'route' key.
      3. Regex match route+content with NDJSON unescaping.
      4. Last-resort regex with unquoted keys.
    """
    stripped = text.strip()

    # Stage 1: entire text is valid JSON with route
    if stripped.startswith("{"):
        try:
            obj = json.loads(stripped)
            if "route" in obj:
                return {"route": str(obj["route"]),
                        "content": str(obj.get("content", ""))}
        except (json.JSONDecodeError, ValueError):
            pass

    # Stage 2: line-by-line scan for any valid JSON with route
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("{") and line.endswith("}"):
            try:
                obj = json.loads(line)
                if "route" in obj:
                    return {"route": str(obj["route"]),
                            "content": str(obj.get("content", ""))}
            except (json.JSONDecodeError, ValueError):
                continue

    # Stage 3: regex with NDJSON unescaping
    unescaped = text.replace('\\"', '"').replace('\\\\', '\\')
    for m in _ROUTE_RE.finditer(unescaped):
        route = m.group(1)
        content_raw = m.group(2)
        try:
            content = json.loads('"' + content_raw + '"')
        except (json.JSONDecodeError, ValueError):
            content = content_raw
        return {"route": route, "content": content}

    # Stage 4: last resort — raw regex on original text
    for m in re.finditer(
        r'\broute\s*:\s*"?(\w+)"?\s*[,;]\s*content\s*:\s*"((?:[^"\\]|\\.)*)"',
        text, re.DOTALL
    ):
        content_raw = m.group(2)
        try:
            content = json.loads('"' + content_raw + '"')
        except (json.JSONDecodeError, ValueError):
            content = content_raw
        return {"route": m.group(1), "content": content}

    return {"route": "", "content": stripped}


# ── Route correction ─────────────────────────────────────────
#
# If the LLM outputs an unexpected route (or no route), we retry
# once with an explicit correction prompt.  The caller provides a
# `run_fn(correction_prompt) -> str` — spawn a subprocess, call
# an API, etc.  The protocol layer doesn't care HOW.

def try_route_correction(
    output: dict,
    expected_routes: str,
    run_fn,  # (prompt: str) -> str
) -> dict:
    """If route is missing or unexpected, retry with correction prompt.

    Returns corrected output dict, or the original if correction
    fails or isn't needed.
    """
    if not expected_routes:
        return output

    expected = [r.strip() for r in expected_routes.split(",") if r.strip()]
    if not expected:
        return output

    route = output.get("route", "")
    if route and route in expected:
        return output  # already valid

    log(f"[nexus] route='{route}' not in {expected}, retrying")
    target = expected[0]
    correction = (
        f'Output EXACTLY this JSON on a single line with no other text: '
        f'{{"route":"{target}","content":"done"}}'
    )
    try:
        response = run_fn(correction)
    except Exception as e:
        log(f"[nexus] correction call failed: {e}")
        return output

    output2 = parse_output(response)
    route2 = output2.get("route", "")
    if route2 and (not expected or route2 in expected):
        log(f"[nexus] correction OK: route={route2}")
        return output2

    log(f"[nexus] correction failed: route={route2}")
    return output
