$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$projectRoot = Split-Path -Parent $scriptRoot
$sharedDir = Join-Path $projectRoot "gitnexus-shared"
$backendDir = Join-Path $projectRoot "gitnexus"
$webDir = Join-Path $projectRoot "gitnexus-web"
$releaseDir = Join-Path $projectRoot "release"
$stagingDir = Join-Path $releaseDir "gitnexus-build"
$timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
$zipPath = Join-Path $releaseDir "gitnexus-build-$timestamp.zip"

function Test-Command {
  param([Parameter(Mandatory = $true)][string]$Name)
  return $null -ne (Get-Command $Name -ErrorAction SilentlyContinue)
}

function Assert-Command {
  param([Parameter(Mandatory = $true)][string]$Name)
  if (-not (Test-Command -Name $Name)) {
    throw "Missing required command: $Name"
  }
}

function Assert-NodeVersion {
  $raw = (& node -v).Trim()
  if ($raw.StartsWith("v")) {
    $raw = $raw.Substring(1)
  }

  $version = [Version]$raw
  if ($version.Major -lt 20) {
    throw "Node.js 20+ required. Current version: $version"
  }
}

function Ensure-NpmInstall {
  param([Parameter(Mandatory = $true)][string]$Path)

  $nodeModules = Join-Path $Path "node_modules"
  if (Test-Path $nodeModules) {
    return
  }

  Write-Host ""
  Write-Host "Installing dependencies in $Path ..." -ForegroundColor Cyan
  Push-Location $Path
  try {
    & npm install
  }
  finally {
    Pop-Location
  }
}

function Invoke-NpmScript {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)][string]$Script
  )

  Push-Location $Path
  try {
    & npm run $Script
  }
  finally {
    Pop-Location
  }
}

function Copy-IfExists {
  param(
    [Parameter(Mandatory = $true)][string]$Source,
    [Parameter(Mandatory = $true)][string]$Destination
  )

  if (Test-Path $Source) {
    Copy-Item -Path $Source -Destination $Destination -Recurse -Force
  }
}

Assert-Command -Name "node"
Assert-Command -Name "npm"
Assert-NodeVersion

Ensure-NpmInstall -Path $sharedDir
Ensure-NpmInstall -Path $backendDir
Ensure-NpmInstall -Path $webDir

Write-Host ""
Write-Host "Building shared package ..." -ForegroundColor Green
Invoke-NpmScript -Path $sharedDir -Script "build"

Write-Host "Building backend package ..." -ForegroundColor Green
Invoke-NpmScript -Path $backendDir -Script "build"

Write-Host "Building web package ..." -ForegroundColor Green
Invoke-NpmScript -Path $webDir -Script "build"

New-Item -ItemType Directory -Force -Path $releaseDir | Out-Null
if (Test-Path $stagingDir) {
  Remove-Item -LiteralPath $stagingDir -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $stagingDir | Out-Null

$backendStage = Join-Path $stagingDir "gitnexus"
$webStage = Join-Path $stagingDir "gitnexus-web"
$sharedStage = Join-Path $stagingDir "gitnexus-shared"

New-Item -ItemType Directory -Force -Path $backendStage, $webStage, $sharedStage | Out-Null

Copy-IfExists -Source (Join-Path $backendDir "dist") -Destination $backendStage
Copy-IfExists -Source (Join-Path $webDir "dist") -Destination $webStage
Copy-IfExists -Source (Join-Path $sharedDir "dist") -Destination $sharedStage

Copy-IfExists -Source (Join-Path $backendDir "package.json") -Destination $backendStage
Copy-IfExists -Source (Join-Path $webDir "package.json") -Destination $webStage
Copy-IfExists -Source (Join-Path $sharedDir "package.json") -Destination $sharedStage
Copy-IfExists -Source (Join-Path $projectRoot "README.md") -Destination $stagingDir
Copy-IfExists -Source (Join-Path $projectRoot "RUNBOOK.md") -Destination $stagingDir

if (Test-Path $zipPath) {
  Remove-Item -LiteralPath $zipPath -Force
}
Compress-Archive -Path (Join-Path $stagingDir "*") -DestinationPath $zipPath -Force

Write-Host ""
Write-Host "Build complete." -ForegroundColor Green
Write-Host "Artifacts:" -ForegroundColor Green
Write-Host "  Backend dist: $backendDir\\dist"
Write-Host "  Web dist:     $webDir\\dist"
Write-Host "  Package zip:  $zipPath"
