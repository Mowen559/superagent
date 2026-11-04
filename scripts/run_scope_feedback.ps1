param([string]$Label = 'hub-scope-feedback-v1')
$ErrorActionPreference = 'Stop'
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$stageRoot = Join-Path $repositoryRoot ('artifacts/' + $Label)
$frozen = Get-Content -LiteralPath (Join-Path $stageRoot 'frozen.json') -Raw | ConvertFrom-Json
$prior = $frozen.prior_identity
$binary = Join-Path $prior.stage 'hub.exe'
if ((Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash -ne $frozen.candidate_sha256) { throw 'Prior candidate binary changed' }
$port = 39824
$probe = [System.Net.Sockets.TcpClient]::new()
try { $probe.Connect('127.0.0.1',$port); throw 'Feedback port already occupied' } catch [System.Net.Sockets.SocketException] {} finally { $probe.Dispose() }
$env:HUB_APP_DATA_ROOT = $prior.app_data_root
$env:HUB_SKILLS_ROOT = Join-Path $repositoryRoot 'skills'
$env:HUB_AGENT_MAX_MODEL_TURNS = '10'
$env:DISABLE_FASTEMBED = '1'
$env:HUB_DELIVERY_PROFILE_PATH = $frozen.profile_path
$env:TEMP = $stageRoot
$env:TMP = $stageRoot
$serviceProcess = Start-Process -FilePath $binary -ArgumentList @('service',[string]$port) -WorkingDirectory $repositoryRoot -WindowStyle Hidden -RedirectStandardOutput (Join-Path $stageRoot 'stdout.log') -RedirectStandardError (Join-Path $stageRoot 'stderr.log') -PassThru
$identity = @{
    pid=$serviceProcess.Id; port=$port; stage=$prior.stage; app_data_root=$prior.app_data_root;
    binarySha256=$frozen.candidate_sha256; profile=$frozen.profile_path;
    settings_reference=$prior.settings_reference; settings_sha256=$prior.settings_sha256;
    model_turn_budget=10; resume_history=$true; prior_task=$frozen.prior_task;
    prior_session=$frozen.prior_session; control_stage=$stageRoot
}
$identityPath = Join-Path $stageRoot 'identity.json'
$identity | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $identityPath -Encoding utf8
try {
    $ready=$false
    for ($i=0; $i -lt 80; $i++) {
        $client=[System.Net.Sockets.TcpClient]::new()
        try { $client.Connect('127.0.0.1',$port); $ready=$true; break } catch { Start-Sleep -Milliseconds 250 } finally { $client.Dispose() }
    }
    if (-not $ready) { throw 'Feedback service did not listen' }
    & node (Join-Path $repositoryRoot '../deepcloud-ui/scripts/hub-delivery-replay.cjs') --requests (Join-Path $stageRoot 'requests.json') --output (Join-Path $repositoryRoot ('docs/reviews/'+$Label+'.json')) --base ('http://127.0.0.1:'+$port) --identity $identityPath --isolated *> (Join-Path $stageRoot 'harness.log')
    if ($LASTEXITCODE -ne 0) { throw 'Feedback SDK infrastructure failed' }
    Get-Content -LiteralPath (Join-Path $stageRoot 'harness.log') -Tail 1
} finally {
    $owned=Get-CimInstance Win32_Process -Filter ('ProcessId='+$serviceProcess.Id) -ErrorAction SilentlyContinue
    $stopped=$false
    if ($owned) {
        if ($owned.ExecutablePath -ne $binary) { throw 'Feedback PID identity changed; refusing stop' }
        Stop-Process -Id $serviceProcess.Id -ErrorAction Stop
        $stopped=$true
    }
    @{pid=$serviceProcess.Id; executable=$binary; stopped=$stopped} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $stageRoot 'runtime-cleanup.json') -Encoding utf8
}
