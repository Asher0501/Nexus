<#
.SYNOPSIS
    Nexus CLI 端到端测试
#>

$ErrorActionPreference = "Continue"
$nexus = ".\target\debug\nexus-cli.exe"
$script:passed = 0
$script:failed = 0

function Pass { $script:passed++; Write-Host "  PASS $($args -join ' ')" -ForegroundColor Green }
function Fail { $script:failed++; Write-Host "  FAIL $($args -join ' ')" -ForegroundColor Red }
function Info { Write-Host "  INFO $($args -join ' ')" -ForegroundColor Cyan }
function Skip { Write-Host "  SKIP $($args -join ' ')" -ForegroundColor Yellow }

$testDir = Join-Path $env:TEMP "nexus-e2e-$(Get-Random)"
New-Item -ItemType Directory -Path $testDir -Force | Out-Null

Write-Host "=== Nexus E2E Tests ===" -ForegroundColor Magenta
Write-Host ""

# --- Test 1: Basic single-node workflow ---
Write-Host "[Test 1] Basic single-node workflow" -ForegroundColor Yellow
[System.IO.File]::WriteAllText((Join-Path $testDir "t1.json"), '{"nodes":[{"id":"echo","providers":[{"type":"subprocess","command":"cmd.exe /c echo hello"}],"process_timeout_secs":30,"predecessors":[]}]}', [System.Text.UTF8Encoding]::new($false))
$r = & $nexus run (Join-Path $testDir "t1.json") 2>&1
if ($LASTEXITCODE -eq 0) { Pass "exit 0" } else { Fail "expected 0 got $LASTEXITCODE" }

# --- Test 2: Two-node chain ---
Write-Host "[Test 2] Two-node chain" -ForegroundColor Yellow
[System.IO.File]::WriteAllText((Join-Path $testDir "t2.json"), '{"nodes":[{"id":"a","providers":[{"type":"subprocess","command":"cmd.exe /c echo from_a"}],"process_timeout_secs":10,"predecessors":[]},{"id":"b","providers":[{"type":"subprocess","command":"cmd.exe /c echo from_b"}],"process_timeout_secs":10,"predecessors":[{"node_id":"a","trigger":"all","event":"complete"}]}]}', [System.Text.UTF8Encoding]::new($false))
$r = & $nexus run (Join-Path $testDir "t2.json") 2>&1
if ($LASTEXITCODE -eq 0) { Pass "exit 0" } else { Fail "expected 0 got $LASTEXITCODE" }

# --- Test 3: Validate only ---
Write-Host "[Test 3] Validate only" -ForegroundColor Yellow
$r = & $nexus run (Join-Path $testDir "t2.json") --validate-only 2>&1
if ($LASTEXITCODE -eq 0) { Pass "exit 0" } else { Fail "expected 0 got $LASTEXITCODE" }

# --- Test 4: Invalid path ---
Write-Host "[Test 4] Invalid path" -ForegroundColor Yellow
$r = & $nexus run "nonexistent.json" 2>&1
if ($LASTEXITCODE -eq 1) { Pass "exit 1" } else { Fail "expected 1 got $LASTEXITCODE" }

# --- Test 5: Invalid JSON ---
Write-Host "[Test 5] Invalid JSON" -ForegroundColor Yellow
[System.IO.File]::WriteAllText((Join-Path $testDir "bad.json"), '{bad json}', [System.Text.UTF8Encoding]::new($false))
$r = & $nexus run (Join-Path $testDir "bad.json") 2>&1
if ($LASTEXITCODE -eq 1) { Pass "exit 1" } else { Fail "expected 1 got $LASTEXITCODE" }

