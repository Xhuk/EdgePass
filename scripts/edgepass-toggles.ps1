# Read, rewrite or fire edgePass's header toggles over the local named pipe.
#
# A toggle is data: { id, label, probe, on, off, alt }. This script is the door to
# the same list Ajustes edits (config.json -> toggles[]) and to the same click the
# header fires, so there is nothing to reimplement here.
#
# The vocabulary is published, never guessed: -Catalog prints every probe and
# action with what it does, and every reply carries it too. An unknown id comes
# back as an error *with* that catalog.
#
# Examples:
#   powershell -File scripts\edgepass-toggles.ps1 -List
#   powershell -File scripts\edgepass-toggles.ps1 -Catalog
#   powershell -File scripts\edgepass-toggles.ps1 -Fire kv              # flip
#   powershell -File scripts\edgepass-toggles.ps1 -Fire kv -Dir off
#   powershell -File scripts\edgepass-toggles.ps1 -Set (Get-Content .\toggles.json -Raw)
#   powershell -File scripts\edgepass-toggles.ps1 -Services
#   powershell -File scripts\edgepass-toggles.ps1 -SetServices (Get-Content .\services.json -Raw)

param(
  [switch]$List,
  [switch]$Catalog,
  [string]$Fire = "",
  # flip (default) | on | off | alt
  [string]$Dir = "flip",
  # Full replacement list, as a JSON array of specs.
  [string]$Set = "",
  # The owner's services: read, or replace the whole list (`services[]`).
  [switch]$Services,
  [string]$SetServices = "",
  [string]$PipeName = "edgepass"
)

$ErrorActionPreference = "Stop"

if ($Fire) {
  $payload = @{ op = "toggle"; id = $Fire.Trim(); dir = $Dir.Trim() }
} elseif ($SetServices) {
  # Same contract as -Set: the whole list, validated by Rust.
  $svcs = $SetServices | ConvertFrom-Json
  if ($svcs -isnot [array]) { $svcs = @($svcs) }
  $payload = @{ op = "services"; set = $svcs }
} elseif ($Services) {
  $payload = @{ op = "services" }
} elseif ($Set) {
  # The whole list; Rust validates ids against the catalog and caps the count.
  $specs = $Set | ConvertFrom-Json
  if ($specs -isnot [array]) { $specs = @($specs) }
  $payload = @{ op = "toggles"; set = $specs }
} else {
  $payload = @{ op = "toggles" }
}

$json = ($payload | ConvertTo-Json -Compress -Depth 8) + "`n"
$bytes = [Text.Encoding]::UTF8.GetBytes($json)

$pipe = New-Object System.IO.Pipes.NamedPipeClientStream(".", $PipeName, [IO.Pipes.PipeDirection]::InOut)
try {
  $pipe.Connect(3000)
  $pipe.Write($bytes, 0, $bytes.Length)
  $pipe.Flush()
  $reader = New-Object IO.StreamReader($pipe, [Text.Encoding]::UTF8, $false, 1024, $true)
  $line = $reader.ReadLine()
} finally {
  $pipe.Dispose()
}

$reply = $line | ConvertFrom-Json
if (-not $reply.ok) {
  Write-Error ("edgePass: " + $reply.error)
  # The refusal carries the vocabulary: print it so the next call can be right.
  if ($reply.catalog) {
    Write-Host "probes :" ($reply.catalog.probes.id -join ", ")
    Write-Host "actions:" ($reply.catalog.actions.id -join ", ")
  }
  if ($reply.ids) { Write-Host "LEDs   :" ($reply.ids -join ", ") }
  exit 1
}

if ($Fire) {
  Write-Host ("{0} -> {1} · {2}" -f $reply.id, $reply.action, $reply.toast)
  exit 0
}

if ($Services -or $SetServices) {
  Write-Host ("services ({0}/{1}):" -f $reply.services.Count, $reply.max)
  foreach ($s in $reply.services) {
    Write-Host ("  {0,-16} {1,-6} probe={2}" -f $s.id, $s.label, $s.probe.kind)
    if ($s.probe.image) {
      $match = if ($s.probe.match) { " match=" + $s.probe.match } else { "" }
      Write-Host ("  {0,-16} image={1}{2}" -f "", $s.probe.image, $match)
    }
    if ($s.probe.cmd) { Write-Host ("  {0,-16} cmd={1}" -f "", ($s.probe.cmd -join " ")) }
  }
  if ($reply.leds) { Write-Host ("LEDs que las usan: " + ($reply.leds -join ", ")) }
  Write-Host ""
  Write-Host "probes svc.*:"
  foreach ($p in $reply.probes) {
    Write-Host ("  {0,-18} on={1} off={2}" -f $p.id, $p.on, $p.off)
  }
  exit 0
}

Write-Host ("toggles ({0}/{1}) · label = id, probe, on/off:" -f $reply.toggles.Count, $reply.max)
foreach ($t in $reply.toggles) {
  $state = if ($t.state.on) { "on " } else { "off" }
  $flags = @()
  if ($t.state.warn) { $flags += "warn" }
  if ($t.state.ask) { $flags += "ask" }
  if (-not $t.state.live) { $flags += "no-click" }
  $suffix = if ($flags.Count) { " [" + ($flags -join " ") + "]" } else { "" }
  $alt = if ($t.alt) { " · alt=" + $t.alt } else { "" }
  Write-Host ("  {0,-4} {1}  {2}  {3} / {4}{5}{6}" -f $t.label, $state, $t.probe, $t.on, $t.off, $alt, $suffix)
  Write-Host ("       probe: {0}" -f $t.state.detail)
}

if ($Catalog -or -not $List) {
  Write-Host ""
  Write-Host ("probes (max {0} toggles, label <= {1} chars):" -f $reply.catalog.maxToggles, $reply.catalog.maxLabelChars)
  foreach ($p in $reply.catalog.probes) {
    Write-Host ("  {0,-18} on={1} off={2} alt={3}" -f $p.id, $p.on, $p.off, $p.alt)
    Write-Host ("  {0,-18} {1}" -f "", $p.doc)
  }
  Write-Host "actions:"
  foreach ($a in $reply.catalog.actions) {
    Write-Host ("  {0,-18} {1}" -f $a.id, $a.doc)
  }
}
