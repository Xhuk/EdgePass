<#
.SYNOPSIS
    Receptor del hook del server MCP: reasonixmcp lo llama despues de cada
    escritura — una nota de steering del comite (HTTP) o un mensaje en el board
    de un agente (stdio).

.DESCRIPTION
    El evento llega dos veces y este script acepta las dos: por stdin como JSON
    (lo que quiere un receptor de verdad) y por variables REASONIXMCP_HOOK_*
    (lo que se lee sin parser). No decide nada: traduce y delega en el
    notificador compartido, que dedupea contra el watcher.

    Se registra en reasonixmcp.toml (local, gitignored):

        [hooks]
        on_write = "powershell -NoProfile -ExecutionPolicy Bypass -File C:\proyectos\edgePass\scripts\edgepass-mcp-hook.ps1"

    Y con -Execute al final del comando, ademas de avisar lanza `reasonix run`
    en el proyecto.

    Nunca falla hacia arriba: si algo falta, sale 0 y sin ruido. El server loguea
    la salida del hook, pero una escritura ya hecha no se pierde por esto.
#>
[CmdletBinding()]
param([switch]$Execute)

$ErrorActionPreference = 'Stop'

$notify = Join-Path $PSScriptRoot 'edgepass-steering-notify.ps1'
if (-not (Test-Path -LiteralPath $notify)) { exit 0 }

# stdin primero: es el contrato principal del hook.
$event = $null
try {
    $raw = [Console]::In.ReadToEnd()
    if ($raw -and $raw.Trim()) { $event = $raw | ConvertFrom-Json }
} catch { $event = $null }

$kind = if ($event) { $event.kind } else { $env:REASONIXMCP_HOOK_KIND }
$path = if ($event) { $event.path } else { $env:REASONIXMCP_HOOK_PATH }
$root = if ($event) { $event.root } else { $env:REASONIXMCP_HOOK_ROOT }
$tool = if ($event) { $event.tool } else { $env:REASONIXMCP_HOOK_TOOL }

if ([string]::IsNullOrWhiteSpace($path)) { exit 0 }

& $notify -Path $path -Kind $kind -Root $root -Tool $tool -Source 'mcp' -Execute:$Execute
exit 0
