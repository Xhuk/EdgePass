<#
.SYNOPSIS
  Runner local de GitHub Actions (imagen gha-local) - el camino manual.

.DESCRIPTION
  El LED GW de edgePass hace todo esto solo (vigila Docker Desktop + el contenedor y lo
  recrea cuando la credencial vence). Este script existe para cuando NO quieres abrir el
  widget: registrar un repo, construir la imagen, levantar/parar el runner y ver el log.

  La receta es la misma en los dos caminos (env RUNNER_*, volumen de trabajo, socket de
  Docker): lo que hace el widget esta en src-tauri/src/gha.rs (run_args) y lo que hace
  este script, aca abajo. No hay un tercer lugar.

  La fila del runner vive en config.json del widget (ghaRunners[]), asi que agregarlo con
  -Add hace que el LED GW lo vigile en el proximo poll (5 s). Sin fila, el LED dice
  "sin runners declarados" en ambar, a proposito: una luz verde sobre la nada miente.

.EXAMPLE
  powershell -File scripts\gha-local.ps1 -Status
  powershell -File scripts\gha-local.ps1 -Add Xhuk/ContinuityBridge2.0
  powershell -File scripts\gha-local.ps1 -Build
  powershell -File scripts\gha-local.ps1 -Ensure
  powershell -File scripts\gha-local.ps1 -Logs -Id cb2 -Follow
  powershell -File scripts\gha-local.ps1 -Down
#>
[CmdletBinding()]
param(
  [switch]$Status,
  [string]$Add,
  [string]$Remove,
  [string]$Id,
  [string]$Labels = "self-hosted,Linux,X64,local",
  [string]$Image = "gha-local:latest",
  [string[]]$Mount = @(),
  [switch]$Build,
  [switch]$Ensure,
  [switch]$Down,
  [switch]$Repair,
  [switch]$Logs,
  [switch]$Follow,
  [switch]$Watch,
  [switch]$Force
)

# "Continue", no "Stop": en PowerShell 5.1 el stderr de un exe nativo (docker/gh) se
# vuelve error terminante con Stop, y consultar algo que NO existe es normal aca
# (contenedor ausente, imagen sin construir, Docker parado). En su lugar se mira
# $LASTEXITCODE despues de cada llamada, que es el contrato real.
$ErrorActionPreference = "Continue"

$RepoRoot = Split-Path -Parent $PSScriptRoot
$GhaDir = Join-Path $RepoRoot "docker\gha-local"
$ConfigPath = Join-Path $env:APPDATA "edgePass\config.json"
$VolumeSuffix = "gha-local-work"
$SocketMount = "/var/run/docker.sock:/var/run/docker.sock"

function Say($msg) { Write-Host $msg -ForegroundColor Cyan }
function Ok($msg) { Write-Host $msg -ForegroundColor Green }
function Warn($msg) { Write-Host $msg -ForegroundColor Yellow }
function Die($msg) { Write-Host $msg -ForegroundColor Red; exit 1 }

function Test-Docker {
  docker info --format '{{.ServerVersion}}' *> $null
  return $LASTEXITCODE -eq 0
}

function Get-Config {
  if (-not (Test-Path $ConfigPath)) {
    Die "No encuentro $ConfigPath. Corre edgePass una vez para que escriba su config."
  }
  return Get-Content $ConfigPath -Raw | ConvertFrom-Json
}

function Save-Config($cfg) {
  if (-not $Force) {
    $backup = "$ConfigPath.bak-gha-$(Get-Date -Format yyyyMMdd-HHmmss)"
    Copy-Item -LiteralPath $ConfigPath -Destination $backup -Force
    Say "Backup: $backup"
  }
  # El widget puede reescribir el archivo en cualquier momento (relee en cada poll):
  # escribimos atomico para no dejarlo a medias.
  #
  # Sin BOM, y no por estetica: `Set-Content -Encoding UTF8` de PS 5.1 escribe el BOM
  # UTF-8, y `serde_json` rechaza un BOM al principio -> el widget se queda sin config
  # ("invalid type: ..."), en silencio. `WriteAllText` con `UTF8Encoding($false)` no lo
  # pone. (Ya paso una vez con este script.)
  $tmp = "$ConfigPath.gha-tmp"
  $json = $cfg | ConvertTo-Json -Depth 12
  [System.IO.File]::WriteAllText($tmp, $json, (New-Object System.Text.UTF8Encoding($false)))
  Move-Item -LiteralPath $tmp -Destination $ConfigPath -Force
}

