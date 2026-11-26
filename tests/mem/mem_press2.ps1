$ErrorActionPreference = "SilentlyContinue"
# 空闲 30s 后读（临时分配应回落）
Start-Sleep 30
$p = Get-Process myapi -EA SilentlyContinue
if (-not $p) { Write-Output "MYAPI_NOT_RUNNING"; exit }
Write-Output ("WS_idle30_MB=" + [math]::Round($p.WorkingSet64/1MB,2))
# 再打一轮 500
& python "C:\Users\a1\myapi\tests\load_mem.py" 500 50
Start-Sleep 3
$p2 = Get-Process myapi -EA SilentlyContinue
Write-Output ("WS_round2_MB=" + [math]::Round($p2.WorkingSet64/1MB,2))
