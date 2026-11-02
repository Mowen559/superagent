param(
    [Parameter(Mandatory=$true)][string]$Profile,
    [Parameter(Mandatory=$true)][string]$IdentityOutput,
    [Parameter(Mandatory=$true)][int]$Port,
    [ValidateRange(1,40)][int]$ModelTurnBudget = 32
)
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath $IdentityOutput) { throw 'Do not overwrite an existing runtime identity' }
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$profilePath = (Resolve-Path -LiteralPath $Profile).Path
$settingsReference = Join-Path $env:USERPROFILE '.superagent/settings'
if (-not (Test-Path -LiteralPath (Join-Path $settingsReference 'llm.json') -PathType Leaf)) { throw 'Existing model settings are unavailable' }
$stage = Join-Path $env:TEMP ('hub-independent-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stage | Out-Null
$dataRoot = Join-Path $stage 'data'
New-Item -ItemType Directory -Path $dataRoot | Out-Null
# Reference the existing configuration; never copy credential contents.
New-Item -ItemType Junction -Path (Join-Path $dataRoot 'settings') -Target $settingsReference | Out-Null
$binary = Join-Path $stage 'hub.exe'
Copy-Item -LiteralPath (Join-Path $repositoryRoot 'target/debug/hub.exe') -Destination $binary
$initialEvolutionExists = Test-Path -LiteralPath (Join-Path $dataRoot 'evolution/strategies.json')
$env:HUB_APP_DATA_ROOT = $dataRoot
$env:HUB_SKILLS_ROOT = Join-Path $repositoryRoot 'skills'
$env:HUB_AGENT_MAX_MODEL_TURNS = [string]$ModelTurnBudget
$env:DISABLE_FASTEMBED = '1'
$env:HUB_DELIVERY_PROFILE_PATH = $profilePath
$frozenSourcePath = Join-Path $repositoryRoot 'frozen-source.json'
if (Test-Path -LiteralPath $frozenSourcePath) {
    $frozenSource = Get-Content -LiteralPath $frozenSourcePath -Raw | ConvertFrom-Json
    $env:NODE_PATH = $frozenSource.node_modules_reference
}
$serviceProcess = Start-Process -FilePath $binary -ArgumentList @('service',[string]$Port) -WorkingDirectory $repositoryRoot -WindowStyle Hidden -RedirectStandardOutput (Join-Path $stage 'stdout.log') -RedirectStandardError (Join-Path $stage 'stderr.log') -PassThru
$identity = @{
    pid=$serviceProcess.Id; port=$Port; stage=$stage; app_data_root=$dataRoot;
    profile=$profilePath; binarySha256=(Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash;
    settings_reference=$settingsReference;
    settings_sha256=(Get-FileHash -LiteralPath (Join-Path $settingsReference 'llm.json') -Algorithm SHA256).Hash;
    evolution_initial_exists=$initialEvolutionExists;
    evolution_baseline='empty; private to this service and independent Run';
    model_turn_budget=$ModelTurnBudget;
    launcher_sha256=(Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash
}
$identity | ConvertTo-Json | Set-Content -LiteralPath $IdentityOutput -Encoding utf8
Write-Output ('Started isolated test Hub on port ' + $Port + ', PID ' + $serviceProcess.Id)
