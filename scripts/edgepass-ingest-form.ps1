# Push an ingest form into the running edgePass widget (LLM / agent).
# Does NOT send secret values — only path + variable names.
#
# Example:
#   powershell -File scripts\edgepass-ingest-form.ps1 `
#     -Namespace buenaidee -Path "projects/cb2/infisical" `
#     -Keys DATABASE_URL,DB_USER,DB_PASSWORD,API_TOKEN `
#     -Title "CB2 Infisical"

param(
  [string]$Namespace = ".",
  [Parameter(Mandatory = $true)][string]$Path,
  [Parameter(Mandatory = $true)][string[]]$Keys,
  [string]$Title = "",
  [string]$PipeName = "\\.\pipe\edgepass"
)

$ErrorActionPreference = "Stop"

$ns = if ($Namespace -eq ".") { "" } else { $Namespace.Trim() }
$payload = @{
  op        = "ingest-form"
  namespace = $ns
  path      = $Path.Trim().Trim("/")
  keys      = @($Keys | ForEach-Object { "$_".Trim() } | Where-Object { $_ })
}
if ($Title) { $payload.title = $Title }

$json = ($payload | ConvertTo-Json -Compress -Depth 5) + "`n"
$bytes = [Text.Encoding]::UTF8.GetBytes($json)

$pipe = New-Object System.IO.Pipes.NamedPipeClientStream(".", "edgepass", [IO.Pipes.PipeDirection]::InOut)
try {
  $pipe.Connect(3000)
  $pipe.Write($bytes, 0, $bytes.Length)
  $pipe.Flush()
  $reader = New-Object IO.StreamReader($pipe, [Text.Encoding]::UTF8, $false, 1024, $true)
  $line = $reader.ReadLine()
  Write-Host $line
} finally {
  $pipe.Dispose()
}