function Get-Runners($cfg) {
  $runners = @()
  if ($cfg.PSObject.Properties.Name -contains "ghaRunners") { $runners = @($cfg.ghaRunners) }
  if ($Id) { $runners = @($runners | Where-Object { $_.id -eq $Id }) }
  return $runners
}

function Test-ImagePresent([string]$img) {
  docker image inspect $img *> $null
  return $LASTEXITCODE -eq 0
}

function Get-ContainerFacts([string]$name) {
  if (-not (Test-Docker)) { return $null }
  $line = docker inspect $name --format '{{.State.Status}}|{{.RestartCount}}' 2>$null
  if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($line)) { return $null }
  $parts = $line.Trim().Split("|")
  return [pscustomobject]@{ State = $parts[0]; Restarts = [int]$parts[1] }
}

function Get-RegistrationToken([string]$repo) {
  Say "Acunando registration token para $repo (caduca en ~1 h)..."
  $token = (gh api -X POST "repos/$repo/actions/runners/registration-token" --jq ".token")
  if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($token)) {
    Die "gh no pudo acunar el token para $repo. Revisa 'gh auth status'."
  }
  return $token.Trim()
}

function Show-Log($name, [switch]$FollowNow) {
  if ($FollowNow) { docker logs -f --tail 40 $name } else { docker logs --tail 60 $name }
}

function New-Runner($runner, $token) {
  $name = if ($runner.name) { $runner.name } else { "gha-local-$($runner.id)" }
  $img = if ($runner.image) { $runner.image } else { $Image }
  $vol = if ($runner.workVolume) { $runner.workVolume } else { "$VolumeSuffix-$($runner.id)" }
  $mount = if ($runner.socketMount) { $runner.socketMount } else { $SocketMount }
  $labels = if ($runner.labels) { $runner.labels } else { $Labels }
  $repo = $runner.repo

  $mounts = @()
  if ($runner.PSObject.Properties.Name -contains "mounts") { $mounts = @($runner.mounts | Where-Object { $_ -and $_.ToString().Trim() }) }

  $dockerArgs = @(
    "run", "-d", "--name", $name, "--restart", "unless-stopped",
    "-e", "RUNNER_SCOPE=repo",
    "-e", "REPO_URL=https://github.com/$repo",
    "-e", "RUNNER_TOKEN=$token",
    "-e", "RUNNER_NAME=$name",
    "-e", "RUNNER_LABELS=$labels",
    "-e", "RUNNER_WORKDIR=/tmp/runner-work",
    "-v", "${vol}:/tmp/runner-work",
    "-v", $mount
  )
  # Mounts extra de la fila: la llave SSH del VPS, tipicamente (en OpenBao vive como
  # path, no como contenido).
  foreach ($m in $mounts) { $dockerArgs += @("-v", $m.ToString().Trim()) }
  $dockerArgs += $img
  $id = docker @dockerArgs
  if ($LASTEXITCODE -ne 0) { Die "docker run fallo para $name." }
  Ok "$name creado ($($id.Substring(0, 12)))"
  Warn "Ojo: el token va como -e en el argv de este script (lo ves en el log del shell)."
  Warn "El widget no: usa el env del proceso hijo (gha.rs run_args). Para credenciales de"
  Warn "verdad (CR_PAT, SSH) usa el campo secretEnv de la fila en config.json."
}

# ---------------------------------------------------------------- status
function Invoke-Status {
  $cfg = Get-Config
  $runners = Get-Runners $cfg
  $dockerUp = Test-Docker
  Write-Host ""
  if ($dockerUp) { Ok "Docker Desktop: responde ($(docker context show))" }
  else { Warn "Docker Desktop: NO responde" }
  if ($runners.Count -eq 0) {
    Warn "Sin runners declarados. Agrega uno:  -Add owner/repo"
    return
  }
  foreach ($r in $runners) {
    $name = if ($r.name) { $r.name } else { "gha-local-$($r.id)" }
    $img = if ($r.image) { $r.image } else { $Image }
    $imgOk = Test-ImagePresent $img
    $facts = Get-ContainerFacts $name
    $state = if ($facts) { $facts.State } else { "absent" }
    $restarts = if ($facts) { $facts.Restarts } else { 0 }
    $line = "$($r.id) :: $($r.repo) :: $img presente=$imgOk :: contenedor $state ($restarts reinicios)"
    if ($facts -and $facts.State -eq "running" -and $imgOk) { Ok $line } else { Warn $line }
  }
}

