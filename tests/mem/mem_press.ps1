$ErrorActionPreference = "SilentlyContinue"
if (-not (Get-NetTCPConnection -LocalPort 9999 -State Listen -EA SilentlyContinue)) {
    Start-Process python -ArgumentList "C:\Users\a1\myapi\tests\mock_upstream_mem.py","9999" -WindowStyle Hidden
}
Start-Sleep 2
if (-not (Get-NetTCPConnection -LocalPort 17834 -State Listen -EA SilentlyContinue)) {
    Start-Process "C:\Users\a1\myapi\rs\target\debug\myapi.exe" -WorkingDirectory "C:\Users\a1\myapi\rs" -WindowStyle Hidden
}
Start-Sleep 5
$p0 = Get-Process myapi -EA SilentlyContinue
if (-not $p0) { Write-Output "MYAPI_NOT_RUNNING"; exit }
$ws0 = $p0.WorkingSet64
Write-Output ("WS0_MB=" + [math]::Round($ws0/1MB,2))
& python "C:\Users\a1\myapi\tests\load_mem.py" 500 50
Start-Sleep 3
$p1 = Get-Process myapi -EA SilentlyContinue
$ws1 = $p1.WorkingSet64
Write-Output ("WS1_MB=" + [math]::Round($ws1/1MB,2))
Write-Output ("delta_MB=" + [math]::Round(($ws1-$ws0)/1MB,2))
Write-Output ("growth_pct=" + [math]::Round(($ws1-$ws0)/$ws0*100,2))
