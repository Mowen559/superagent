$proj = [uri]::EscapeDataString('D:\mowen\deepcloud')
$base = 'http://127.0.0.1:3001/api/understand/project'
# No refresh -> should hit the claude-scoped cache instantly.
$uri = $base + '?project=' + $proj + '&explain=true&engine=claude&lang=zh-CN'
$t = Measure-Command { $script:r = Invoke-RestMethod -Uri $uri -TimeoutSec 30 }
Write-Output ('claude cache-hit ms: ' + [int]$t.TotalMilliseconds + '  cached: ' + $r.cached + '  len: ' + ([string]$r.understanding).Length)
# Default LLM engine should be a SEPARATE cache entry (cacheOnly probe -> miss expected until generated).
$uri2 = $base + '?project=' + $proj + '&explain=true&lang=zh-CN&cacheOnly=true'
$t2 = Measure-Command { $script:r2 = Invoke-RestMethod -Uri $uri2 -TimeoutSec 30 }
Write-Output ('llm cacheOnly probe ms: ' + [int]$t2.TotalMilliseconds + '  cached: ' + $r2.cached + '  cacheMiss: ' + $r2.cacheMiss)
