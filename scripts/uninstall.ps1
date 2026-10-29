# Stops agent-usage-bar and removes the sign-in task install.ps1 registered.
# -RemoveFiles also deletes the installed binary; -RemoveData deletes
# %LOCALAPPDATA%\agent-usage-bar (config, status.json, log and the
# DPAPI-encrypted Claude sign-in).
param([switch]$RemoveFiles, [switch]$RemoveData)
$ErrorActionPreference = 'Stop'

$dir = Join-Path $env:LOCALAPPDATA 'Programs\agent-usage-bar'
$data = Join-Path $env:LOCALAPPDATA 'agent-usage-bar'

if (Get-ScheduledTask -TaskName agent-usage-bar -ErrorAction SilentlyContinue) {
    Unregister-ScheduledTask -TaskName agent-usage-bar -Confirm:$false
    Write-Host 'Removed the agent-usage-bar sign-in task.'
}
Get-Process agent-usage-bar -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300

if ($RemoveFiles -and (Test-Path $dir)) {
    Remove-Item -Recurse -Force $dir
    Write-Host "Removed $dir."
}
if ($RemoveData -and (Test-Path $data)) {
    Remove-Item -Recurse -Force $data
    Write-Host "Removed $data."
}
