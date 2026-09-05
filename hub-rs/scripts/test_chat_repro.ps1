$key = "sk-3c22e1fdc7ab4dce8cf3c5d2d0c7a18c"
$sys = "You are a codebase assistant. Use the following project context.`n`nProject memories:`n- graph_service aligned with CBM schema (super-agent/hub-rs/src/services/graph_service.rs)`n`nRelated code symbols (from indexed graph):`n- [Function] hybridSearch (super-agent/GitNexus/gitnexus/src/core/search/hybrid-search.ts) via fts`n- [Function] hybrid_search (super-agent/hub-rs/src/services/vector_service.rs) via fts+semantic`n- [Interface] HybridSearchResult (super-agent/GitNexus/gitnexus/src/core/search/hybrid-search.ts) via fts"
$payload = @{
    model = "gemini-3-flash"
    max_tokens = 8000
    temperature = 0.2
    system = $sys
    messages = @(@{ role = "user"; content = "用一句话说明这个项目里 hub-rs 的 hybrid_search 是怎么融合 FTS 和向量检索的" })
} | ConvertTo-Json -Depth 5
$bytes = [System.Text.Encoding]::UTF8.GetBytes($payload)
$r = Invoke-WebRequest -Uri "http://127.0.0.1:8045/v1/messages" -Method POST -Headers @{ "Authorization" = "Bearer $key" } -Body $bytes -ContentType "application/json; charset=utf-8" -TimeoutSec 90 -UseBasicParsing
$raw = [System.Text.Encoding]::UTF8.GetString($r.RawContentStream.ToArray())
Write-Host $raw
