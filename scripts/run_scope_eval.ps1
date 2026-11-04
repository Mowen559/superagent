param([string]$Label = 'hub-scope-v1')
$ErrorActionPreference = 'Stop'
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$stageRoot = Join-Path $repositoryRoot ('artifacts/' + $Label)
$cases = Get-Content -LiteralPath (Join-Path $stageRoot 'cases.json') -Raw | ConvertFrom-Json
$originalTemp = $env:TEMP
$originalTmp = $env:TMP
$cleanup = @()
try {
    $caseIndex = 0
    foreach ($case in $cases) {
        $port = 39821 + $caseIndex
        $caseIndex++
        if (Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue) { throw "Port $port is already occupied" }
        $env:TEMP = $case.dir
        $env:TMP = $case.dir
        $identityPath = Join-Path $case.dir 'identity.json'
        & (Join-Path $PSScriptRoot 'start_delivery_eval_service.ps1') -Profile (Join-Path $case.dir 'profile.json') -IdentityOutput $identityPath -Port $port -ModelTurnBudget $case.turns
        $identity = Get-Content -LiteralPath $identityPath -Raw | ConvertFrom-Json
        try {
            $ready = $false
            for ($probe = 0; $probe -lt 80; $probe++) {
                $client = [System.Net.Sockets.TcpClient]::new()
                try { $client.Connect('127.0.0.1', $port); $ready = $true; break } catch { Start-Sleep -Milliseconds 250 } finally { $client.Dispose() }
            }
            if (-not $ready) { throw 'Owned test service did not listen' }
            $report = Join-Path $repositoryRoot ('docs/reviews/' + $Label + '-' + $case.kind + '.json')
            & node (Join-Path $repositoryRoot '../deepcloud-ui/scripts/hub-delivery-replay.cjs') --requests (Join-Path $case.dir 'requests.json') --output $report --base ('http://127.0.0.1:' + $port) --identity $identityPath --isolated *> (Join-Path $case.dir 'harness.log')
            if ($LASTEXITCODE -ne 0) { throw ('Harness infrastructure failed: ' + $case.kind) }
            Get-Content -LiteralPath (Join-Path $case.dir 'harness.log') -Tail 1
        } finally {
            $owned = Get-CimInstance Win32_Process -Filter ('ProcessId=' + $identity.pid) -ErrorAction SilentlyContinue
            $expectedPath = Join-Path $identity.stage 'hub.exe'
            $stopped = $false
            if ($owned) {
                if ($owned.ExecutablePath -ne $expectedPath) { throw 'PID identity changed; refusing stop' }
                Stop-Process -Id $identity.pid -ErrorAction Stop
                $stopped = $true
            }
            $cleanup += @{ case=$case.kind; pid=$identity.pid; executable=$expectedPath; stopped=$stopped }
            $cleanup | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $stageRoot 'runtime-cleanup.json') -Encoding utf8
        }
    }
} finally {
    $env:TEMP = $originalTemp
    $env:TMP = $originalTmp
}
