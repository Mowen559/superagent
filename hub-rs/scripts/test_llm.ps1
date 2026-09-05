$ErrorActionPreference = "Stop"
$base = "http://127.0.0.1:8045"
$key = "sk-3c22e1fdc7ab4dce8cf3c5d2d0c7a18c"
$model = "claude-sonnet-4-6"

$body = @{
    model = $model
    max_tokens = 64
    temperature = 0.2
    messages = @(@{ role = "user"; content = "Reply with exactly: PONG" })
} | ConvertTo-Json -Depth 5

function TryCall($desc, $headers) {
    Write-Host "=== $desc ==="
    try {
        $r = Invoke-WebRequest -Uri "$base/v1/messages" -Method POST -Headers $headers -Body $body -ContentType "application/json" -TimeoutSec 30 -UseBasicParsing
        Write-Host "HTTP $($r.StatusCode)"
        Write-Host $r.Content
    } catch {
        if ($_.Exception.Response) {
            $sr = New-Object System.IO.StreamReader($_.Exception.Response.GetResponseStream())
            Write-Host "HTTP $([int]$_.Exception.Response.StatusCode): $($sr.ReadToEnd())"
        } else { Write-Host "ERR: $($_.Exception.Message)" }
    }
    Write-Host ""
}

TryCall "Bearer auth (hub-rs style)" @{ "Authorization" = "Bearer $key" }
TryCall "x-api-key + version" @{ "x-api-key" = $key; "anthropic-version" = "2023-06-01" }
