$key = "sk-3c22e1fdc7ab4dce8cf3c5d2d0c7a18c"
$models = @("gemini-3-flash", "gemini-2.5-flash", "gemini-2.5-flash-lite", "gemini-3-pro-preview")
$headers = @{ "Authorization" = "Bearer $key"; "Content-Type" = "application/json" }

foreach ($m in $models) {
    # OpenAI-compatible
    $oaiBody = @{ model = $m; max_tokens = 64; temperature = 0.2; messages = @(@{ role = "user"; content = "Reply with exactly: PONG" }) } | ConvertTo-Json -Depth 5
    try {
        $r = Invoke-RestMethod -Uri "http://127.0.0.1:8045/v1/chat/completions" -Method POST -Headers $headers -Body $oaiBody -TimeoutSec 40
        $txt = $r.choices[0].message.content
        Write-Host "[OAI ] $m => OK: $txt"
    } catch {
        Write-Host "[OAI ] $m => ERR: $($_.Exception.Message)"
    }
    # Anthropic
    $antBody = @{ model = $m; max_tokens = 64; temperature = 0.2; messages = @(@{ role = "user"; content = "Reply with exactly: PONG" }) } | ConvertTo-Json -Depth 5
    try {
        $r = Invoke-RestMethod -Uri "http://127.0.0.1:8045/v1/messages" -Method POST -Headers $headers -Body $antBody -TimeoutSec 40
        $txt = ($r.content | Where-Object { $_.type -eq "text" } | ForEach-Object { $_.text }) -join ""
        Write-Host "[ANT ] $m => OK: $txt"
    } catch {
        Write-Host "[ANT ] $m => ERR: $($_.Exception.Message)"
    }
}
