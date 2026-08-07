$body = @{ prompt = 'Reply with exactly: HUB-CLAUDE-OK'; project = 'D:\mowen\deepcloud' } | ConvertTo-Json
$r = Invoke-RestMethod -Uri 'http://127.0.0.1:3001/api/claude/run' -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 30
Write-Output ("taskId: " + $r.taskId)
$tid = $r.taskId
for ($i = 0; $i -lt 24; $i++) {
  Start-Sleep -Seconds 5
  $t = Invoke-RestMethod -Uri "http://127.0.0.1:3001/api/tasks/$tid" -TimeoutSec 10
  Write-Output ("poll " + $i + ": " + $t.status)
  if ($t.status -eq 'completed' -or $t.status -eq 'failed') { break }
}
$logs = Invoke-RestMethod -Uri "http://127.0.0.1:3001/api/tasks/$tid/logs" -TimeoutSec 10
Write-Output '--- logs ---'
Write-Output $logs.logs