# ---------------------------------------------------------------- add / remove
function Invoke-Add([string]$repo) {
  if ($repo -notmatch "^[^/]+/[^/]+$") { Die "Repo invalido: '$repo'. Formato: owner/repo" }
  $cfg = Get-Config
  $id = if ($Id) { $Id } else { ($repo.Split("/")[1]).ToLower() }
  $existing = @()
  if ($cfg.PSObject.Properties.Name -contains "ghaRunners") { $existing = @($cfg.ghaRunners) }
  $existing = @($existing | Where-Object { $_.id -ne $id })
  $row = [pscustomobject]@{
    id         = $id
    repo       = $repo
    labels     = $Labels
    image      = $Image
    mounts     = @($Mount)
    secretEnv  = @()
  }
  $existing += $row
  if ($cfg.PSObject.Properties.Name -contains "ghaRunners") { $cfg.ghaRunners = $existing }
  else { $cfg | Add-Member -NotePropertyName ghaRunners -NotePropertyValue $existing }
  Save-Config $cfg
  Ok "Runner '$id' -> $repo agregado. El LED GW lo vigila en el proximo poll."
  Say "Siguiente: -Ensure (construye la imagen si falta y lo levanta)."
}

function Invoke-Remove([string]$id) {
  $cfg = Get-Config
  $keep = @()
  if ($cfg.PSObject.Properties.Name -contains "ghaRunners") {
    $keep = @($cfg.ghaRunners | Where-Object { $_.id -ne $id })
  }
  $cfg.ghaRunners = $keep
  Save-Config $cfg
  Ok "Runner '$id' fuera de la config."
  $name = "gha-local-$id"
  $facts = Get-ContainerFacts $name
  if ($facts) {
    docker rm -f $name *> $null
    Ok "Contenedor $name borrado."
  }
  Warn "El runner sigue registrado en GitHub: borralo en Settings -> Actions -> Runners."
}

# ---------------------------------------------------------------- build / ensure
function Invoke-Build {
  if (-not (Test-Path (Join-Path $GhaDir "Dockerfile"))) { Die "No encuentro el Dockerfile en $GhaDir" }
  Say "Construyendo $Image desde $GhaDir (la primera vez tarda: Go, Node, CLI de Docker)..."
  docker build -t $Image $GhaDir
  if ($LASTEXITCODE -ne 0) { Die "docker build fallo." }
  Ok "Imagen $Image lista."
}

function Invoke-Ensure {
  if (-not (Test-Docker)) {
    Warn "Docker no responde: arranca Docker Desktop y volve a correr esto (el widget lo hace solo)."
    $exe = "C:\Program Files\Docker\Docker\Docker Desktop.exe"
    if (Test-Path $exe) { Start-Process -FilePath $exe; Warn "Lo arranque; espera ~1 min y repeti." }
    return
  }
  $cfg = Get-Config
  $runners = Get-Runners $cfg
  if ($runners.Count -eq 0) { Die "Sin runners declarados. Usa -Add owner/repo." }
  foreach ($r in $runners) {
    $name = if ($r.name) { $r.name } else { "gha-local-$($r.id)" }
    $img = if ($r.image) { $r.image } else { $Image }
    if (-not (Test-ImagePresent $img)) { Invoke-Build }
    $facts = Get-ContainerFacts $name
    $healthy = $facts -and $facts.State -eq "running"
    if ($healthy -and -not $Repair) { Ok "$name ya estaba arriba."; continue }
    if ($facts) { docker rm -f $name *> $null; Say "$name fuera ($($facts.State)); recreando." }
    elseif ($Repair) { Say "$name no existia; creando." }
    $token = Get-RegistrationToken $r.repo
    New-Runner $r $token
  }
}

function Invoke-Down {
  $cfg = Get-Config
  $runners = Get-Runners $cfg
  if ($runners.Count -eq 0) { Warn "Sin runners declarados."; return }
  foreach ($r in $runners) {
    $name = if ($r.name) { $r.name } else { "gha-local-$($r.id)" }
    $facts = Get-ContainerFacts $name
    if (-not $facts) { Warn "$name no existia."; continue }
    docker stop $name *> $null
    Ok "$name parado (Docker Desktop queda quieto)."
  }
}

