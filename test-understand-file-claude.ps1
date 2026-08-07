$proj = [uri]::EscapeDataString('D:\mowen\deepcloud')
$path = [uri]::EscapeDataString('super-agent/hub-rs/src/services/claude_cli.rs')
$base = 'http://127.0.0.1:3001/api/understand/file'
$uri = $base + '?project=' + $proj + '&path=' + $path + '&explain=true&engine=claude&refresh=true&lang=zh-CN'
Write-Output '=== file engine=claude (refresh) ==='
try {
  $t = Measure-Command { $script:r = Invoke-RestMethod -Uri $uri -TimeoutSec 260 }
  Write-Output ('ms: ' + [int]$t.TotalMilliseconds + '  cached: ' + $r.cached + '  err: ' + $r.understandingError)
  $u = [string]$r.understanding
  Write-Output ('understanding length: ' + $u.Length)
  if ($u.Length -gt 500) { $u = $u.Substring(0,500) }
  Write-Output $u
} catch { Write-Output ('ERROR: ' + $_.Exception.Message) }
# cache hit
$uri2 = $base + '?project=' + $proj + '&path=' + $path + '&explain=true&engine=claude&lang=zh-CN'
$t2 = Measure-Command { $script:r2 = Invoke-RestMethod -Uri $uri2 -TimeoutSec 30 }
Write-Output ('cache-hit ms: ' + [int]$t2.TotalMilliseconds + '  cached: ' + $r2.cached)
