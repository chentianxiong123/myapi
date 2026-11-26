$ErrorActionPreference = "SilentlyContinue"
# 清理旧进程
Get-Process myapi -EA SilentlyContinue | Stop-Process -Force
Get-Process python -EA SilentlyContinue | ForEach-Object { $_.Kill() }
Start-Sleep 1
# 启动 mock + myapi
Start-Process python -ArgumentList "C:\Users\a1\myapi\tests\mock_upstream_mem.py","9999" -WindowStyle Hidden
Start-Sleep 2
Start-Process "C:\Users\a1\myapi\rs\target\debug\myapi.exe" -WorkingDirectory "C:\Users\a1\myapi\rs" -WindowStyle Hidden
Start-Sleep 5
$p = Get-Process myapi -EA SilentlyContinue
if (-not $p) { Write-Output "MYAPI_NOT_RUNNING"; exit }
$ws0 = $p.WorkingSet64
Write-Output ("T0_idle_MB=" + [math]::Round($ws0/1MB,2))
# 压力第 1 轮
& python "C:\Users\a1\myapi\tests\load_mem.py" 500 50
Start-Sleep 3
Write-Output ("T1_after_round1_MB=" + [math]::Round((Get-Process myapi).WorkingSet64/1MB,2))
# 空闲 30s
Start-Sleep 30
Write-Output ("T2_idle30s_MB=" + [math]::Round((Get-Process myapi).WorkingSet64/1MB,2))
# 压力第 2 轮
& python "C:\Users\a1\myapi\tests\load_mem.py" 500 50
Start-Sleep 3
Write-Output ("T3_after_round2_MB=" + [math]::Round((Get-Process myapi).WorkingSet64/1MB,2))
