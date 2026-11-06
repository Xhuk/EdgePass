# Restart edgePass (kill if running, then launch)
# Use: .\restart-edgepass.ps1

param(
  [switch]$StartOpenBao
)

$ErrorActionPreference = "Stop"
# El script vive en scripts\: la raiz del repo es su carpeta padre (igual que launch-edgepass.ps1).
# Sin esto buscaba scripts\src-tauri\target\release\edgepass.exe y nunca encontraba el binario.
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$exe = Join-Path $root "src-tauri\target\release\edgepass.exe"
if (-not (Test-Path $exe)) {
  Write-Host "edgepass.exe not found. Build first: .\launch-edgepass.ps1"
  exit 1
}

# Kill existing instance
$running = Get-Process -Name "edgepass" -ErrorAction SilentlyContinue
if ($running) {
  Write-Host "Stopping edgePass..."
  $running | Stop-Process -Force
  Start-Sleep -Seconds 1
}

# Start OpenBao if requested or if not running
if ($StartOpenBao -or -not (Get-Service -Name "OpenBao" -ErrorAction SilentlyContinue)) {
  $svc = Get-Service -Name "OpenBao" -ErrorAction SilentlyContinue
  if ($svc -and $svc.Status -ne "Running") {
    Write-Host "Starting OpenBao service..."
    Start-Service -Name "OpenBao" -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 3
  }
}

# Launch edgePass
Write-Host "Launching edgePass..."
Start-Process -FilePath $exe
Write-Host "Done. edgePass is running."
