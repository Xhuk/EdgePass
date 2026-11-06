<#
.SYNOPSIS
    Avisa cuando aparece o cambia una nota de steering, y opcionalmente despierta
    un agente para consumirla.

.DESCRIPTION
    Es la punta comun del loop. La usan las dos mitades:

      * el hook del server MCP (scripts\edgepass-mcp-hook.ps1), que se dispara
        cuando el comite escribe por HTTP o un agente escribe en el board por
        stdio;
      * el watcher de la carpeta (scripts\edgepass-steering-watch.ps1), que cubre
        lo que no pasa por el MCP: una nota copiada a mano, un board editado por
        otro agente, un archivo restaurado.

    Dedupea por ruta + tamano + mtime, asi que el que llega segundo no repite ni
    el aviso ni el disparo.

    Estado y evidencia, fuera del repo (nada de esto se versiona):
      %APPDATA%\edgePass\steering-notices.jsonl       una linea JSON por aviso
      %APPDATA%\edgePass\steering-notices.state.json  claves ya avisadas
      %APPDATA%\edgePass\steering-notices.log         texto, para mirar rapido

    Este script NO escribe dentro de .reasonix\steering\ ni se llama a si mismo:
    hacerlo despertaria al watcher y ese es justo el bucle que hay que evitar.

.PARAMETER Execute
    Ademas de avisar, lanza `reasonix run` en el proyecto para que la nota se
    consuma sola. Apagado por defecto: que un agente arranque solo es una
    decision, no un descuido.
#>
[CmdletBinding()]
param(
    [string]$Path = '',
    [string]$Kind = 'steering',
    [string]$Root = '',
    [string]$Tool = '',
    [string]$Source = 'manual',
    [switch]$Execute,
    [string]$Prompt = 'Read the new notes in .reasonix\steering\ and the board, then tell me what has to happen before touching anything.'
)

$ErrorActionPreference = 'Stop'

$stateDir = Join-Path $env:APPDATA 'edgePass'
$logPath = Join-Path $stateDir 'steering-notices.log'
$jsonlPath = Join-Path $stateDir 'steering-notices.jsonl'
$statePath = Join-Path $stateDir 'steering-notices.state.json'
$lockPath = Join-Path $stateDir 'steering-notices.lock'

New-Item -ItemType Directory -Force -Path $stateDir | Out-Null

function Write-NoticeLog([string]$Message) {
    $line = '{0} {1}' -f (Get-Date).ToString('yyyy-MM-dd HH:mm:ss'), $Message
    try { Add-Content -LiteralPath $logPath -Value $line -Encoding UTF8 } catch { }
    Write-Verbose $line
}

function Show-NoticeToast([string]$Title, [string]$Text) {
    # Best-effort: en una sesion no interactiva el toast no aparece y no pasa
    # nada, porque la evidencia queda en el .log y en el .jsonl.
    try {
        [void][Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime]
        [void][Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime]
        $template = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent(
            [Windows.UI.Notifications.ToastTemplateType]::ToastText02)
        $nodes = $template.GetElementsByTagName('text')
        [void]$nodes.Item(0).AppendChild($template.CreateTextNode($Title))
        [void]$nodes.Item(1).AppendChild($template.CreateTextNode($Text))
        $toast = New-Object Windows.UI.Notifications.ToastNotification $template
        # El AUMID de PowerShell: es el unico registrado que podemos usar sin
        # instalar nada. Con el nombre de la app, Windows descarta el toast.
        $aumid = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe'
        [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($aumid).Show($toast)
        Write-NoticeLog 'toast: mostrado'
    } catch {
        Write-NoticeLog ('toast: no se pudo mostrar ({0})' -f $_.Exception.Message)
    }
}

function Start-NoticeRun([string]$ProjectRoot) {
    $shim = Get-Command reasonix -ErrorAction SilentlyContinue
    if (-not $shim) {
        Write-NoticeLog 'Execute: reasonix no esta en el PATH, no se lanzo nada'
        return
    }
    # El CLI es un shim de npm (.ps1), asi que hay que pasarlo por un host.
    $args = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $shim.Source, 'run', '--add-dir', $ProjectRoot, $Prompt)
    try {
        Start-Process -FilePath 'powershell.exe' -ArgumentList $args -WorkingDirectory $ProjectRoot -WindowStyle Hidden
        Write-NoticeLog ('Execute: reasonix run lanzado en {0}' -f $ProjectRoot)
    } catch {
        Write-NoticeLog ('Execute: fallo el lanzamiento ({0})' -f $_.Exception.Message)
    }
}

if ([string]::IsNullOrWhiteSpace($Path)) {
    Write-NoticeLog 'ignorado: sin path'
    return
}
if (-not (Test-Path -LiteralPath $Path)) {
    Write-NoticeLog ('ignorado: {0} no existe' -f $Path)
    return
}
$file = Get-Item -LiteralPath $Path
if ($file.PSIsContainer) { return }

$key = '{0}|{1}|{2}' -f $file.FullName, $file.Length, $file.LastWriteTimeUtc.Ticks

# Estado con lock de archivo: el hook y el watcher pueden ver la misma escritura
# al mismo tiempo y no queremos dos avisos por una nota.
$lock = $null
for ($i = 0; $i -lt 40 -and -not $lock; $i++) {
    try { $lock = [IO.File]::Open($lockPath, 'OpenOrCreate', 'ReadWrite', 'None') }
    catch { Start-Sleep -Milliseconds 50 }
}
if (-not $lock) {
    Write-NoticeLog 'sin lock: otro aviso esta en curso, se saltea este para no duplicar'
    return
}
try {
    $keys = @()
    if (Test-Path -LiteralPath $statePath) {
        try {
            $parsed = (Get-Content -LiteralPath $statePath -Raw) | ConvertFrom-Json
            foreach ($item in @($parsed.keys)) {
                # Solo claves con la forma esperada. Si el archivo se ensucio
                # (ConvertTo-Json de un array de uno no es un array), se cae la
                # basura y se reescribe limpio; el peor caso es un aviso de mas.
                if ($item -is [string] -and $item -match '\|\d+\|\d+$') { $keys += $item }
            }
        } catch { $keys = @() }
    }
    if ($keys -contains $key) {
        Write-NoticeLog ('ya avisado: {0}' -f $file.Name)
        return
    }
    $keys += $key
    if ($keys.Count -gt 400) { $keys = $keys[($keys.Count - 400)..($keys.Count - 1)] }
    $payload = @{ keys = [string[]]$keys } | ConvertTo-Json -Compress -Depth 3
    [IO.File]::WriteAllText($statePath, $payload, (New-Object Text.UTF8Encoding($false)))
} finally {
    if ($lock) { $lock.Close(); $lock.Dispose() }
}

$project = if ($Root) { $Root } else { Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $file.FullName)) }
$notice = [ordered]@{
    at     = (Get-Date).ToString('o')
    kind   = $Kind
    source = $Source
    tool   = $Tool
    root   = $project
    path   = $file.FullName
    name   = $file.Name
    bytes  = $file.Length
}
Add-Content -LiteralPath $jsonlPath -Value ($notice | ConvertTo-Json -Compress) -Encoding UTF8
Write-NoticeLog ('aviso: kind={0} source={1} tool={2} name={3}' -f $Kind, $Source, $Tool, $file.Name)

Show-NoticeToast -Title ('edgePass: {0} nueva' -f $Kind) -Text ('{0} - {1}' -f (Split-Path -Leaf $project), $file.Name)

if ($Execute) { Start-NoticeRun -ProjectRoot $project }
