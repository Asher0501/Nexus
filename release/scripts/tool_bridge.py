"""
Tool Bridge — MCP tool discovery + built-in shell command execution.

Following Claude Code's architecture:
  - MCP servers provide domain tools (filesystem, github, search, ...)
  - Built-in execute_command provides shell-level terminal access
  - Single unified tool list presented to the LLM
  - Tool calls auto-routed to the correct backend

Config:
  - NEXUS_MCP_CONFIG env var → path to MCP config JSON
  - Default: ~/.nexus/mcp.json

MCP config format (compatible with Claude Code's mcp.json):
  {
    "mcpServers": {
      "filesystem": {
        "command": "npx",
        "args": ["-y", "@anthropic/mcp-server-filesystem", "/path"]
      }
    }
  }
"""

import json
import os
import uuid
import subprocess
import sys
import time
from pathlib import Path


# ── Built-in tools (always available, no MCP required) ──────────

BUILTIN_TOOLS = [
    {
        "name": "read_file",
        "description": "Read the contents of a file. If the path is a directory, lists its contents.",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Path to the file or directory"}
            },
            "required": ["path"],
        },
    },
    {
        "name": "write_file",
        "description": "Write content to a file, overwriting existing content. Creates a .bak backup automatically before overwriting.",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Path to write the file"},
                "content": {"type": "string", "description": "Complete file content to write"},
            },
            "required": ["path", "content"],
        },
    },
    {
        "name": "execute_command",
        "description": "Run a shell command. Use for: listing directories (dir/ls), searching (grep/findstr), running scripts, installing packages, etc. For reading/writing individual files, prefer read_file/write_file.",
        "input_schema": {
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "The shell command to execute."},
                "requires_approval": {"type": "boolean", "default": False},
            },
            "required": ["command"],
        },
    },
    {
        "name": "ask_human",
        "description": "Ask a human for input when you are uncertain or need clarification. Use this for architectural decisions, trade-off calls, or when you need domain knowledge you lack. The human will see your question and options, then respond.",
        "input_schema": {
            "type": "object",
            "properties": {
                "question": {"type": "string", "description": "The question to ask. Be specific."},
                "context": {"type": "string", "description": "Brief context: why you need input."},
                "options": {"type": "array", "items": {"type": "string"}, "description": "Suggested options for the human to choose from."},
            },
            "required": ["question"],
        },
    },
]

# ── Human-in-the-loop: HTTP memory pool ────────────────────

POOL_PORT = int(os.environ.get("NEXUS_HUMAN_PORT", "19876"))
POOL_URL = f"http://127.0.0.1:{POOL_PORT}"


