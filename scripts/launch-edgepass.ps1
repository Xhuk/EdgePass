# edgePass launcher (Windows)
# Builds a standalone .exe — no Vite/dev server needed at runtime.

param(
  [switch]$Dev,
  [switch]$BuildOnly,
  [switch]$Run
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if ($Dev) {
  npm run tauri dev
  exit $LASTEXITCODE
}

# Suelta el lock del artefacto: con el widget corriendo, el linker no puede
# sobrescribir target\release\edgepass.exe (+ edgepass_lib.dll) y falla con
# os error 5 "Acceso denegado".
$running = Get-Process -Name "edgepass" -ErrorAction SilentlyContinue
if ($running) {
  Write-Host "Deteniendo edgePass (bloquea el .exe de release)…"
  $running | Stop-Process -Force
  Start-Sleep -Seconds 1
}

Write-Host "Building edgePass release (binario embebido, sin Vite)…"
npm run tauri build
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$exe = Join-Path $root "src-tauri\target\release\edgepass.exe"
if (-not (Test-Path $exe)) {
  throw "Build OK but exe not found: $exe"
}

Write-Host "OK $exe"
if ($Run -or -not $BuildOnly) {
  Start-Process -FilePath $exe
}
