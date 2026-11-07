# Copy edgePass's agent skills into a skills hub.
#
# The installer ships the app, not the instructions: an agent needs the Markdown,
# and the binary is not where it looks. The default destination is the shared
# skills hub (`~/.agents/skills`); point `-Dest` at your agent's own folder if you
# do not use a hub. `-Force` replaces an older copy.
#
# Examples:
#   powershell -File scripts\install-skills.ps1
#   powershell -File scripts\install-skills.ps1 -Dest "$HOME\.claude\skills" -Force

param(
  [string]$Dest = (Join-Path $HOME ".agents\skills"),
  [switch]$Force
)

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$names = @("edgepass", "edgepass-led", "openbao-bootstrap")

New-Item -ItemType Directory -Force -Path $Dest | Out-Null
$copied = 0
foreach ($name in $names) {
  $src = Join-Path $root "skills\$name"
  if (-not (Test-Path (Join-Path $src "SKILL.md"))) {
    throw "missing $src\SKILL.md"
  }
  $dst = Join-Path $Dest $name
  if (Test-Path $dst) {
    if (-not $Force) {
      Write-Host "exists: $dst (use -Force to replace)"
      continue
    }
    Remove-Item -Recurse -Force $dst
  }
  Copy-Item -Recurse -Force $src $dst
  Write-Host "installed: $name -> $dst"
  $copied++
}

Write-Host ""
Write-Host "Done ($copied skill(s)). Tell your agent: «usa edge pass»."
Write-Host "If you use a hub with links (skills_sync), run your sync after this."
