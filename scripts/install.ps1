# Installs agent-usage-bar for your user account: it starts at every sign-in
# and sits in the notification area (explorer's or wdock's). Needs no admin.
#
# What it does:
#   1. builds the release binary (unless -NoBuild, or -Exe points at one),
#   2. copies it to %LOCALAPPDATA%\Programs\agent-usage-bar\agent-usage-bar.exe,
#   3. registers a per-user scheduled task "agent-usage-bar" that runs it at
#      sign-in, and starts it now unless -NoStart.
#
# A task rather than the HKCU Run key: run from a packaged (MSIX) app's
# terminal, HKCU registry writes are redirected into the package and explorer
# never sees a Run entry; task registration is not redirected.
#
# Undo with scripts\uninstall.ps1.
param([switch]$NoBuild, [switch]$NoStart, [string]$Exe)
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$built = if ($Exe) { $Exe } else { Join-Path $root 'target\release\agent-usage-bar.exe' }
$dir = Join-Path $env:LOCALAPPDATA 'Programs\agent-usage-bar'
$target = Join-Path $dir 'agent-usage-bar.exe'

if (-not $NoBuild -and -not $Exe) {
    $env:Path += ";$env:USERPROFILE\.cargo\bin"
    Push-Location $root
    try { cargo build --release; if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' } } finally { Pop-Location }
}
if (-not (Test-Path $built)) { throw "missing $built; build first or drop -NoBuild" }

# Replace a running copy.
Get-Process agent-usage-bar -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300

New-Item -ItemType Directory -Force $dir | Out-Null
Copy-Item $built $target -Force
$user = "$env:USERDOMAIN\$env:USERNAME"
$task = @{
    TaskName    = 'agent-usage-bar'
    Description = 'Starts agent-usage-bar (AI agent usage in the tray) at sign-in'
    Action      = New-ScheduledTaskAction -Execute $target
    Trigger     = New-ScheduledTaskTrigger -AtLogOn -User $user
    Principal   = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
    Settings    = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
        -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew
}
Register-ScheduledTask @task -Force | Out-Null

Write-Host "Installed $target; the 'agent-usage-bar' scheduled task starts it at every sign-in."
if (-not $NoStart) {
    Start-ScheduledTask -TaskName agent-usage-bar
    Write-Host 'agent-usage-bar started.'
}
