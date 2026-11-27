Set-Location C:\Users\a1\myapi\rs
$stamp = Get-Date -Format 'yyyyMMddHHmmss'
"start-$stamp" | Out-File C:\Users\a1\myapi\rs\test_start.txt -Encoding ascii
cargo test 2>&1 | Out-File -FilePath C:\Users\a1\myapi\rs\test_result.log -Encoding utf8
"end-$stamp-exit-$LASTEXITCODE" | Out-File C:\Users\a1\myapi\rs\test_end.txt -Encoding ascii