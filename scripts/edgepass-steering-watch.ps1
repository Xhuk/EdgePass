<#
.SYNOPSIS
    Watcher del steering: avisa cuando una nota nueva aparece o cambia en
    <root>\.reasonix\steering\ de un proyecto permitido (y con -Execute lanza
    `reasonix run` para que la consuma).

.DESCRIPTION
    Es la red de seguridad del hook del MCP: cubre lo que no pasa por el server
    — una nota copiada a mano, un board tocado por otro agente, un archivo
    restaurado. Los roots salen de la [roots] allow del toml del server, asi que
    lo que se vigila es exactamente lo que el comite puede leer: una sola fuente
    de verdad.

    Uso:
      .\edgepass-steering-watch.ps1                    # catch-up y despues vigila
      .\edgepass-steering-watch.ps1 -Once              # catch-up y sale
      .\edgepass-steering-watch.ps1 -Install           # tarea al logon (continua)
      .\edgepass-steering-watch.ps1 -Install -Execute  # y que ademas despierte

    Lo que ve lo delega en scripts\edgepass-steering-notify.ps1, que dedupea
    contra el hook: una nota se avisa una sola vez, la haya visto quien la haya
    visto.
#>
[CmdletBinding()]
param(
    [string[]]$Roots = @(),
    [string]$Config = '',
    [int]$DebounceMs = 1500,
    [switch]$Once,
    [switch]$Install,
    [switch]$Execute,
    [string]$TaskName = 'edgePass-steering-watch'
)

$ErrorActionPreference = 'Stop'
$notify = Join-Path $PSScriptRoot 'edgepass-steering-notify.ps1'
$logPath = Join-Path (Join-Path $env:APPDATA 'edgePass') 'steering-watch.log'

function Write-WatchLog([string]$Message) {
    # La tarea programada corre oculto: sin esto, un watcher vivo y uno muerto
    # se ven igual desde afuera.
    $line = '{0} {1}' -f (Get-Date).ToString('yyyy-MM-dd HH:mm:ss'), $Message
    try {
        $dir = Split-Path -Parent $logPath
        if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
        Add-Content -LiteralPath $logPath -Value $line -Encoding UTF8
    } catch { }
    Write-Host $line
}

# $PSScriptRoot esta vacio en los defaults del param cuando el script corre con
# -File, asi que el toml se resuelve aca y no alla arriba.
if ([string]::IsNullOrWhiteSpace($Config)) {
    $Config = Join-Path $PSScriptRoot '..\mcp\reasonixmcp\reasonixmcp.toml'
}
$Config = [IO.Path]::GetFullPath($Config)

function Strip-TomlComments([string]$Text) {
    # Los comentarios del toml muestran ejemplos, y uno de ellos es justamente
    # `#   allow = ["C:\\projects\\*"]`. Sin sacarlos, el watcher vigila todo
    # C:\proyectos, que es lo contrario de lo que la lista decide.
    $out = New-Object Text.StringBuilder
    $inString = $false
    for ($i = 0; $i -lt $Text.Length; $i++) {
        $c = $Text[$i]
        if ($c -eq '"') { $inString = -not $inString; [void]$out.Append($c); continue }
        if ($c -eq '#' -and -not $inString) {
            while ($i -lt $Text.Length -and $Text[$i] -ne "`n") { $i++ }
            [void]$out.Append("`n")
            continue
        }
        [void]$out.Append($c)
    }
    return $out.ToString()
}

function Get-AllowedRoots([string]$TomlPath) {
    if (-not (Test-Path -LiteralPath $TomlPath)) { return @() }
    $text = Strip-TomlComments (Get-Content -LiteralPath $TomlPath -Raw)
    $section = [regex]::Match($text, '(?ms)^\s*\[roots\]\s*$(.*?)(^\[|\z)')
    if (-not $section.Success) { return @() }
    $allow = [regex]::Match($section.Groups[1].Value, '(?ms)\ballow\s*=\s*\[(.*?)\]')
    if (-not $allow.Success) { return @() }
    $out = @()
    foreach ($m in [regex]::Matches($allow.Groups[1].Value, '"((?:[^"\\]|\\.)*)"')) {
        $entry = $m.Groups[1].Value -replace '\\\\', '\'
        if ($entry -match '[\*\?]') {
            # Un patron cubre una carpeta padre: se expande un nivel, igual que
            # el server. Sin coincidencias no se inventa nada.
            $out += @(Get-ChildItem -Path $entry -Directory -ErrorAction SilentlyContinue |
                ForEach-Object { $_.FullName })
            continue
        }
        $out += $entry
    }
    return @($out | Sort-Object -Unique)
}

function Get-KindFor([string]$File) {
    if ([IO.Path]::GetFileName($File) -eq 'board.jsonl') { return 'board' }
    return 'steering'
}

function Get-ProjectRootFor([string]$File) {
    # <root>\.reasonix\steering\<archivo> -> <root>
    $dir = Split-Path -Parent $File
    $dir = Split-Path -Parent $dir
    return (Split-Path -Parent $dir)
}