def _ensure_pool_server():
    """Start the HTTP pool server if not already running.
    Spawns humansrv.py as a subprocess to avoid import path issues."""
    import urllib.request as _ur
    try:
        _ur.urlopen(f"{POOL_URL}/health", timeout=1)
        return  # already running
    except Exception:
        pass

    # Find humansrv.py — try locations relative to this script
    script_dir = os.path.dirname(os.path.abspath(__file__)) if "__file__" in dir() else os.getcwd()
    candidates = [
        os.path.join(script_dir, "humansrv.py"),
        os.path.join(os.getcwd(), "humansrv.py"),
        os.path.join(os.getcwd(), "scripts", "humansrv.py"),
    ]
    server_script = None
    for c in candidates:
        if os.path.isfile(c):
            server_script = c
            break
    if not server_script:
        # Try import as last resort
        try:
            from humansrv import start_server as _start
            _start(POOL_PORT)
            for _ in range(20):
                try:
                    _ur.urlopen(f"{POOL_URL}/health", timeout=0.2)
                    return
                except Exception:
                    time.sleep(0.1)
        except ImportError:
            pass
        sys.stderr.write("[tool_bridge] humansrv.py not found\n")
        return

    # Spawn as subprocess (daemon, survives wrapper)
    subprocess.Popen(
        [sys.executable, server_script, str(POOL_PORT)],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    for _ in range(20):
        try:
            _ur.urlopen(f"{POOL_URL}/health", timeout=0.2)
            return
        except Exception:
            time.sleep(0.1)
    sys.stderr.write("[tool_bridge] pool server failed to start\n")


# ── Human-in-the-loop: file-based fallback ─────────────────

HUMAN_POLL_INTERVAL = 0.2


def _human_dir() -> str:
    return os.environ.get("NEXUS_HUMAN_DIR", os.path.join(os.getcwd(), "tmp", "human_io"))


def _run_ask_human(tool_input: dict) -> str:
    """Ask a human via the in-memory HTTP pool. Blocks until answered."""
    try:
        import urllib.request as _ur
        qid = str(uuid.uuid4())
        question = tool_input.get("question", "")
        options = tool_input.get("options", [])
        context = tool_input.get("context", "")

        # 1. Ensure pool server is running (first wrapper starts it)
        _ensure_pool_server()

        # 2. Post question to pool
        body = json.dumps({"question": question, "options": options, "context": context}).encode()
        req = _ur.Request(f"{POOL_URL}/q", data=body, method="POST")
        req.add_header("Content-Type", "application/json")
        resp = _ur.urlopen(req)
        data = json.loads(resp.read().decode())
        qid = data["qid"]

        # 3. Signal via stderr for Dashboard/CLI display
        sys.stderr.write(f"[HUMAN_QUESTION]{json.dumps({'id': qid, 'question': question, 'options': options, 'context': context}, ensure_ascii=True)}\n")
        sys.stderr.flush()

        # 4. Block until answer (server-side threading.Event)
        resp = _ur.urlopen(f"{POOL_URL}/a/{qid}")
        data = json.loads(resp.read().decode())
        answer = data.get("answer", "")

        sys.stderr.write(f"[HUMAN_ANSWERED]{json.dumps({'id': qid})}\n")
        sys.stderr.flush()
        return answer

    except Exception as e:
        sys.stderr.write(f"[HUMAN_ERROR] {e}\n")
        sys.stderr.flush()
        # Fallback to file-based if pool fails
        return _run_ask_human_file(tool_input)


def _acquire_tty_lock(lock_path: Path) -> bool:
    """Try to acquire the terminal lock. Returns True if acquired."""
    try:
        # Atomic: create only if doesn't exist (O_CREAT | O_EXCL)
        fd = os.open(str(lock_path), os.O_CREAT | os.O_EXCL | os.O_WRONLY)
        os.close(fd)
        return True
    except (OSError, FileExistsError):
        return False


def _try_terminal_input(question: str, options: list, context: str) -> str | None:
    """Try to read answer directly from terminal (bypasses engine's stdin pipe).
    Returns answer on success, None if terminal unavailable.
    Claude Code style — engine closed the pipe, we can open the real terminal.
    """
    try:
        tty = open("CONIN$" if sys.platform == "win32" else "/dev/tty",
                    "r", encoding="utf-8", errors="replace")
    except (OSError, IOError):
        return None

    try:
        sys.stderr.write("\n" + "=" * 54 + "\n")
        if context:
            sys.stderr.write(f"Context: {context}\n")
        sys.stderr.write(f"Question: {question}\n")
        if options:
            for i, o in enumerate(options):
                sys.stderr.write(f"  [{i + 1}] {o}\n")
            sys.stderr.write("Enter number or type answer: ")
        else:
            sys.stderr.write("Your answer: ")
        sys.stderr.flush()

        line = tty.readline()
        if not line:
            return None
        answer = line.strip()
        if answer.isdigit() and options:
            idx = int(answer) - 1
            if 0 <= idx < len(options):
                answer = options[idx]
        sys.stderr.write(f"Received: {answer}\n" + "=" * 54 + "\n")
        sys.stderr.flush()
        return answer
    except Exception:
        return None
    finally:
        try: tty.close()
        except Exception: pass


def _run_ask_human_file(tool_input: dict) -> str:
    """File-based fallback when HTTP pool is unavailable."""
    qid = str(uuid.uuid4())
    question = tool_input.get("question", "")
    options = tool_input.get("options", [])
    context = tool_input.get("context", "")

    q_dir = Path(_human_dir()) / qid
    q_dir.mkdir(parents=True, exist_ok=True)
    (q_dir / "question.json").write_text(json.dumps({
        "id": qid, "question": question, "options": options,
        "context": context, "status": "waiting", "ts": time.time(),
    }), encoding="utf-8")

    payload = json.dumps({"id": qid, "question": question,
                          "options": options, "context": context}, ensure_ascii=True)
    sys.stderr.write(f"[HUMAN_QUESTION]{payload}\n")
    sys.stderr.flush()

    answer_file = q_dir / "answer.json"
    deadline = time.time() + 86400
    while time.time() < deadline:
        if answer_file.exists():
            try:
                ans = json.loads(answer_file.read_text(encoding="utf-8"))
                answer = ans.get("answer", str(ans))
                sys.stderr.write(f"[HUMAN_ANSWERED]{json.dumps({'id': qid})}\n")
                sys.stderr.flush()
                answer_file.unlink(missing_ok=True)
                (q_dir / "question.json").unlink(missing_ok=True)
                try: q_dir.rmdir()
                except OSError: pass
                return answer
            except (json.JSONDecodeError, OSError):
                pass
        time.sleep(HUMAN_POLL_INTERVAL)

    sys.stderr.write(f"[HUMAN_TIMEOUT]{json.dumps({'id': qid})}\n")
    sys.stderr.flush()
    (q_dir / "question.json").unlink(missing_ok=True)
    return "ERROR: No human response received within time limit."


def _read_file(path: str) -> str:
    """Read a file or list a directory.  Returns content string."""
    if os.path.isdir(path):
        entries = os.listdir(path)
        dirs = [f"{e}/" for e in entries if os.path.isdir(os.path.join(path, e))]
        files = [e for e in entries if not os.path.isdir(os.path.join(path, e))]
        listing = "\n".join(sorted(dirs) + sorted(files))
        return f"Directory '{path}' ({len(entries)} entries):\n{listing}"
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            content = f.read()
        return f"File '{path}' ({len(content)} chars):\n\n{content}"
    except FileNotFoundError:
        return f"ERROR: File '{path}' not found"
    except Exception as e:
        return f"ERROR reading '{path}': {e}"


def _write_file(path: str, content: str) -> str:
    """Write content to a file with .bak backup.  Returns result string."""
    try:
        if os.path.exists(path):
            bak = path + ".bak"
            if os.path.exists(bak):
                os.remove(bak)
            os.rename(path, bak)
        os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(content)
        return f"File '{path}' written successfully ({len(content)} chars)"
    except Exception as e:
        return f"ERROR writing '{path}': {e}"


def _run_read_file(tool_input: dict) -> str:
    return _read_file(tool_input.get("path", ""))


def _run_write_file(tool_input: dict) -> str:
    return _write_file(tool_input.get("path", ""), tool_input.get("content", ""))


def _run_shell(tool_input: dict) -> str:
    """Execute a shell command and return its output."""
    cmd = tool_input.get("command", "")
    if not cmd:
        return "ERROR: no command provided"

    try:
        result = subprocess.run(
            cmd,
            shell=True,
            capture_output=True,
            text=True,
            errors="replace",  # tolerate bytes outside the system code page
            timeout=300,  # 5 min max
            cwd=os.getcwd(),
        )
        out = result.stdout
        if result.stderr:
            out += f"\n[stderr]\n{result.stderr}"
        if result.returncode != 0:
            out += f"\n[exit code: {result.returncode}]"
        return out or "(no output)"
    except subprocess.TimeoutExpired:
        return "ERROR: command timed out (300s)"
    except Exception as e:
        return f"ERROR: {e}"


# ── MCP config loading ───────────────────────────────────────────

def _resolve_mcp_config_path() -> str | None:
    """Find MCP config file.  Env var takes priority."""
    env_path = os.environ.get("NEXUS_MCP_CONFIG")
    if env_path:
        p = Path(env_path)
        return str(p) if p.exists() else None
    default = Path.home() / ".nexus" / "mcp.json"
    return str(default) if default.exists() else None


def load_mcp_config() -> dict:
    """Load MCP server configuration.  Returns empty dict if no config."""
    path = _resolve_mcp_config_path()
    if not path:
        return {}
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return {}


def _parse_server_config(server_cfg: dict) -> tuple[str, list[str], dict[str, str]]:
    """Extract command, args, env from an MCP server config entry."""
    cmd = server_cfg.get("command", "")
    args = server_cfg.get("args", [])
    env = server_cfg.get("env", {})
    # Merge server env with current process env
    merged_env = os.environ.copy()
    merged_env.update(env)
    return cmd, args, merged_env


# ── MCP tool schema conversion ───────────────────────────────────

def _mcp_tool_to_anthropic(server_name: str, mcp_tool: dict) -> dict:
    """Convert MCP tool schema to Anthropic tool-use schema.

    MCP format:
      {"name": "read", "description": "...", "inputSchema": {"type": "object", ...}}

    Anthropic format:
      {"name": "read", "description": "...", "input_schema": {"type": "object", ...}}

    The formats are almost identical — both use JSON Schema for inputs.
    Main difference: MCP uses "inputSchema" (camelCase), Anthropic uses "input_schema" (snake_case).
    """
    input_schema = mcp_tool.get("inputSchema", {"type": "object", "properties": {}, "required": []})
    return {
        "name": mcp_tool.get("name", "unknown"),
        "description": mcp_tool.get("description", ""),
        "input_schema": input_schema,
    }


# ── Tool discovery (sync wrapper around async MCP) ──────────────

def _discover_mcp_tools_sync(server_name: str, cmd: str, args: list[str], env: dict[str, str]) -> list[dict]:
    """Spawn MCP server, communicate via JSON-RPC stdio, run tools/list.

    Uses communicate() to send all messages at once, avoiding
    line-by-line pipe buffering issues on Windows.
    """
    try:
        proc = subprocess.Popen(
            [cmd] + args,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=env,
        )
    except FileNotFoundError:
        sys.stderr.write(f"[tool_bridge] MCP server '{server_name}': command not found: {cmd}\n")
        return []
    except Exception as e:
        sys.stderr.write(f"[tool_bridge] MCP server '{server_name}': spawn failed: {e}\n")
        return []

    try:
        # Build all requests: initialize + notified + tools/list
        input_data = (
            json.dumps({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "nexus-llm-sdk", "version": "0.1.0"},
                },
            }) + "\n"
            + json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n"
            + json.dumps({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}) + "\n"
        )
        stdout, _stderr = proc.communicate(input=input_data, timeout=10)

        # Parse responses: each line is a JSON-RPC response
        for line in stdout.strip().split("\n"):
            line = line.strip()
            if not line:
                continue
            try:
                resp = json.loads(line)
                if resp.get("id") == 2:  # tools/list response
                    tools = resp.get("result", {}).get("tools", [])
                    sys.stderr.write(f"[tool_bridge] MCP '{server_name}': {len(tools)} tools discovered\n")
                    return tools
            except json.JSONDecodeError:
                continue

        sys.stderr.write(f"[tool_bridge] MCP '{server_name}': no tools/list response\n")
        return []

    except subprocess.TimeoutExpired:
        proc.kill()
        sys.stderr.write(f"[tool_bridge] MCP '{server_name}': timeout\n")
        return []
    except Exception as e:
        sys.stderr.write(f"[tool_bridge] MCP '{server_name}': tools/list failed: {e}\n")
        return []
    finally:
        try:
            proc.wait(timeout=2)
        except Exception:
            proc.kill()


