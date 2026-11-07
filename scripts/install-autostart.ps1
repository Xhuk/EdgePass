# Install edgePass to start at user login, after OpenBao service is up.
# Not a Windows Service (GUI apps can't live in Session 0) — Scheduled Task at logon.

param(
  [switch]$Remove,
  [string]$ExePath = ""
)

$ErrorActionPreference = "Stop"
$taskName = "edgePass"

if (-not $ExePath) {
  $cand = @(
    (Join-Path $PSScriptRoot "..\src-tauri\target\release\edgepass.exe"),
    (Join-Path $env:LOCALAPPDATA "edgePass\edgepass.exe"),
    (Join-Path $env:LOCALAPPDATA "Programs\edgePass\edgepass.exe"),
    (Join-Path $env:USERPROFILE "scoop\apps\edgepass\current\edgepass.exe")
  ) | ForEach-Object { [IO.Path]::GetFullPath($_) }
  $ExePath = $cand | Where-Object { Test-Path $_ } | Select-Object -First 1
}

if ($Remove) {
  Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
  Write-Host "Removed scheduled task '$taskName'"
  exit 0
}

if (-not $ExePath -or -not (Test-Path -LiteralPath $ExePath)) {
  throw "edgepass.exe not found. Build first: npm run tauri build"
}

# Native task action: no PowerShell wrapper and no -ExecutionPolicy Bypass.
# OpenBao runs as its own service; a vault that is still starting is a state the
# widget renders ("OpenBao offline" plus the Svc button), so waiting for it here
# was not load-bearing. The 8 s delay is for the logon burst.
$action = New-ScheduledTaskAction -Execute $ExePath
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
$trigger.Delay = 'PT8S'
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -MultipleInstances IgnoreNew -ExecutionTimeLimit ([TimeSpan]::Zero)
Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Settings $settings -Force | Out-Null

Write-Host "OK: task '$taskName' at logon (sin reintentos automaticos)"
Write-Host "  exe: $ExePath"
Write-Host "  Remove with: powershell -File $($MyInvocation.MyCommand.Path) -Remove"
