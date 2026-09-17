$scriptDir = $PSScriptRoot
Set-Location $scriptDir
$candidates = @(
    "$env:CARGO_TARGET_DIR\release\hub.exe",
    "$env:CARGO_TARGET_DIR\debug\hub.exe",
    (Join-Path $scriptDir "target\release\hub.exe"),
    (Join-Path $scriptDir "target\debug\hub.exe"),
    "E:\cargo_target\release\hub.exe",
    "E:\cargo_target\debug\hub.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Get-Item | Sort-Object LastWriteTime -Descending

if ($candidates.Count -eq 0) {
    Write-Error "hub.exe not found. Please run 'cargo build --bin hub' first."
    exit 1
}

$exe = $candidates[0].FullName
Write-Output "Using hub binary: $exe (Built at $($candidates[0].LastWriteTime))"
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