function Get-PendingNotes([string[]]$ProjectRoots) {
    $found = @()
    foreach ($root in $ProjectRoots) {
        $steering = Join-Path $root '.reasonix\steering'
        if (-not (Test-Path -LiteralPath $steering)) { continue }
        $found += @(Get-ChildItem -LiteralPath $steering -File -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -eq 'board.jsonl' -or $_.Extension -eq '.md' } |
            ForEach-Object { $_.FullName })
    }
    return @($found)
}

function Test-InterestingNote([string]$Candidate) {
    # El watcher ve todo el arbol .reasonix: solo importa una nota o el board.
    if ([string]::IsNullOrWhiteSpace($Candidate)) { return $false }
    if ($Candidate -notmatch '\\steering\\') { return $false }
    if ([IO.Path]::GetFileName($Candidate) -ne 'board.jsonl' -and [IO.Path]::GetExtension($Candidate) -ne '.md') { return $false }
    return Test-Path -LiteralPath $Candidate -PathType Leaf
}

if ($Install) {
    $taskArgs = @('-NoProfile', '-WindowStyle', 'Hidden', '-ExecutionPolicy', 'Bypass',
        '-File', $PSCommandPath, '-Config', $Config)
    if ($Execute) { $taskArgs += '-Execute' }
    $quoted = ($taskArgs | ForEach-Object { if ($_ -match '\s') { '"' + $_ + '"' } else { $_ } }) -join ' '
    $action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument $quoted
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
        -StartWhenAvailable -MultipleInstances IgnoreNew -ExecutionTimeLimit ([TimeSpan]::Zero)
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
    Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Settings $settings -Force | Out-Null
    Write-WatchLog "tarea registrada: $TaskName (arranca al logon)"
    Write-WatchLog "para arrancarla ahora: Start-ScheduledTask -TaskName $TaskName"
    return
}

if (-not $Roots -or $Roots.Count -eq 0) { $Roots = Get-AllowedRoots -TomlPath $Config }
if (-not $Roots -or $Roots.Count -eq 0) {
    Write-WatchLog "no hay roots que vigilar: revisa [roots] allow en $Config"
    return
}
Write-WatchLog ('vigilando {0} proyecto(s): {1}' -f $Roots.Count, ($Roots -join ', '))

foreach ($note in (Get-PendingNotes -ProjectRoots $Roots)) {
    & $notify -Path $note -Kind (Get-KindFor $note) -Root (Get-ProjectRootFor $note) -Source 'catchup' -Execute:$Execute
}
if ($Once) { return }

$watchers = @()
foreach ($root in $Roots) {
    $reasonixDir = Join-Path $root '.reasonix'
    if (-not (Test-Path -LiteralPath $reasonixDir)) {
        Write-WatchLog ('salteado: {0} (todavia no tiene .reasonix; el catch-up lo cubre)' -f $root)
        continue
    }
    $w = New-Object IO.FileSystemWatcher
    $w.Path = $reasonixDir
    $w.Filter = '*.*'
    $w.IncludeSubdirectories = $true
    $w.NotifyFilter = [IO.NotifyFilters]'FileName, LastWrite, Size, DirectoryName'
    $w.EnableRaisingEvents = $true
    $watchers += [pscustomobject]@{ Root = $root; Dir = $reasonixDir; Watcher = $w }
}
if ($watchers.Count -eq 0) {
    Write-WatchLog 'nada que vigilar (ningun root tiene .reasonix todavia)'
    return
}

Write-WatchLog 'esperando cambios (Ctrl+C para salir)'

# Los eventos se registran en vez de usar WaitForChanged: esa API se pierde
# cambios que ya estaban encolados (comprobado con una sonda), y una nota que se
# pierde es exactamente lo que este script existe para evitar.
$subs = @()
foreach ($entry in $watchers) {
    foreach ($eventName in 'Created', 'Changed', 'Renamed') {
        $subs += Register-ObjectEvent -InputObject $entry.Watcher -EventName $eventName `
            -SourceIdentifier ('edgepass.watch.{0}.{1}' -f ($entry.Dir -replace '[\\:]', '_'), $eventName)
    }
    $entry.Watcher.EnableRaisingEvents = $true
}

$pending = @{}
try {
    while ($true) {
        $ev = Wait-Event -Timeout 1
        if ($ev) {
            $changed = $ev.SourceEventArgs.FullPath
            Remove-Event -EventIdentifier $ev.EventIdentifier -ErrorAction SilentlyContinue
            if (Test-InterestingNote $changed) {
                # Agrupa la rafaga de una sola escritura (crear + cambiar tamano).
                $pending[$changed] = (Get-Date).AddMilliseconds($DebounceMs)
            }
        }
        if ($pending.Count -eq 0) { continue }
        $now = Get-Date
        foreach ($key in @($pending.Keys)) {
            if ($pending[$key] -gt $now) { continue }
            $pending.Remove($key)
            if (-not (Test-Path -LiteralPath $key)) { continue }
            & $notify -Path $key -Kind (Get-KindFor $key) -Root (Get-ProjectRootFor $key) -Source 'watch' -Execute:$Execute
        }
    }
} finally {
    foreach ($sub in $subs) { Unregister-Event -SourceIdentifier $sub.Name -ErrorAction SilentlyContinue }
    foreach ($entry in $watchers) {
        $entry.Watcher.EnableRaisingEvents = $false
        $entry.Watcher.Dispose()
    }
}
