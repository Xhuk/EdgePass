# Mirror `docs/` into the GitHub wiki.
#
# `docs/` is the source of truth: it is versioned with the code and reviewed in
# the same pull request. The wiki is a convenience for browsing, so every page
# says where it came from and this script only ever writes in one direction.
#
# The wiki git repository does not exist until the first page is saved in the
# GitHub UI (Wiki -> Create the first page). After that:
#
#   powershell -File scripts\sync-wiki.ps1
#   powershell -File scripts\sync-wiki.ps1 -WhatIf     # show the plan only

param(
  [string]$Repo = "Xhuk/EdgePass",
  [string]$Message = "docs: sync from the repository",
  [switch]$WhatIf
)

$ErrorActionPreference = "Continue"
$root = Split-Path $PSScriptRoot -Parent
$docs = Join-Path $root "docs"
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("edgepass-wiki-" + [Guid]::NewGuid().ToString("N").Substring(0, 8))

$pages = Get-ChildItem $docs -Filter *.md |
  Where-Object { $_.Name -ne "README.md" } |
  Sort-Object Name

if ($pages.Count -eq 0) { throw "no markdown in $docs" }

Write-Host "pages: $($pages.Count)"
foreach ($p in $pages) { Write-Host "  $($p.Name) -> $([IO.Path]::GetFileNameWithoutExtension($p.Name))" }

if ($WhatIf) { Write-Host "(dry run: nothing pushed)"; return }

git clone "https://github.com/$Repo.wiki.git" $tmp 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) {
  throw "the wiki repository does not exist yet: open https://github.com/$Repo/wiki and save a first page, then run this again"
}

$index = New-Object System.Collections.Generic.List[string]
$index.Add("# edgePass wiki")
$index.Add("")
$index.Add("Browsable mirror of the repository's ``docs/`` folder. The repository is the source of truth.")
$index.Add("")
foreach ($p in $pages) {
  $name = [IO.Path]::GetFileNameWithoutExtension($p.Name)
  $body = Get-Content $p.FullName -Raw
  $banner = "> **Generated from ``docs/$($p.Name)`` in the repository.** Do not edit this page:`n> change the file in ``docs/`` and run ``scripts\sync-wiki.ps1``.`n`n"
  Set-Content -Path (Join-Path $tmp "$name.md") -Value ($banner + $body) -Encoding UTF8
  $index.Add("- [$name]($name)")
}
Set-Content -Path (Join-Path $tmp "Home.md") -Value ($index -join "`n") -Encoding UTF8

Push-Location $tmp
try {
  git add -A
  git -c user.name="Xhuk" -c user.email="xhuk@users.noreply.github.com" commit -m $Message 2>$null
  if ($LASTEXITCODE -ne 0) { Write-Host "nothing to commit"; return }
  git push origin HEAD 2>&1 | Out-Null
  if ($LASTEXITCODE -ne 0) { throw "git push failed" }
  Write-Host "wiki updated: https://github.com/$Repo/wiki"
  exit 0
} finally {
  Pop-Location
  Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
