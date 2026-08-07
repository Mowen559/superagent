Set-Location D:\mowen\deepcloud\super-agent\hub-rs
$exe = 'D:\mowen\deepcloud\super-agent\hub-rs\target\release\hub-rs.exe'
Start-Process -FilePath $exe -WindowStyle Hidden
Write-Output 'started hub-rs'
for ($i = 0; $i -lt 15; $i++) {
  Start-Sleep -Seconds 2
  try {
    $h = Invoke-RestMethod -Uri 'http://127.0.0.1:3001/api/health' -TimeoutSec 3
    Write-Output ('health ok after ' + (($i + 1) * 2) + 's')
    break
  } catch { }
}
Write-Output '--- claude status ---'
Invoke-RestMethod -Uri 'http://127.0.0.1:3001/api/claude/status' -TimeoutSec 30 | ConvertTo-Json -Compress
