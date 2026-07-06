<#
.SYNOPSIS
    Nexus 打包脚本 — 构建 release 并打包分发文件
#>

$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $root "nexus-dist"

# 清理旧包
if (Test-Path $dist) { Remove-Item -Recurse -Force $dist }

# 构建
Write-Host "Building release..." -ForegroundColor Yellow
Push-Location $root
$build = cargo build --release 2>&1
if ($LASTEXITCODE -ne 0) { Write-Host $build; throw "Build failed" }

# 创建目录结构
$dirs = @(
    (Join-Path $dist "bin"),
    (Join-Path $dist "workflows"),
    (Join-Path $dist "docs")
)
foreach ($d in $dirs) { New-Item -ItemType Directory -Path $d -Force | Out-Null }

# 复制二进制
Copy-Item (Join-Path $root "target\release\nexus-cli.exe") (Join-Path $dist "bin\")
Copy-Item (Join-Path $root "target\release\nexus-mcp-server.exe") (Join-Path $dist "bin\")

# 复制工作流示例
Copy-Item (Join-Path $root "workflows\*") (Join-Path $dist "workflows\")

# 复制核心文档
Copy-Item (Join-Path $root "README.md") (Join-Path $dist "\")
Copy-Item (Join-Path $root "docs\architecture\NODE_PROTOCOL.md") (Join-Path $dist "docs\")
Copy-Item (Join-Path $root "test_workflow.json") (Join-Path $dist "\")

# 统计
$size = (Get-ChildItem -Recurse $dist | Measure-Object -Property Length -Sum).Sum
Write-Host "Package created: $dist" -ForegroundColor Green
Write-Host "Total size: $([math]::Round($size / 1KB)) KB" -ForegroundColor Green
Write-Host "Files:" -ForegroundColor Cyan
Get-ChildItem -Recurse $dist | Where-Object { -not $_.PSIsContainer } | ForEach-Object {
    Write-Host "  $($_.FullName.Replace($dist,''))  ($([math]::Round($_.Length / 1KB)) KB)"
}
Pop-Location
