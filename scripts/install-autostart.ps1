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
    (Join-Path $env:LOCALAPPDATA "Programs\edgePass\edgepass.exe")
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

$wrapper = Join-Path $env:LOCALAPPDATA "edgePass\start-with-openbao.ps1"
New-Item -ItemType Directory -Force -Path (Split-Path $wrapper) | Out-Null

@'
$ErrorActionPreference = "SilentlyContinue"
$log = Join-Path $env:LOCALAPPDATA "edgePass\autostart.log"
function Write-Log($m) { "$(Get-Date -Format s) $m" | Out-File -FilePath $log -Append -Encoding UTF8 }

# OpenBao corre como servicio propio: aquí solo esperamos a que esté arriba
# (max ~60s) y lanzamos el widget. No gestionamos su ciclo de vida.
for ($i = 0; $i -lt 30; $i++) {
  $svc = Get-Service -Name OpenBao -ErrorAction SilentlyContinue
  if ($svc -and $svc.Status -eq "Running") { break }
  Start-Sleep -Seconds 2
}
# Give API a moment, then launch widget
Start-Sleep -Seconds 2

# Una sola instancia: si ya corre, no lanzamos otra.
if (Get-Process -Name "edgepass" -ErrorAction SilentlyContinue) {
  Write-Log "edgePass ya estaba en ejecucion; no se lanza otra instancia"
  exit 0
}
Write-Log "lanzando edgePass"
Start-Process -FilePath "__EXE__"
'@.Replace("__EXE__", $ExePath.Replace("'", "''")) | Set-Content -Path $wrapper -Encoding UTF8

$action = New-ScheduledTaskAction -Execute "powershell.exe" -Argument "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$wrapper`""
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
# Sin -RestartInterval/-RestartCount: la tarea es "lanzar una vez al logon".
# Reintentar cada minuto relanzaba el wrapper en bucle. Tampoco matamos el
# widget por límite de tiempo (-ExecutionTimeLimit 0 = sin límite) y evitamos
# instancias duplicadas.
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -MultipleInstances IgnoreNew -ExecutionTimeLimit ([TimeSpan]::Zero)
Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Settings $settings -Force | Out-Null

Write-Host "OK: task '$taskName' at logon (sin reintentos automaticos)"
Write-Host "  exe: $ExePath"
Write-Host "  espera al servicio Windows 'OpenBao' (que corre por su cuenta) y lanza edgePass una vez"
Write-Host "  log: $env:LOCALAPPDATA\edgePass\autostart.log"
Write-Host "Remove with: powershell -File $($MyInvocation.MyCommand.Path) -Remove"
