# edgePass en modo dev: HMR del frontend + recompilación rápida del Rust.
# Use: .\dev-edgepass.ps1
#
# Por qué un script y no `npm run tauri dev` a secas: edgePass es de **instancia
# única a propósito** (lib.rs, "single instance"; ipc.rs: dos widgets serían dos
# guardianes, dos handles de SQLite y dos servidores del pipe `\\.\pipe\edgepass`),
# así que el release de uso diario y el dev **no pueden convivir**. Este script
# baja el release antes de arrancar.

param(
  # Dejá el release corriendo si sabés que querés quedarte con el dev y ya lo bajaste.
  [switch]$KeepRelease,
  # Arranca OpenBao si el servicio está parado (igual que restart-edgepass.ps1).
  [switch]$StartOpenBao
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not $KeepRelease) {
  $running = Get-Process -Name "edgepass" -ErrorAction SilentlyContinue
  if ($running) {
    Write-Host "Bajando el edgePass de uso diario (instancia única)..."
    $running | Stop-Process -Force
    Start-Sleep -Seconds 1
  }
}

if ($StartOpenBao) {
  $svc = Get-Service -Name "OpenBao" -ErrorAction SilentlyContinue
  if ($svc -and $svc.Status -ne "Running") {
    Write-Host "Arrancando el servicio OpenBao..."
    Start-Service -Name "OpenBao" -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 3
  }
}

# `tauri dev` corre `beforeDevCommand` (npm run dev) y abre la ventana contra
# http://localhost:1420. Entonces:
#   - editar src/App.tsx o src/App.css  -> HMR, se ve al instante, sin recompilar;
#   - editar src-tauri/src/*.rs         -> recompila en perfil **dev** (sin LTO ni
#     codegen-units 1) y reinicia la ventana: decenas de segundos, no minutos.
#
# El `.exe` de release NO queda bloqueado por esta instancia (corre desde
# target\debug), así que se puede compilar el release en paralelo.
Write-Host "Arrancando tauri dev (Ctrl+C para salir)..."
npm run tauri dev
