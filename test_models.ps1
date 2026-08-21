$modelsToTest = @(
    "claude-sonnet-4-6",
    "claude-opus-4-6-thinking",
    "claude-3-5-sonnet-20241022",
    "claude-3-opus-20240229",
    "claude-3-5-haiku-20241022",
    "gpt-4o",
    "gpt-4o-mini",
    "gpt-4",
    "gpt-4-turbo",
    "gpt-3.5-turbo",
    "gemini-3.7-flash-tiered",
    "gemini-pro-agent"
)

$uri = "http://127.0.0.1:3001/v1/chat/completions"

$results = @()

foreach ($model in $modelsToTest) {
    Write-Host "Testing model: $model ..." -NoNewline
    $body = @{
        model = $model
        messages = @(
            @{
                role = "user"
                content = "Ping! Reply with 'PONG' and your model name if you can read this."
            }
        )
        max_tokens = 50
    } | ConvertTo-Json -Depth 4 -Compress

    $startTime = Get-Date
    try {
        $response = Invoke-RestMethod -Uri $uri -Method Post -Body $body -ContentType "application/json" -TimeoutSec 30
        $duration = ((Get-Date) - $startTime).TotalMilliseconds
        
        $content = ""
        if ($response.choices -and $response.choices.Count -gt 0) {
            $content = $response.choices[0].message.content
        } else {
            $content = ($response | ConvertTo-Json -Compress -Depth 5)
        }
        
        Write-Host " SUCCESS ($([math]::Round($duration)) ms)" -ForegroundColor Green
        $results += [PSCustomObject]@{
            Model = $model
            Status = "SUCCESS"
            DurationMs = [math]::Round($duration)
            Response = $content
            Error = ""
        }
    } catch {
        $duration = ((Get-Date) - $startTime).TotalMilliseconds
        $errMessage = $_.Exception.Message
        if ($_.Exception.Response) {
            $reader = New-Object System.IO.StreamReader($_.Exception.Response.GetResponseStream())
            $errBody = $reader.ReadToEnd()
            $errMessage = "$errMessage | Body: $errBody"
        }
        Write-Host " FAILED ($([math]::Round($duration)) ms) - $errMessage" -ForegroundColor Red
        $results += [PSCustomObject]@{
            Model = $model
            Status = "FAILED"
            DurationMs = [math]::Round($duration)
            Response = ""
            Error = $errMessage
        }
    }
}

Write-Host "`n=== Test Results Summary ==="
$results | Format-Table -AutoSize
$results | ConvertTo-Json -Depth 4 | Out-File -FilePath "test_results.json" -Encoding utf8
