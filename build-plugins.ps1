Write-Host "========================================="
Write-Host "Building IDE Plugins for AIndex Hub"
Write-Host "========================================="

# Scheme B: Integrated Packaging - Prepare bundles
Write-Host "`n[0/2] Preparing Integrated Bundles (Scheme B)..."
$bundleSrc = "$PSScriptRoot\deepcloud-bundle\windows-x64"

# 1. Prepare for VS Code
$vscodeBundleDest = "$PSScriptRoot\vscode-plugin\bundle"
if (Test-Path $vscodeBundleDest) { Remove-Item -Recurse -Force $vscodeBundleDest }
Copy-Item -Path $bundleSrc -Destination $vscodeBundleDest -Recurse -Force
Write-Host "Copied bundle to VS Code plugin."

# 2. Prepare for IDEA (Zip it so it's easy to extract from JAR)
$ideaResDir = "$PSScriptRoot\idea-plugin\src\main\resources"
if (-not (Test-Path $ideaResDir)) { New-Item -ItemType Directory -Force -Path $ideaResDir }
$ideaZipDest = "$ideaResDir\bundle.zip"
if (Test-Path $ideaZipDest) { Remove-Item -Force $ideaZipDest }
Compress-Archive -Path "$bundleSrc\*" -DestinationPath $ideaZipDest -Force
Write-Host "Zipped bundle to IDEA resources."

# Build VS Code Extension
Write-Host "`n[1/2] Building VS Code Extension..."
Push-Location "vscode-plugin"
npm install
npm run compile
# Install vsce globally if missing
if (-not (Get-Command vsce -ErrorAction SilentlyContinue)) {
    npm install -g @vscode/vsce
}
vsce package --no-dependencies
Pop-Location

# Build IDEA Plugin
Write-Host "`n[2/2] Building IntelliJ IDEA Plugin..."
Push-Location "idea-plugin"
# Use Gradle Wrapper to build plugin
if (-not (Test-Path "gradlew.bat")) {
    gradle wrapper
}
.\gradlew.bat buildPlugin
Pop-Location

Write-Host "`n========================================="
Write-Host "Build Complete!"
Write-Host "- VS Code Plugin: vscode-plugin/*.vsix"
Write-Host "- IDEA Plugin: idea-plugin/build/distributions/*.zip"
Write-Host "========================================="
