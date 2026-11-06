<#
.SYNOPSIS
  Corre un workflow de GitHub Actions EN LOCAL, sin GitHub (nektos/act).

.DESCRIPTION
  Son dos caminos distintos y conviene tener claros los dos:

    runner self-hosted (imagen gha-local)  -> GitHub dispara y encola; tu maquina ejecuta
                                              (cero minutos hospedados, pero pasa por GitHub)
    act (este script)                      -> lee el workflow del repo y lo corre en
                                              contenedores de Docker aca; no sube nada ni
                                              aparece un run en el repo

  `act` sirve para: probar un workflow antes de pushear, reproducir un fallo sin ensuciar
  el repo, o correr un job entero cuando no queres que quede registro. NO es un Actions
  exacto: hay acciones que asumen el entorno hospedado, los `runs-on` de otro sistema no
  existen, y los secretos los tenes que dar vos (no los lee de OpenBao todavia).

  El binario se baja solo la primera vez (gh release download nektos/act) a
  %LOCALAPPDATA%\edgePass\tools\act\act.exe.

.EXAMPLE
  powershell -File scripts\act-local.ps1 -List
  powershell -File scripts\act-local.ps1 -Workflow .github/workflows/ci.yml -Job build
  powershell -File scripts\act-local.ps1 -Workflow .github/workflows/build.yml -Event push -DryRun
  powershell -File scripts\act-local.ps1 -Workflow .github/workflows/deploy.yml -Secret "CR_PAT=xxx"
#>
[CmdletBinding()]
param(
  # Carpeta del repo. Vacio = el directorio actual.
  [string]$Repo = "",
  # Workflow a correr (relativo al repo). Vacio = act elige (o usa -List).
  [string]$Workflow = "",
  # Job puntual (-j). Vacio = todos los del evento.
  [string]$Job = "",
  # Evento a simular: workflow_dispatch, push, pull_request...
  [string]$Event = "workflow_dispatch",
  # Imagen que reemplaza a ubuntu-latest (act trae el entorno Linux en Docker).
  [string]$Image = "catthehacker/ubuntu:act-22.04",
  # Variables de entorno para el job: -Env FOO=bar
  [string[]]$Env = @(),
  # Secretos: -Secret "KEY=valor" (ojo: quedan en el historial de tu shell)
  [string[]]$Secret = @(),
  # Archivo .secrets (KEY=valor por linea). Mejor que -Secret para valores de verdad.
  [string]$SecretsFile = "",
  # Variables del workflow (lo que lee vars.X): -Var BRANCH=main
  [string[]]$Var = @(),
  # Lista los jobs que act ve en los workflows del repo.
  [switch]$List,
  # Muestra que haria, sin ejecutar.
  [switch]$DryRun,
  # Fuerza el pull de la imagen (por defecto no se re-baja en cada corrida).
  [switch]$Pull,
  # Verbose de act. (No se puede llamar -Verbose: es un parametro comun de PowerShell.)
  [switch]$ActVerbose
)

# "Continue", no "Stop": en PS 5.1 el stderr de un exe nativo se vuelve error terminante.
$ErrorActionPreference = "Continue"

function Say($m) { Write-Host $m -ForegroundColor Cyan }
function Ok($m) { Write-Host $m -ForegroundColor Green }
function Warn($m) { Write-Host $m -ForegroundColor Yellow }
function Die($m) { Write-Host $m -ForegroundColor Red; exit 1 }

if ([string]::IsNullOrWhiteSpace($Repo)) { $Repo = (Get-Location).Path }
if (-not (Test-Path $Repo)) { Die "No existe $Repo" }
Push-Location $Repo
if (-not (Test-Path (Join-Path $Repo ".git"))) {
  # Un worktree tiene .git como archivo, no como carpeta: se comprueba con git.
  git rev-parse --is-inside-work-tree *> $null
  if ($LASTEXITCODE -ne 0) { Pop-Location; Die "$Repo no es un repo git." }
}

function Get-ActBinary {
  $dir = Join-Path $env:LOCALAPPDATA "edgePass\tools\act"
  $exe = Join-Path $dir "act.exe"
  if (Test-Path $exe) { return $exe }
  $cmd = Get-Command act -ErrorAction SilentlyContinue
  if ($cmd) { return $cmd.Source }
  Say "Bajando nektos/act (una sola vez, ~9 MB)..."
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  gh release download --repo nektos/act --pattern "act_Windows_x86_64.zip" --dir $dir --clobber 2>&1 | Out-Null
  if ($LASTEXITCODE -ne 0) { Die "no pude bajar act (revisa 'gh auth status')." }
  Expand-Archive -Path (Join-Path $dir "act_Windows_x86_64.zip") -DestinationPath $dir -Force
  Remove-Item (Join-Path $dir "act_Windows_x86_64.zip") -Force -ErrorAction SilentlyContinue
  if (-not (Test-Path $exe)) { Die "act no quedo en $exe" }
  return $exe
}

$act = Get-ActBinary

if ($List) {
  Say "== jobs que act ve en $Repo =="
  & $act -l 2>&1 | ForEach-Object { "$_" }
  Pop-Location
  exit $LASTEXITCODE
}

$args = @($Event)
if ($Workflow) { $args += @("-W", $Workflow) }
if ($Job) { $args += @("-j", $Job) }
# act corre en Linux dentro de Docker: el ubuntu-latest de los workflows se mapea a la
# imagen que elijas (evita que se baje una distinta en cada job).
$args += @("-P", "ubuntu-latest=$Image")
foreach ($e in $Env) { $args += @("--env", $e) }
foreach ($s in $Secret) { $args += @("--secret", $s) }
if ($SecretsFile) { $args += @("--secret-file", $SecretsFile) }
foreach ($v in $Var) { $args += @("--var", $v) }
if ($DryRun) { $args += "-n" }
if (-not $Pull) { $args += "--pull=false" }
if ($ActVerbose) { $args += "-v" }

Say "=========================================="
Say " repo     : $Repo"
Say " workflow : $(if ($Workflow) { $Workflow } else { '(el que elija act)' })"
Say " job      : $(if ($Job) { $Job } else { '(todos los del evento)' })"
Say " evento   : $Event  (local, sin GitHub)"
Say " imagen   : $Image"
if ($SecretsFile) { Say " secretos : $SecretsFile" }
if ($Secret.Count -gt 0) { Warn " (los -Secret quedan en el historial de tu shell; usa -SecretsFile)" }
Say "=========================================="
docker image inspect $Image *> $null
if ($LASTEXITCODE -ne 0) { Warn "La primera corrida baja la imagen ($Image): puede tardar unos minutos." }

# act escribe su log en stderr y PowerShell 5.1 lo pinta como error: se pasa por el
# stream de salida para que se lea como lo que es (y $LASTEXITCODE sigue siendo el suyo).
& $act @args 2>&1 | ForEach-Object { "$_" }
$code = $LASTEXITCODE
Write-Host ""
if ($code -eq 0) { Ok "== ok (local) ==" } else { Warn "== act salio con codigo $code ==" }
Pop-Location
exit $code
