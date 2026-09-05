Get-Process hub-rs -ErrorAction SilentlyContinue | ForEach-Object { Write-Output ('stopping hub-rs pid ' + $_.Id); Stop-Process -Id $_.Id -Force }
Start-Sleep -Seconds 1
Write-Output 'hub-rs stopped'
