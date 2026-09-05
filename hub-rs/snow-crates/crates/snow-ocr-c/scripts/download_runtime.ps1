$ErrorActionPreference = "Stop"

$runtimeVersion = "1.24.2"
$packageUrl = "https://www.nuget.org/api/v2/package/Microsoft.ML.OnnxRuntime/$runtimeVersion"
$packageBytes = 124756536
$packageSha256 = "782564d3d68e87269ea1812cc94710ad06fe014faf093f01a6f89d0bd58719d3"
$runtimeBytes = 14148680
$runtimeSha256 = "114947d633e6844ce3c4b51ef6678f776628571d08a5763859c61642c8dcca9c"

$assetDirectory = Join-Path $PSScriptRoot "..\assets\onnxruntime\windows-x64"
$archive = Join-Path $assetDirectory "onnxruntime.download.zip"
$extracted = Join-Path $assetDirectory "onnxruntime.download"
$destination = Join-Path $assetDirectory "onnxruntime.dll"

New-Item -ItemType Directory -Force -Path $assetDirectory | Out-Null

try {
    Invoke-WebRequest -Uri $packageUrl -OutFile $archive
    $archiveItem = Get-Item -LiteralPath $archive
    if ($archiveItem.Length -ne $packageBytes) {
        throw "Unexpected package size: $($archiveItem.Length)"
    }
    $archiveHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
    if ($archiveHash -ne $packageSha256) {
        throw "Unexpected package SHA-256: $archiveHash"
    }

    Expand-Archive -LiteralPath $archive -DestinationPath $extracted
    $source = Join-Path $extracted "runtimes\win-x64\native\onnxruntime.dll"
    $runtimeItem = Get-Item -LiteralPath $source
    if ($runtimeItem.Length -ne $runtimeBytes) {
        throw "Unexpected runtime size: $($runtimeItem.Length)"
    }
    $runtimeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $source).Hash.ToLowerInvariant()
    if ($runtimeHash -ne $runtimeSha256) {
        throw "Unexpected runtime SHA-256: $runtimeHash"
    }
    Move-Item -Force -LiteralPath $source -Destination $destination
    Write-Host "Verified ONNX Runtime $runtimeVersion ($($runtimeItem.Length) bytes)"
}
finally {
    if (Test-Path -LiteralPath $archive) {
        Remove-Item -Force -LiteralPath $archive
    }
    if (Test-Path -LiteralPath $extracted) {
        Remove-Item -Recurse -Force -LiteralPath $extracted
    }
}