function Invoke-Logs {
  $cfg = Get-Config
  $runners = Get-Runners $cfg
  if ($runners.Count -eq 0) { Die "Sin runners declarados." }
  foreach ($r in $runners) {
    $name = if ($r.name) { $r.name } else { "gha-local-$($r.id)" }
    Say "== $name =="
    Show-Log $name -FollowNow:$Follow
  }
}

# ---------------------------------------------------------------- visor (Watch)
# El runner NO vuelca la salida de los steps a su stdout: `docker logs` tiene el ciclo de
# vida (que job corre, como termino) y la salida real vive en
# `_diag/Worker_*.log` adentro del contenedor. El visor sigue las dos, sin repetir nada,
# y termina cuando el job termina (el `-NoExit` del lanzador deja la ventana abierta
# para leer el resultado).
#
# ASCII puro a proposito: PowerShell 5.1 lee este archivo como cp1252 (no lleva BOM) y
# un solo caracter no-ASCII sale mojibake en la consola.
function Get-WorkerFile($name) {
  return (docker exec $name sh -c 'ls -t /home/runner/actions-runner/_diag/Worker_*.log 2>/dev/null | head -1' 2>$null | Out-String).Trim()
}

function Invoke-Watch {
  $cfg = Get-Config
  $runners = Get-Runners $cfg
  if ($runners.Count -eq 0) { Die "Sin runners declarados." }
  $r = $runners[0]
  $name = if ($r.name) { $r.name } else { "gha-local-$($r.id)" }

  Write-Host "== visor del runner local: $name ($($r.repo)) ==" -ForegroundColor Cyan
  Write-Host "Ctrl+C para cerrar. Sigo el ciclo de vida y la salida del job en curso." -ForegroundColor DarkGray

  $seenLife = 0
  $worker = ""
  $workerLines = -1
  $job = ""
  $done = ""

  while ($true) {
    $life = @(docker logs $name 2>$null)
    if ($life.Count -gt $seenLife) {
      $life[$seenLife..($life.Count - 1)] | ForEach-Object { Write-Host $_ -ForegroundColor DarkGray }
      $seenLife = $life.Count
    }

    # Linea por linea: un regex multilinea se come el primer "Running job:" y devuelve un
    # nombre vacio (paso, y asi se veia "job <<>>").
    foreach ($line in $life) {
      if ($line -match '^[^:]+:\s*Running job:\s*(.+)$') {
        $found = $Matches[1].Trim()
        if ($found -ne $job) {
          $job = $found
          $done = ""
          Write-Host ""
          Write-Host "--> job '$job' arrancando..." -ForegroundColor Yellow
        }
      }
      if ($job -and $line -match 'completed with result:\s*(\w+)') {
        if ($line -like "*Job $job completed*") { $done = $Matches[1] }
      }
    }

    $wf = Get-WorkerFile $name
    if ($wf -and $wf -ne $worker) {
      $worker = $wf
      Write-Host "--- $worker (ultimas 25 lineas) ---" -ForegroundColor DarkCyan
      $head = (docker exec $name sh -c "tail -n 25 '$worker'" 2>$null | Out-String)
      if ($head.Trim()) { Write-Host $head }
      $total = (docker exec $name sh -c "wc -l < '$worker'" 2>$null | Out-String).Trim()
      $workerLines = 0
      if ($total -match '^\d+$') { $workerLines = [int]$total }
    }
    if ($worker -and $workerLines -ge 0) {
      $skip = $workerLines + 1
      $chunk = (docker exec $name sh -c "tail -n +$skip '$worker'" 2>$null | Out-String)
      if ($chunk.Trim()) {
        Write-Host $chunk
        $workerLines += ($chunk -split "`n").Count
      }
    }

    if ($done) {
      $color = if ($done -eq 'Succeeded') { 'Green' } else { 'Red' }
      Write-Host ""
      Write-Host "== job '$job' $done ==" -ForegroundColor $color
      exit 0
    }

    Start-Sleep -Seconds 2
  }
}

# ---------------------------------------------------------------- dispatch
if ($Add) { Invoke-Add $Add; exit 0 }
if ($Remove) { Invoke-Remove $Remove; exit 0 }
if ($Build) { Invoke-Build; exit 0 }
if ($Ensure -or $Repair) { Invoke-Ensure; exit 0 }
if ($Down) { Invoke-Down; exit 0 }
if ($Logs) { Invoke-Logs; exit 0 }
if ($Watch) { Invoke-Watch; exit 0 }
Invoke-Status