def _call_mcp_tool_sync(server_name: str, cmd: str, args: list[str], env: dict[str, str],
                         tool_name: str, tool_input: dict) -> str:
    """Call an MCP tool via JSON-RPC stdio using communicate()."""
    try:
        proc = subprocess.Popen(
            [cmd] + args,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=env,
        )
    except Exception as e:
        return f"ERROR spawning MCP server '{server_name}': {e}"

    try:
        input_data = (
            json.dumps({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "nexus-llm-sdk", "version": "0.1.0"},
                },
            }) + "\n"
            + json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n"
            + json.dumps({
                "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": {"name": tool_name, "arguments": tool_input},
            }) + "\n"
        )
        stdout, _stderr = proc.communicate(input=input_data, timeout=30)

        # Parse the tools/call response (last JSON-RPC response)
        for line in reversed(stdout.strip().split("\n")):
            line = line.strip()
            if not line:
                continue
            try:
                resp = json.loads(line)
                if resp.get("id") == 3:  # tools/call response
                    if "error" in resp:
                        return f"ERROR: MCP '{server_name}': {resp['error']}"
                    content = resp.get("result", {}).get("content", [])
                    texts = [c.get("text", str(c)) for c in content if isinstance(c, dict)]
                    return "\n".join(texts)
            except json.JSONDecodeError:
                continue

        return f"ERROR: MCP '{server_name}': no tools/call response"

    except subprocess.TimeoutExpired:
        proc.kill()
        return f"ERROR: MCP '{server_name}': timeout"
    except Exception as e:
        return f"ERROR: MCP '{server_name}' tools/call failed: {e}"
    finally:
        try:
            proc.wait(timeout=5)
        except Exception:
            proc.kill()


