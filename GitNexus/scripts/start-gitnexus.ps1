$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$projectRoot = Split-Path -Parent $scriptRoot
$sharedDir = Join-Path $projectRoot "gitnexus-shared"
$backendDir = Join-Path $projectRoot "gitnexus"
$webDir = Join-Path $projectRoot "gitnexus-web"
$indexDir = Join-Path $projectRoot ".gitnexus"

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

function Invoke-AnalyzeIfNeeded {
  if (Test-Path $indexDir) {
    Write-Host "Index already exists, skipping analyze." -ForegroundColor Yellow
    return
  }

  Write-Host ""
  Write-Host "Creating initial GitNexus index for this repository ..." -ForegroundColor Cyan
  Push-Location $backendDir
  try {
    & npx tsx src/cli/index.ts analyze $projectRoot
  }
  finally {
    Pop-Location
  }
}

function Start-Window {
  param(
    [Parameter(Mandatory = $true)][string]$Title,
    [Parameter(Mandatory = $true)][string]$WorkingDirectory,
    [Parameter(Mandatory = $true)][string]$Command
  )

  $escapedDir = $WorkingDirectory.Replace("'", "''")
  $payload = "Set-Location '$escapedDir'; `$host.UI.RawUI.WindowTitle = '$Title'; $Command"
  Start-Process powershell -ArgumentList "-NoExit", "-ExecutionPolicy", "Bypass", "-Command", $payload | Out-Null
}

Assert-Command -Name "node"
Assert-Command -Name "npm"
Assert-Command -Name "npx"
Assert-NodeVersion

Ensure-NpmInstall -Path $sharedDir
Ensure-NpmInstall -Path $backendDir
Ensure-NpmInstall -Path $webDir

Write-Host ""
Write-Host "Building shared package ..." -ForegroundColor Green
Invoke-NpmScript -Path $sharedDir -Script "build"

Invoke-AnalyzeIfNeeded

Write-Host ""
Write-Host "Starting backend and frontend ..." -ForegroundColor Green
Start-Window -Title "GitNexus Backend" -WorkingDirectory $backendDir -Command "npm run serve"
Start-Window -Title "GitNexus Web" -WorkingDirectory $webDir -Command "npm run dev"

Write-Host ""
Write-Host "GitNexus is starting." -ForegroundColor Green
Write-Host "Backend: http://127.0.0.1:4747" -ForegroundColor Green
Write-Host "Frontend: check the Vite window for the local URL (usually http://127.0.0.1:5173)" -ForegroundColor Green
Write-Host ""
Write-Host "If this is the first run, dependency install may take a few minutes." -ForegroundColor Yellow
