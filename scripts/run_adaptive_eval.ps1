param([string]$Label='hub-adaptive-v1')
$ErrorActionPreference='Stop'
$repositoryRoot=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$stageRoot=Join-Path $repositoryRoot ('artifacts/'+$Label)
$cleanup=@()
foreach($kind in @('simple','complex')) {
    $dir=Join-Path $stageRoot $kind
    $frozen=Get-Content -LiteralPath (Join-Path $dir 'frozen.json') -Raw | ConvertFrom-Json
    $port=if($kind -eq 'simple'){39831}else{39832}
    $probe=[System.Net.Sockets.TcpClient]::new()
    try{$probe.Connect('127.0.0.1',$port);throw 'Port is occupied'}catch [System.Net.Sockets.SocketException]{}finally{$probe.Dispose()}
    $privateStage=Join-Path $dir ('runtime-'+[guid]::NewGuid().ToString('N'))
    $dataRoot=Join-Path $privateStage 'data'
    New-Item -ItemType Directory -Path $dataRoot | Out-Null
    $settingsReference=Join-Path $env:USERPROFILE '.superagent/settings'
    New-Item -ItemType Junction -Path (Join-Path $dataRoot 'settings') -Target $settingsReference | Out-Null
    $binary=Join-Path $privateStage 'hub.exe'
    Copy-Item -LiteralPath (Join-Path $repositoryRoot 'target/debug/hub.exe') -Destination $binary
    if((Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash -ne $frozen.candidate.sha256){throw 'Candidate changed'}
    $env:HUB_APP_DATA_ROOT=$dataRoot
    $env:HUB_SKILLS_ROOT=Join-Path $repositoryRoot 'skills'
    $env:HUB_AGENT_MAX_MODEL_TURNS=[string]$frozen.model_turns
    $env:DISABLE_FASTEMBED='1'
    Remove-Item Env:HUB_DELIVERY_PROFILE_PATH -ErrorAction SilentlyContinue
    $env:TEMP=$dir
    $env:TMP=$dir
    $serviceProcess=Start-Process -FilePath $binary -ArgumentList @('service',[string]$port) -WorkingDirectory $repositoryRoot -WindowStyle Hidden -RedirectStandardOutput (Join-Path $privateStage 'stdout.log') -RedirectStandardError (Join-Path $privateStage 'stderr.log') -PassThru
    $identity=@{pid=$serviceProcess.Id;port=$port;stage=$privateStage;app_data_root=$dataRoot;binarySha256=$frozen.candidate.sha256;profile=$null;settings_reference=$settingsReference;settings_sha256=(Get-FileHash -LiteralPath (Join-Path $settingsReference 'llm.json') -Algorithm SHA256).Hash;model_turn_budget=$frozen.model_turns;evolution_baseline='private empty service, complex continuation retains its own history'}
    $identityPath=Join-Path $dir 'identity.json'
    $identity | ConvertTo-Json | Set-Content -LiteralPath $identityPath -Encoding utf8
    try{
        $ready=$false
        for($n=0;$n -lt 80;$n++){$c=[System.Net.Sockets.TcpClient]::new();try{$c.Connect('127.0.0.1',$port);$ready=$true;break}catch{Start-Sleep -Milliseconds 250}finally{$c.Dispose()}}
        if(-not $ready){throw 'Private service not listening'}
        [string[]]$extra=@()
        if($kind -eq 'complex'){$extra=@('--adaptive-continuation')}
        & node (Join-Path $PSScriptRoot 'hub-adaptive-replay.cjs') --requests (Join-Path $dir 'requests.json') --output (Join-Path $repositoryRoot ('docs/reviews/'+$Label+'-'+$kind+'.json')) --base ('http://127.0.0.1:'+$port) --identity $identityPath --isolated @extra *> (Join-Path $dir 'harness.log')
        if($LASTEXITCODE -ne 0){throw ('Harness infrastructure failed: '+$kind)}
        Get-Content -LiteralPath (Join-Path $dir 'harness.log') -Tail 2
    }finally{
        $owned=Get-CimInstance Win32_Process -Filter ('ProcessId='+$serviceProcess.Id) -ErrorAction SilentlyContinue
        $stopped=$false
        if($owned){if($owned.ExecutablePath -ne $binary){throw 'PID changed; refuse stop'};Stop-Process -Id $serviceProcess.Id -ErrorAction Stop;$stopped=$true}
        $cleanup+=@{kind=$kind;pid=$serviceProcess.Id;executable=$binary;stopped=$stopped}
        $cleanup | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $stageRoot 'runtime-cleanup.json') -Encoding utf8
    }
}
