$proj = [uri]::EscapeDataString('D:\mowen\deepcloud')
$base = 'http://127.0.0.1:3001/api/understand/project'
$uri = $base + '?project=' + $proj + '&explain=true&engine=claude&refresh=true&lang=zh-CN'
Write-Output ('URI: ' + $uri)
Write-Output '=== engine=claude (refresh) ==='
try {
  $t = Measure-Command { $script:r = Invoke-RestMethod -Uri $uri -TimeoutSec 260 }
  Write-Output ('ms: ' + [int]$t.TotalMilliseconds)
  Write-Output ('cached: ' + $r.cached)
  Write-Output ('understandingError: ' + $r.understandingError)
  $u = [string]$r.understanding
  Write-Output ('understanding length: ' + $u.Length)
  Write-Output '--- understanding (first 900 chars) ---'
  if ($u.Length -gt 900) { $u = $u.Substring(0,900) }
  Write-Output $u
} catch {
  Write-Output ('ERROR: ' + $_.Exception.Message)
}
