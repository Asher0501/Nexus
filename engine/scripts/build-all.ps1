param([switch]$Release = $true)
$config = if ($Release) { "release" } else { "debug" }
Write-Host "=== Building Nexus ($config) ==="

Write-Host "--- Building engine ---"
cargo build --$config -p nexus-engine

Write-Host "--- Building CLI ---"
cargo build --$config -p nexus-cli

if (Test-Path "nexus-mcp-server/Cargo.toml") {
    Write-Host "--- Building MCP server ---"
    cargo build --$config -p nexus-mcp-server
}

Write-Host "--- Running tests ---"
cargo test --workspace

Write-Host "=== Build complete ==="
