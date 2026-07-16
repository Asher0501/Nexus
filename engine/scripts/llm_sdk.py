"""
LLM SDK Node — Nexus wrapper for Anthropic Python SDK.

Unlike llm_node.py (which spawns a CLI), this script calls the
Anthropic API directly via `pip install anthropic`.  This enables
token-level streaming, native tool calling, and no CLI dependency.

Protocol layer (stdin, output parsing, route correction) is shared
in nexus_protocol.py.  This file only contains SDK-specific logic:
credential loading, tool definitions, API calls.
"""

import json
import os
import sys
import time

from nexus_protocol import log, parse_output, read_context, sanitize, try_route_correction, write_output
from tool_bridge import ToolBridge

try:
    import anthropic
except ImportError:
    write_output({
        "route": "error",
        "content": "anthropic package not installed. Run: pip install anthropic",
    })
    sys.exit(1)

# ═══════════════════════════════════════════════════════════════════
# Credential loading
# ═══════════════════════════════════════════════════════════════════

def _load_settings_env() -> dict:
    """Read env vars from Claude Code settings.json as fallback.
    Also injects them into os.environ so the Anthropic SDK picks up
    ANTHROPIC_BASE_URL etc. automatically."""
    settings_path = os.path.join(os.path.expanduser("~"), ".claude", "settings.json")
    try:
        with open(settings_path) as f:
            env_vars = json.load(f).get("env", {})
        for k, v in env_vars.items():
            if k not in os.environ and v:
                os.environ[k] = v
        return env_vars
    except Exception:
        return {}


def load_credentials(ctx: dict) -> dict:
    """Resolve API credentials in priority order."""
    exts = ctx.get("extensions", {})
    api_key_env = exts.get("_sdk_api_key_env", "ANTHROPIC_API_KEY")
    api_key = os.environ.get(api_key_env)
    if not api_key:
        api_key = os.environ.get("ANTHROPIC_AUTH_TOKEN")
    if not api_key:
        settings_env = _load_settings_env()
        api_key = (settings_env.get(api_key_env) or
                   settings_env.get("ANTHROPIC_AUTH_TOKEN"))
    if not api_key:
        raise ValueError(
            f"API key not found in env var '{api_key_env}', "
            f"ANTHROPIC_AUTH_TOKEN, or ~/.claude/settings.json. "
            f"Set one of them or configure _sdk_api_key_env."
        )
    max_tokens = exts.get("_sdk_max_tokens", "4096")
    return {
        "api_key": api_key,
        "model": exts.get("_sdk_model", "claude-sonnet-5-20251001"),
        "system": exts.get("_sdk_system_prompt", None),
        "max_tokens": int(max_tokens),
    }


# ═══════════════════════════════════════════════════════════════════
# Tool Bridge (MCP + built-in shell, unified interface)
# ═══════════════════════════════════════════════════════════════════

_bridge: ToolBridge | None = None

def get_bridge() -> ToolBridge:
    global _bridge
    if _bridge is None:
        _bridge = ToolBridge()
        _bridge.discover()
    return _bridge


# ═══════════════════════════════════════════════════════════════════
# Anthropic SDK call with tool-use loop (tools from ToolBridge)
# ═══════════════════════════════════════════════════════════════════

def call_api(sdk_cfg: dict, prompt: str) -> str:
    """Call Anthropic API.  Runs a tool-use loop (max 20 turns) so the
    LLM can read/write/append files before producing its final answer."""
    bridge = get_bridge()
    tools = bridge.tools

    client = anthropic.Anthropic(api_key=sdk_cfg["api_key"])
    system = sdk_cfg.get("system")
    messages = [{"role": "user", "content": prompt}]

    log(f"[llm_sdk] model={sdk_cfg['model']} max_tokens={sdk_cfg['max_tokens']} tools={len(tools)}")
    start_time = time.time()
    total_tokens = 0

    for _turn in range(20):
        kwargs = dict(
            model=sdk_cfg["model"],
            max_tokens=sdk_cfg["max_tokens"],
            messages=messages,
            tools=tools,
        )
        if system:
            kwargs["system"] = system

        try:
            response = client.messages.create(**kwargs)
        except anthropic.APIError as e:
            elapsed = time.time() - start_time
            log(f"[llm_sdk] API error after {elapsed:.1f}s: {e}")
            return json.dumps({"route": "error", "content": f"API error: {e}"})

        total_tokens += (response.usage.output_tokens
                         if hasattr(response, 'usage') else 0)

        if response.stop_reason == "tool_use":
            tool_results = []
            assistant_content = []
            for block in response.content:
                if block.type == "tool_use":
                    tool_input = (block.input if isinstance(block.input, dict)
                                  else {})
                    # Show a short description of the tool input (first non-empty value)
                    input_desc = next((f"{k}={str(v)[:40]}" for k, v in tool_input.items() if v), '?')
                    log(f"[llm_sdk] tool_call: {block.name}({input_desc})")
                    result = bridge.execute(block.name, tool_input)
                    tool_results.append({
                        "type": "tool_result",
                        "tool_use_id": block.id,
                        "content": result,
                    })
                assistant_content.append(block)

            messages.append({"role": "assistant", "content": assistant_content})
            messages.append({"role": "user", "content": tool_results})
            continue

        # stop_reason == "end_turn" — extract text
        text_parts = []
        final_content = []
        for block in response.content:
            if block.type == "text":
                text_parts.append(block.text)
                log(block.text)  # stream to engine
            final_content.append(block)

        messages.append({"role": "assistant", "content": final_content})
        elapsed = time.time() - start_time
        full = "".join(text_parts)
        log(f"[llm_sdk] {total_tokens} tokens in {elapsed:.1f}s")
        return full

    elapsed = time.time() - start_time
    log(f"[llm_sdk] max tool turns reached after {elapsed:.1f}s")
    return json.dumps({"route": "error", "content": "Max tool-use turns exceeded (20)"})


# ═══════════════════════════════════════════════════════════════════
# Main
# ═══════════════════════════════════════════════════════════════════

def main():
    ctx = read_context()

    try:
        sdk_cfg = load_credentials(ctx)
    except ValueError as e:
        write_output({"route": "error", "content": str(e)})
        sys.exit(1)

    # Sanitize system_prompt — Chinese chars in workflow JSON can
    # produce lone surrogates via serde_json escapes on Windows.
    if sdk_cfg.get("system"):
        sdk_cfg["system"] = sanitize(sdk_cfg["system"])

    # Build prompt from extensions or inputs
    prompt = ctx.get("extensions", {}).get("prompt", "")
    if not prompt:
        inputs = ctx.get("inputs", {})
        if inputs:
            prompt = "\n".join(f"[{k}]: {v}" for k, v in inputs.items())
        else:
            prompt = "No input provided."

    # Append timeout context if previous run timed out
    meta = ctx.get("metadata", {})
    if meta.get("timed_out"):
        prompt += (
            "\n\nIMPORTANT: The previous execution of this node timed out. "
            "Adjust your response accordingly."
        )

    expected_routes = ctx.get("extensions", {}).get("route", "")
    prompt = sanitize(prompt)

    def _run(p: str) -> str:
        return call_api(sdk_cfg, p)

    try:
        response = _run(prompt)
    except Exception as e:
        log(f"[llm_sdk] fatal: {e}")
        write_output({"route": "error", "content": sanitize(str(e))})
        sys.exit(1)

    output = parse_output(response)
    output = try_route_correction(output, expected_routes, _run)
    write_output(output)


if __name__ == "__main__":
    main()
