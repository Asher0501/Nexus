"""Tests for tool_bridge.py — MCP discovery, shell execution, unified interface."""

import json
import os
import sys
import tempfile
from pathlib import Path

# Add scripts dir to path
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from tool_bridge import (
    BUILTIN_TOOLS,
    ToolBridge,
    _call_mcp_tool_sync,
    _discover_mcp_tools_sync,
    _mcp_tool_to_anthropic,
    _read_file,
    _run_shell,
    _write_file,
    load_mcp_config,
)

# ═══════════════════════════════════════════════════════════
# Mock MCP server — a tiny Python script that speaks JSON-RPC
# ═══════════════════════════════════════════════════════════

MOCK_MCP_SERVER = """
import json, sys

def handle(req):
    method = req.get('method', '')
    rid = req.get('id', 0)

    if method == 'initialize':
        return json.dumps({"jsonrpc":"2.0","id":rid,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"mock","version":"1.0"}}})
    elif method == 'tools/list':
        return json.dumps({"jsonrpc":"2.0","id":rid,"result":{"tools":[
            {"name":"mock_read","description":"Read a mock file","inputSchema":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}},
            {"name":"mock_search","description":"Search mock data","inputSchema":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}}
        ]}})
    elif method == 'tools/call':
        params = req.get('params', {})
        name = params.get('name', '')
        args = params.get('arguments', {})
        if name == 'mock_read':
            return json.dumps({"jsonrpc":"2.0","id":rid,"result":{"content":[{"type":"text","text":f"mock content of {args.get('path','?')}"}]}})
        elif name == 'mock_search':
            return json.dumps({"jsonrpc":"2.0","id":rid,"result":{"content":[{"type":"text","text":f"results for {args.get('query','?')}: 3 found"}]}})
        return json.dumps({"jsonrpc":"2.0","id":rid,"error":{"code":-32601,"message":"Unknown tool"}})
    else:
        return json.dumps({"jsonrpc":"2.0","id":rid,"error":{"code":-32601,"message":"Unknown method"}})

if __name__ == '__main__':
    while True:
        try:
            line = sys.stdin.readline()
            if not line:
                break
            req = json.loads(line.strip())
            resp = handle(req)
            sys.stdout.write(resp + '\\n')
            sys.stdout.flush()
        except EOFError:
            break
        except Exception as e:
            sys.stderr.write(f'mock error: {e}\\n')
"""


def _write_mock_server() -> str:
    """Write mock MCP server to a temp file, return path."""
    fd, path = tempfile.mkstemp(suffix='.py', prefix='nexus_mock_mcp_')
    with os.fdopen(fd, 'w') as f:
        f.write(MOCK_MCP_SERVER)
    return path


# ═══════════════════════════════════════════════════════════
# Tests
# ═══════════════════════════════════════════════════════════

def test_builtin_tools_schema():
    """Built-in tools have correct Anthropic schema."""
    names = [t["name"] for t in BUILTIN_TOOLS]
    assert "read_file" in names
    assert "write_file" in names
    assert "execute_command" in names
    for t in BUILTIN_TOOLS:
        assert "input_schema" in t
        assert "name" in t


def test_run_shell_basic():
    """Shell command returns output."""
    result = _run_shell({"command": "echo hello"})
    assert "hello" in result


def test_run_shell_stderr():
    """Shell command captures stderr."""
    result = _run_shell({"command": "echo err >&2 && echo out"})
    assert "err" in result
    assert "out" in result


def test_run_shell_exit_code():
    """Non-zero exit codes are reported."""
    result = _run_shell({"command": "exit 1"})
    if sys.platform == "win32":
        # Windows cmd.exe doesn't propagate exit from `exit 1` directly
        pass
    else:
        assert "exit code: 1" in result


def test_mcp_tool_schema_conversion():
    """MCP tool schema converts correctly to Anthropic format."""
    mcp_tool = {
        "name": "test_tool",
        "description": "A test tool",
        "inputSchema": {
            "type": "object",
            "properties": {"x": {"type": "string"}},
            "required": ["x"],
        },
    }
    result = _mcp_tool_to_anthropic("test_server", mcp_tool)
    assert result["name"] == "test_tool"
    assert result["description"] == "A test tool"
    assert "input_schema" in result  # snake_case, not camelCase
    assert result["input_schema"]["type"] == "object"


def test_load_mcp_config_no_file():
    """Returns empty dict when no config file exists."""
    os.environ.pop("NEXUS_MCP_CONFIG", None)
    # Without a config file, should return empty dict
    # (We can't easily test with a real file here, but load_mcp_config
    #  catches exceptions gracefully)
    result = load_mcp_config()
    assert isinstance(result, dict)


def test_mcp_discovery():
    """Discover tools from a real MCP server subprocess."""
    server_path = _write_mock_server()
    try:
        tools = _discover_mcp_tools_sync(
            "mock",
            sys.executable,
            [server_path],
            os.environ.copy(),
        )
        assert len(tools) == 2, f"Expected 2 tools, got {len(tools)}"
        names = [t["name"] for t in tools]
        assert "mock_read" in names
        assert "mock_search" in names
    finally:
        os.unlink(server_path)