# --- Test 6: Empty workflow rejected ---
Write-Host "[Test 6] Empty workflow rejected" -ForegroundColor Yellow
[System.IO.File]::WriteAllText((Join-Path $testDir "empty.json"), '{"nodes":[]}', [System.Text.UTF8Encoding]::new($false))
$r = & $nexus run (Join-Path $testDir "empty.json") 2>&1
if ($LASTEXITCODE -eq 1) { Pass "exit 1 (EmptyGraph)" } else { Fail "expected 1 got $LASTEXITCODE" }

# --- Test 7: Dump state ---
Write-Host "[Test 7] Dump state" -ForegroundColor Yellow
$r = & $nexus run (Join-Path $testDir "t1.json") --dump-state 2>&1
if ($LASTEXITCODE -eq 0) { Pass "exit 0" } else { Fail "expected 0 got $LASTEXITCODE" }

# --- Test 8: Max concurrency ---
Write-Host "[Test 8] Max concurrency" -ForegroundColor Yellow
$r = & $nexus run (Join-Path $testDir "t1.json") --max-concurrency 4 2>&1
if ($LASTEXITCODE -eq 0) { Pass "exit 0" } else { Fail "expected 0 got $LASTEXITCODE" }

# --- Test 9: Node timeout ---
Write-Host "[Test 9] Node timeout" -ForegroundColor Yellow
$r = & $nexus run (Join-Path $testDir "t1.json") --node-timeout 7200 2>&1
if ($LASTEXITCODE -eq 0) { Pass "exit 0" } else { Fail "expected 0 got $LASTEXITCODE" }

# --- Test 10: Multi-node with fan-out ---
Write-Host "[Test 10] Fan-out" -ForegroundColor Yellow
[System.IO.File]::WriteAllText((Join-Path $testDir "fan.json"), '{"nodes":[{"id":"root","providers":[{"type":"subprocess","command":"cmd.exe /c echo root"}],"process_timeout_secs":10,"predecessors":[]},{"id":"c1","providers":[{"type":"subprocess","command":"cmd.exe /c echo child1"}],"process_timeout_secs":10,"predecessors":[{"node_id":"root","trigger":"all","event":"complete"}]},{"id":"c2","providers":[{"type":"subprocess","command":"cmd.exe /c echo child2"}],"process_timeout_secs":10,"predecessors":[{"node_id":"root","trigger":"all","event":"complete"}]}]}', [System.Text.UTF8Encoding]::new($false))
$r = & $nexus run (Join-Path $testDir "fan.json") --max-concurrency 2 2>&1
if ($LASTEXITCODE -eq 0) { Pass "exit 0" } else { Fail "expected 0 got $LASTEXITCODE" }

# --- Test 11: OpenCode node E2E ---
Write-Host "[Test 11] OpenCode node E2E" -ForegroundColor Yellow
[System.IO.File]::WriteAllText((Join-Path $testDir "oc.json"), '{"nodes":[{"id":"config","providers":[{"type":"subprocess","command":"cmd.exe /c echo {\\\"prompt\\\":\\\"Say exactly: HELLO_FROM_NEXUS\\\",\\\"model\\\":\\\"anthropic/claude-sonnet-4-20250514\\\"}"}],"process_timeout_secs":10,"predecessors":[]},{"id":"ai","providers":[{"type":"subprocess","command":"powershell -ExecutionPolicy Bypass -File node-opencode.ps1"}],"process_timeout_secs":120,"predecessors":[{"node_id":"config","trigger":"all","event":"complete"}],"inputs":["config"]}]}', [System.Text.UTF8Encoding]::new($false))
$r = & $nexus run (Join-Path $testDir "oc.json") 2>&1
if ($LASTEXITCODE -eq 0) { Pass "exit 0" } else { Fail "expected 0 got $LASTEXITCODE" }

# --- Summary ---
Write-Host ""
Write-Host "Passed: $script:passed  Failed: $script:failed" -ForegroundColor Cyan
Remove-Item -Recurse -Force $testDir -ErrorAction SilentlyContinue
if ($script:failed -gt 0) { exit 1 } else { exit 0 }
