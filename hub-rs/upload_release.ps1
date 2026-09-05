$ErrorActionPreference = 'Stop'
Write-Host "Zipping artifacts..."
Compress-Archive -Path target\release\hub.exe -DestinationPath deepcloud-hub-windows-amd64.zip -Force
if (Test-Path target\release\superagent.exe) {
    Compress-Archive -Path target\release\superagent.exe -DestinationPath superagent-windows-amd64.zip -Force
}
Write-Host "Uploading to GitHub Releases..."
gh release create v0.2.0 deepcloud-hub-windows-amd64.zip superagent-windows-amd64.zip --title 'DeepCloud Hub v0.2.0 (Architecture Fixes)' --notes 'Includes Workspace Isolation, Double-phase commit, Web Interaction Adapter, and Security updates.'
Write-Host "Done!"
