# 测试 node-opencode.ps1 包装器
# 模拟引擎的行为：写 stdin JSON → 读 stdout

$testDir = Join-Path $env:TEMP "nexus-wrapper-test"
New-Item -ItemType Directory -Path $testDir -Force | Out-Null

# 构造与引擎相同的 NodeContext JSON
$ctx = @{
    inputs = @{
        config = '{"prompt":"Say exactly: HELLO_FROM_NEXUS","model":"anthropic/claude-sonnet-4-20250514"}'
    }
    extensions = @{}
}
$json = $ctx | ConvertTo-Json -Compress

# 写入 stdin 文件（引擎通过 pipe 写入）
$stdinFile = Join-Path $testDir "stdin.json"
[System.IO.File]::WriteAllText($stdinFile, $json, [System.Text.UTF8Encoding]::new($false))

# 运行包装器，stdin 重定向
$stdoutFile = Join-Path $testDir "stdout.txt"
$stderrFile = Join-Path $testDir "stderr.txt"
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = "powershell"
$psi.Arguments = "-ExecutionPolicy Bypass -File node-opencode.ps1"
$psi.RedirectStandardInput = $true
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
$psi.UseShellExecute = $false
$psi.WorkingDirectory = "C:\Users\Asher\WorkSpace\05_Projects\10_nexus"

$proc = [System.Diagnostics.Process]::Start($psi)
# 写入 stdin
$stdin = $proc.StandardInput
$stdin.Write($json)
$stdin.Close()
# 读 stdout
$stdout = $proc.StandardOutput.ReadToEnd()
$stderr = $proc.StandardError.ReadToEnd()
$proc.WaitForExit()
$ec = $proc.ExitCode

Write-Host "Exit code: $ec"
Write-Host "--- stdout ---"
Write-Host $stdout
if ($stderr) {
    Write-Host "--- stderr ---"
    Write-Host $stderr
}

Remove-Item -Recurse -Force $testDir -ErrorAction SilentlyContinue
exit $ec
