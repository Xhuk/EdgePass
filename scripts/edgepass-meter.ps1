# Open edgePass's Meter tab on one secret, so the *human* can read and copy it.
#
# This is the "usa edge pass" path: it sends a path (or a query the widget
# resolves against its index) and the widget reads the fields itself. The reply
# carries names only — no value ever travels over the pipe, so whatever calls
# this (an agent) never ends up holding the secret.
#
# Examples:
#   powershell -File scripts\edgepass-meter.ps1 -Query "betty"
#   powershell -File scripts\edgepass-meter.ps1 -Namespace prod -Path "projects/app/users"
#   powershell -File scripts\edgepass-meter.ps1 -Query "infisi cb2" -Title "CB2 Infisical"

param(
  [string]$Query = "",
  [string]$Namespace = "",
  [string]$Path = "",
  [string]$Title = "",
  [string]$PipeName = "edgepass"
)

$ErrorActionPreference = "Stop"

if (-not $Query -and -not $Path) {
  throw "Falta -Query o -Path (ej: -Query 'betty')"
}

$payload = @{ op = "meter" }
if ($Path) {
  $payload.path = $Path.Trim().Trim("/")
  if ($Namespace) { $payload.namespace = $Namespace.Trim() }
} else {
  $payload.query = $Query.Trim()
}
if ($Title) { $payload.title = $Title.Trim() }

$json = ($payload | ConvertTo-Json -Compress -Depth 5) + "`n"
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
  exit 1
}

# Names only, by design: the values are on screen in the widget.
Write-Host ("Meter abierto · {0}/{1} — copia los valores ahí (ojo por fila · ⧉ todo)" -f $reply.namespace, $reply.path)
