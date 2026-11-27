$ErrorActionPreference = 'Stop'
$log = 'C:\Users\a1\myapi\rs\test_result.log'
if (Test-Path $log) { Remove-Item $log -Force }
$p = Start-Process -FilePath powershell.exe `
  -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','C:\Users\a1\myapi\tests\run_test_bg.ps1' `
  -WindowStyle Hidden -PassThru
"launched pid=$($p.Id)" | Out-File C:\Users\a1\myapi\rs\test_pid.txt -Encoding ascii
Write-Output "launched pid=$($p.Id)"