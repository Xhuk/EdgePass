# Install a Start Menu shortcut so edgePass can be launched (and pinned)
# from the apps bar. Points at the standalone release exe, not tauri dev.
# Same exe-resolution as install-autostart.ps1.

param(
  [switch]$Remove,
  [switch]$Desktop,
  [string]$ExePath = ""
)

$ErrorActionPreference = "Stop"
$shortcutName = "edgePass.lnk"
$startMenuDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
$startMenuLnk = Join-Path $startMenuDir $shortcutName
$desktopLnk = Join-Path ([Environment]::GetFolderPath("Desktop")) $shortcutName

function Get-EdgePassExe {
  param([string]$Hint)
  if ($Hint) {
    $full = [IO.Path]::GetFullPath($Hint)
    if (Test-Path -LiteralPath $full) { return $full }
  }
  $cand = @(
    (Join-Path $PSScriptRoot "..\src-tauri\target\release\edgepass.exe"),
    (Join-Path $env:LOCALAPPDATA "Programs\edgePass\edgepass.exe")
  ) | ForEach-Object { [IO.Path]::GetFullPath($_) }
  return $cand | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
}

function New-EdgePassShortcut {
  param(
    [Parameter(Mandatory)][string]$LinkPath,
    [Parameter(Mandatory)][string]$Target
  )
  $dir = Split-Path -Parent $LinkPath
  if (-not (Test-Path -LiteralPath $dir)) {
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
  }
  $shell = New-Object -ComObject WScript.Shell
  $sc = $shell.CreateShortcut($LinkPath)
  $sc.TargetPath = $Target
  $sc.WorkingDirectory = Split-Path -Parent $Target
  $sc.IconLocation = "$Target,0"
  $sc.Description = "edgePass - widget de secretos OpenBao"
  $sc.WindowStyle = 1
  $sc.Save()
  [System.Runtime.InteropServices.Marshal]::ReleaseComObject($sc) | Out-Null
  [System.Runtime.InteropServices.Marshal]::ReleaseComObject($shell) | Out-Null
}

function Try-PinToTaskbar {
  param([Parameter(Mandatory)][string]$LinkPath)
  # Windows 10/11 often hide this verb; never fail the install if pin is refused.
  $folderPath = Split-Path -Parent $LinkPath
  $fileName = Split-Path -Leaf $LinkPath
  $shell = New-Object -ComObject Shell.Application
  $folder = $shell.Namespace($folderPath)
  if (-not $folder) { return $false }
  $item = $folder.ParseName($fileName)
  if (-not $item) { return $false }
  $verb = $item.Verbs() | Where-Object {
    ($_.Name -replace '&', '') -match '(?i)tarea|taskbar|Anclar a la barra|Pin to taskbar'
  } | Select-Object -First 1
  if (-not $verb) { return $false }
  $verb.DoIt()
  return $true
}

if ($Remove) {
  foreach ($p in @($startMenuLnk, $desktopLnk)) {
    if (Test-Path -LiteralPath $p) {
      Remove-Item -LiteralPath $p -Force
      Write-Host "Removed $p"
    }
  }
  Write-Host "OK: shortcut(s) quitados"
  exit 0
}

$ExePath = Get-EdgePassExe -Hint $ExePath
if (-not $ExePath) {
  throw "edgepass.exe not found. Build first: powershell -File scripts\launch-edgepass.ps1 -BuildOnly"
}

New-Item -ItemType Directory -Force -Path $startMenuDir | Out-Null
New-EdgePassShortcut -LinkPath $startMenuLnk -Target $ExePath
Write-Host "OK: Start Menu"
Write-Host "  $startMenuLnk"
Write-Host "  exe: $ExePath"

if ($Desktop) {
  New-EdgePassShortcut -LinkPath $desktopLnk -Target $ExePath
  Write-Host "OK: Desktop"
  Write-Host "  $desktopLnk"
}

$pinned = $false
try { $pinned = Try-PinToTaskbar -LinkPath $startMenuLnk } catch { $pinned = $false }
if ($pinned) {
  Write-Host "OK: anclado a la barra de tareas"
} else {
  Write-Host "Para anclarlo: Inicio > edgePass > clic derecho > Anclar a la barra de tareas"
}

Write-Host "Remove with: powershell -File $($MyInvocation.MyCommand.Path) -Remove"
