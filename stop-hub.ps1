Get-Process hub, hub-rs, superagent -ErrorAction SilentlyContinue | ForEach-Object { Write-Output ('stopping process ' + $_.ProcessName + ' pid ' + $_.Id); Stop-Process -Id $_.Id -Force }
Start-Sleep -Seconds 1
Write-Output 'hub service stopped'
