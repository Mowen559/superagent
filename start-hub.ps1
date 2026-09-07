$scriptDir = $PSScriptRoot
Set-Location $scriptDir
$exe = Join-Path $scriptDir "target\release\hub.exe"
if (-not (Test-Path $exe)) {
    $exe = Join-Path $scriptDir "target\debug\hub.exe"
}
if (-not (Test-Path $exe)) {
    Write-Error "hub.exe not found in target\release or target\debug. Please run 'cargo build --bin hub' first."
    exit 1
}
Start-Process -FilePath $exe -ArgumentList "service", "3001" -WindowStyle Hidden
Write-Output 'started hub-rs service on port 3001'
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
