param(
    [Parameter(Mandatory=$true)][string]$EvidencePrefix,
    [Parameter(Mandatory=$true)][int]$FirstPort
)
$ErrorActionPreference = 'Stop'
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$prefix = [IO.Path]::GetFullPath($EvidencePrefix)
$snapshotOutput = $prefix + '-source-snapshot.json'
$frozenOutput = $prefix + '-frozen.json'
$seriesPrefix = $prefix + '-independent'
foreach ($candidate in @($snapshotOutput, $frozenOutput, ($seriesPrefix + '-series.json'))) {
    if (Test-Path -LiteralPath $candidate) { throw "Evidence already exists: $candidate" }
}
$runtimeRoot = Join-Path $repositoryRoot ('artifacts/.delivery-runtime/' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $runtimeRoot -ErrorAction Stop | Out-Null
$priorTemp = $env:TEMP
$priorTmp = $env:TMP
try {
    $env:TEMP = $runtimeRoot
    $env:TMP = $runtimeRoot
    & python (Join-Path $repositoryRoot 'scripts/snapshot_delivery_inputs.py') --output $snapshotOutput
    if ($LASTEXITCODE -ne 0) { throw 'Source snapshot failed' }
    $snapshot = Get-Content -LiteralPath $snapshotOutput -Raw | ConvertFrom-Json
    $frozenRoot = Join-Path $snapshot.stage 'super-agent'
    & python (Join-Path $frozenRoot 'scripts/hub_delivery_evals.py') prepare-rule --output $frozenOutput --model-turn-budget 40 --timeout-seconds 1500
    if ($LASTEXITCODE -ne 0) { throw 'Frozen rule setup failed' }
    & python (Join-Path $frozenRoot 'scripts/run_independent_delivery_series.py') --frozen $frozenOutput --prefix $seriesPrefix --first-port $FirstPort
    if ($LASTEXITCODE -ne 0) { throw 'Independent series did not pass; original evidence retained' }
} finally {
    $env:TEMP = $priorTemp
    $env:TMP = $priorTmp
}
