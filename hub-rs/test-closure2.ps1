$base = 'http://127.0.0.1:3001'
$project = 'D:\mowen\deepcloud'
$enc = [System.Uri]::EscapeDataString($project)

Write-Output '=== 5. understand/project explain (1st call) ==='
$t1 = Measure-Command { $script:r1 = Invoke-RestMethod -Uri "$base/api/understand/project?project=$enc&explain=true&lang=zh-CN" -TimeoutSec 400 }
Write-Output ("cached: " + $r1.cached + "  ms: " + [int]$t1.TotalMilliseconds + "  len: " + ($r1.understanding | Out-String).Length)

Write-Output ''
Write-Output '=== 6. 2nd call, expect cached=true fast ==='
$t2 = Measure-Command { $script:r2 = Invoke-RestMethod -Uri "$base/api/understand/project?project=$enc&explain=true&lang=zh-CN" -TimeoutSec 60 }
Write-Output ("cached: " + $r2.cached + "  ms: " + [int]$t2.TotalMilliseconds)

Write-Output ''
Write-Output '=== 7. cacheOnly probe ==='
$r3 = Invoke-RestMethod -Uri "$base/api/understand/project?project=$enc&explain=true&cacheOnly=true&lang=zh-CN" -TimeoutSec 60
Write-Output ("cached: " + $r3.cached + "  hasUnderstanding: " + ($null -ne $r3.understanding))

Write-Output ''
Write-Output '=== 8. lang=en-US (expect English) ==='
$r4 = Invoke-RestMethod -Uri "$base/api/understand/project?project=$enc&explain=true&lang=en-US" -TimeoutSec 400
$snippet = ($r4.understanding | Out-String)
Write-Output ("cached: " + $r4.cached + "  snippet: " + $snippet.Substring(0, [Math]::Min(160, $snippet.Length)))

Write-Output ''
Write-Output '=== 9. cache file on disk ==='
$cacheDir = Join-Path $env:USERPROFILE '.aindex-hub\cache\understanding'
Get-ChildItem $cacheDir -ErrorAction SilentlyContinue | ForEach-Object { Write-Output ($_.Name + ' ' + $_.Length + ' bytes') }

Write-Output ''
Write-Output '=== 10. reverse-engineering with project context ==='
$body = @{ requirement = '为订单管理系统增加优惠券模块，支持满减和折扣两种类型，用户下单时自动匹配最优优惠券'; project = $project; options = @{ language = 'zh-CN' } } | ConvertTo-Json
$rev = Invoke-RestMethod -Uri "$base/api/reverse-engineering" -Method Post -ContentType 'application/json; charset=utf-8' -Body ([System.Text.Encoding]::UTF8.GetBytes($body)) -TimeoutSec 400
Write-Output ("modules: " + ($rev.requirement_schema.modules -join ', '))
Write-Output ("features: " + ($rev.requirement_schema.features -join ', '))
Write-Output ("blueprint modules: " + $rev.architecture_blueprint.modules.Count + "  tasks: " + $rev.development_tasks.Count)
foreach ($step in $rev.trace) { Write-Output ("trace: " + $step.stage + " -> " + $step.status + " warnings=" + $step.warnings.Count) }
Write-Output ("supermemory refs: " + $rev.project_context.supermemory.Count)

Write-Output ''
Write-Output '=== 11. supermemory blueprint records ==='
$mem = Invoke-RestMethod -Uri "$base/api/memory/search?project=$enc&q=blueprint"
Write-Output ("blueprint search hits: " + $mem.results.Count)
