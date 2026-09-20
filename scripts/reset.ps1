# reset.ps1 — wipe Larra app data back to first-run state.
# Shelf folders in app data\library (and any older Documents\Larra) stay on disk.

$ErrorActionPreference = "SilentlyContinue"
$AppId = "io.larra.desktop"
$AppData = Join-Path $env:APPDATA $AppId

Write-Host "Stopping Larra…"
Get-Process -Name "larra", "llama-server" | Stop-Process -Force

Start-Sleep -Seconds 1

Write-Host "Removing application state…"
if (Test-Path $AppData) {
    Get-ChildItem -Force $AppData | Where-Object { $_.Name -ne "library" } | Remove-Item -Recurse -Force
}
Remove-Item -Recurse -Force "$env:LOCALAPPDATA\$AppId"

Write-Host ""
Write-Host "Done. Larra is back to first-run state —"
Write-Host "next launch shows onboarding and asks to install the AI model."
Write-Host "Kept on disk: Shelf folders with your files (library under app data)."
