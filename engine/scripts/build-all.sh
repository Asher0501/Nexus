#!/bin/bash
# Build all Nexus binaries for multiple platforms
set -euo pipefail

echo "=== Building Nexus ==="

# Build engine (always)
echo "--- Building engine ---"
cargo build --release -p nexus-engine

# Build CLI
echo "--- Building CLI ---"
cargo build --release -p nexus-cli

# Build MCP server (if it exists)
if [ -f "nexus-mcp-server/Cargo.toml" ]; then
    echo "--- Building MCP server ---"
    cargo build --release -p nexus-mcp-server
fi

# Run tests
echo "--- Running tests ---"
cargo test --workspace

echo "=== Build complete ==="
echo "Binaries:"
echo "  target/release/nexus-cli(.exe)"
echo "  target/release/nexus-mcp-server(.exe) (if built)"
echo "  target/release/libnexus_engine.(rlib|a)"