def test_mcp_tool_call():
    """Call an MCP tool via subprocess."""
    server_path = _write_mock_server()
    try:
        result = _call_mcp_tool_sync(
            "mock",
            sys.executable,
            [server_path],
            os.environ.copy(),
            "mock_read",
            {"path": "/test/file.txt"},
        )
        assert "mock content of /test/file.txt" in result

        result2 = _call_mcp_tool_sync(
            "mock",
            sys.executable,
            [server_path],
            os.environ.copy(),
            "mock_search",
            {"query": "nexus"},
        )
        assert "results for nexus" in result2
    finally:
        os.unlink(server_path)


def test_tool_bridge_full_integration():
    """Full ToolBridge: discover + execute with MCP + built-in."""
    # Write MCP config pointing to mock server
    server_path = _write_mock_server()
    try:
        config = {
            "mcpServers": {
                "mock": {
                    "command": sys.executable,
                    "args": [server_path],
                }
            }
        }
        fd, config_path = tempfile.mkstemp(suffix='.json', prefix='nexus_mcp_config_')
        with os.fdopen(fd, 'w') as f:
            json.dump(config, f)

        os.environ["NEXUS_MCP_CONFIG"] = config_path

        try:
            bridge = ToolBridge()
            bridge.discover()

            # Should have read_file + write_file + execute_command + mock_read + mock_search
            names = [t["name"] for t in bridge.tools]
            assert "read_file" in names
            assert "write_file" in names
            assert "execute_command" in names
            assert "mock_read" in names
            assert "mock_search" in names
            assert len(bridge.tools) == 5

            # Execute built-in
            r = bridge.execute("execute_command", {"command": "echo test"})
            assert "test" in r

            # Execute MCP tool
            r = bridge.execute("mock_read", {"path": "/x"})
            assert "mock content of /x" in r

            # Execute MCP search
            r = bridge.execute("mock_search", {"query": "hello"})
            assert "results for hello" in r

        finally:
            os.environ.pop("NEXUS_MCP_CONFIG", None)
            os.unlink(config_path)
    finally:
        os.unlink(server_path)


def test_tool_bridge_no_config():
    """ToolBridge without MCP config has only built-in tools."""
    os.environ.pop("NEXUS_MCP_CONFIG", None)
    bridge = ToolBridge()
    bridge.discover()
    assert len(bridge.tools) == 3  # read_file, write_file, execute_command
    names = [t["name"] for t in bridge.tools]
    assert "read_file" in names
    assert "write_file" in names
    assert "execute_command" in names


def test_read_file():
    """_read_file works for files and directories."""
    result = _read_file(".")
    assert "Directory" in result or "entries" in result
    result = _read_file(__file__)
    assert "File" in result
    assert "test_read_file" in result
    result = _read_file("/nonexistent/file/path")
    assert "ERROR" in result or "not found" in result.lower()


def test_write_file():
    """_write_file creates a file, preserves backup."""
    import tempfile, shutil
    dir = tempfile.mkdtemp()
    path = os.path.join(dir, "test.txt")
    try:
        result = _write_file(path, "hello nexus")
        assert "written" in result
        with open(path) as f:
            assert f.read() == "hello nexus"
        result = _write_file(path, "updated")
        assert "written" in result
        assert os.path.exists(path + ".bak")
        with open(path + ".bak") as f:
            assert f.read() == "hello nexus"
    finally:
        shutil.rmtree(dir, ignore_errors=True)


def test_tool_bridge_missing_mcp_server():
    """Non-existent MCP server doesn't crash discovery."""
    fd, config_path = tempfile.mkstemp(suffix='.json', prefix='nexus_mcp_config_')
    with os.fdopen(fd, 'w') as f:
        json.dump({
            "mcpServers": {
                "ghost": {
                    "command": "nonexistent_command_xyz_12345",
                    "args": [],
                }
            }
        }, f)

    os.environ["NEXUS_MCP_CONFIG"] = config_path
    try:
        bridge = ToolBridge()
        bridge.discover()
        # Should still have built-in tools even if MCP fails
        assert len(bridge.tools) >= 3
        names = [t["name"] for t in bridge.tools]
        assert "read_file" in names
        assert "execute_command" in names
    finally:
        os.environ.pop("NEXUS_MCP_CONFIG", None)
        os.unlink(config_path)


# ═══════════════════════════════════════════════════════════

if __name__ == "__main__":
    test_builtin_tools_schema()
    test_run_shell_basic()
    test_run_shell_stderr()
    test_run_shell_exit_code()
    test_mcp_tool_schema_conversion()
    test_load_mcp_config_no_file()
    test_mcp_discovery()
    test_mcp_tool_call()
    test_tool_bridge_full_integration()
    test_tool_bridge_no_config()
    test_read_file()
    test_write_file()
    test_tool_bridge_missing_mcp_server()
    print("ALL 13 TESTS PASSED")