# ── Tool Bridge (unified interface) ─────────────────────────────

class ToolBridge:
    """Unified tool discovery and execution bridge.

    Discovers tools from:
      1. Built-in: execute_command (shell access)
      2. MCP servers: configured via ~/.nexus/mcp.json

    Routes tool calls to the correct backend transparently.
    """

    def __init__(self):
        self._mcp_config = load_mcp_config()
        self._mcp_server_registry: dict[str, dict] = {}  # tool_name → server info
        self._tools: list[dict] = []

    def discover(self):
        """Discover all available tools.  Call once at startup."""
        self._tools = list(BUILTIN_TOOLS)
        self._mcp_server_registry = {}

        servers = self._mcp_config.get("mcpServers", {})
        for server_name, server_cfg in servers.items():
            cmd, args, env = _parse_server_config(server_cfg)
            sys.stderr.write(f"[tool_bridge] discovering MCP '{server_name}' ({cmd} {' '.join(args)})\n")
            mcp_tools = _discover_mcp_tools_sync(server_name, cmd, args, env)
            for mt in mcp_tools:
                tool = _mcp_tool_to_anthropic(server_name, mt)
                tool_name = tool["name"]
                self._tools.append(tool)
                self._mcp_server_registry[tool_name] = {
                    "server_name": server_name,
                    "cmd": cmd,
                    "args": args,
                    "env": env,
                }

        builtin_count = len(BUILTIN_TOOLS)
        sys.stderr.write(f"[tool_bridge] total tools: {len(self._tools)} "
                         f"({builtin_count} builtin + {len(self._tools) - builtin_count} MCP)\n")

    @property
    def tools(self) -> list[dict]:
        """All discovered tools in Anthropic schema format."""
        return self._tools

    def execute(self, tool_name: str, tool_input: dict) -> str:
        """Execute a tool call.  Routes to built-in or MCP backend."""
        # Built-in tools
        if tool_name == "read_file":
            return _run_read_file(tool_input)
        if tool_name == "write_file":
            return _run_write_file(tool_input)
        if tool_name == "execute_command":
            return _run_shell(tool_input)
        if tool_name == "ask_human":
            return _run_ask_human(tool_input)

        # MCP tool
        server_info = self._mcp_server_registry.get(tool_name)
        if server_info:
            return _call_mcp_tool_sync(**server_info, tool_name=tool_name, tool_input=tool_input)

        return f"ERROR: unknown tool '{tool_name}'"


# ── Tests ───────────────────────────────────────────────────────

if __name__ == "__main__":
    # Quick smoke test
    bridge = ToolBridge()
    bridge.discover()
    for t in bridge.tools:
        print(f"  [{t['name']}] {t.get('description', '')[:80]}")
    # Test read_file on a directory
    print()
    print("read_file on current dir:", bridge.execute("read_file", {"path": "."})[:200])
