# myapi 网关 mock 测试（win11 PowerShell，全部 mock，不真调上游）
# 验证：编译 → 起服务 → /v1/models / /v1/chat/completions → 停服务
$ErrorActionPreference = "Continue"
Set-Location "C:\Users\a1\myapi\rs"

Write-Output "========== 1. cargo build (debug) =========="
cargo build 2>&1 | Select-Object -Last 5
if (-not (Test-Path "target\debug\myapi.exe")) {
    Write-Output "BUILD FAILED: no myapi.exe"
    exit 1
}
Write-Output "BUILD OK: target\debug\myapi.exe"

Write-Output ""
Write-Output "========== 2. start server (port 17834) =========="
$proc = Start-Process -FilePath ".\target\debug\myapi.exe" -PassThru -RedirectStandardOutput "C:\Users\a1\myapi\rs\mock.log" -WindowStyle Hidden
Start-Sleep -Seconds 3
Write-Output "PID: $($proc.Id)"
try {
    $null = Invoke-WebRequest -Uri "http://127.0.0.1:17834/api/version" -UseBasicParsing -TimeoutSec 5
    Write-Output "server up OK"
} catch {
    Write-Output "SERVER FAILED TO START, log:"
    Get-Content "C:\Users\a1\myapi\rs\mock.log"
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    exit 1
}

Write-Output ""
Write-Output "========== 3. GET /v1/models (expect mock model list) =========="
try {
    $r = Invoke-WebRequest -Uri "http://127.0.0.1:17834/v1/models" -UseBasicParsing -TimeoutSec 5
    Write-Output "STATUS: $($r.StatusCode)"
    Write-Output $r.Content
} catch {
    Write-Output "ERROR: $_"
}

Write-Output ""
Write-Output "========== 4. POST /v1/chat/completions model=mock-model (mock upstream down -> expect 502) =========="
try {
    $body = '{"model":"mock-model","messages":[{"role":"user","content":"hi"}]}'
    $r = Invoke-WebRequest -Uri "http://127.0.0.1:17834/v1/chat/completions" -Method POST -Body $body -ContentType "application/json" -UseBasicParsing -TimeoutSec 8
    Write-Output "STATUS: $($r.StatusCode)"
    Write-Output $r.Content
} catch {
    $resp = $_.Exception.Response
    if ($resp) {
        Write-Output "HTTP STATUS: $([int]$resp.StatusCode)"
        $sr = New-Object System.IO.StreamReader($resp.GetResponseStream())
        Write-Output $sr.ReadToEnd()
    } else {
        Write-Output "ERROR: $_"
    }
}

Write-Output ""
Write-Output "========== 5. POST model=nonexistent (expect 404 no channel) =========="
try {
    $body = '{"model":"nonexistent","messages":[{"role":"user","content":"hi"}]}'
    $r = Invoke-WebRequest -Uri "http://127.0.0.1:17834/v1/chat/completions" -Method POST -Body $body -ContentType "application/json" -UseBasicParsing -TimeoutSec 5
    Write-Output "STATUS: $($r.StatusCode)"
    Write-Output $r.Content
} catch {
    $resp = $_.Exception.Response
    if ($resp) {
        Write-Output "HTTP STATUS: $([int]$resp.StatusCode)"
        $sr = New-Object System.IO.StreamReader($resp.GetResponseStream())
        Write-Output $sr.ReadToEnd()
    } else {
        Write-Output "ERROR: $_"
    }
}

Write-Output ""
Write-Output "========== 6. stop server =========="
Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
Write-Output "server stopped"
Write-Output "========== mock test done =========="
