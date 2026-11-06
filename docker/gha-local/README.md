# gha-local — el runner de GitHub Actions **de esta máquina**

Los minutos hospedados de GitHub Actions se acaban. Un **runner self-hosted no consume
minutos**: corre en tu hardware. Por eso el pipeline se da vuelta:

```
  [ tu máquina ]                   [ GitHub ]                    [ tu máquina ]
  build pesado (Docker Desktop) ▶── GHCR ──▶ deploy.yml ──▶ este runner ──▶ compose up -d
       ▲                                                                          │
       └── el LED GW de edgePass vigila Docker Desktop y este contenedor ◀────────┘
```

Todo pasa acá: el build, la imagen y el deploy. La **máquina destino es esta** (Windows
con WSL2 + Docker Desktop), no un VPS: el runner tiene el socket de Docker del host
montado, así que lo que el workflow levanta queda corriendo donde vivís. GitHub solo
orquesta el job — y como corre en **este** runner (no en `ubuntu-latest`), lo que la fila
declare en `secretEnv` sale de **OpenBao** y no de GitHub Secrets.

## Quién hace qué

| Pieza | Qué hace |
|---|---|
| `docker/gha-local/Dockerfile` | La caja: Ubuntu **22.04** (por .NET/ICU, ver Trampas) + Go, Node 22, Python, CLI de Docker + buildx, `gh`, `make`, `gcc`. Nada más: no sabe de tokens ni de repos. Verifica con `Runner.Listener --version` que el runner arranque de verdad antes de dejar construir. |
| `docker/gha-local/entrypoint.sh` | `config.sh` (si hace falta) y `run.sh`. Si la credencial está muerta **falla y sale** — fingir que sigue vivo es lo que deja los runs en `queued` para siempre. Y si el estado quedó a medias (`.runner` sin `.credentials`), lo limpia y se registra de nuevo. |
| `scripts/gha-local.ps1` | El camino manual: registrar, construir, levantar/parar, ver el log (`-Watch` = el visor de logs). |
| El widget (LED `GW`, `src-tauri/src/gha.rs`) | El camino normal: vigila y **repara** (acuña el registration token con `gh api`, construye la imagen si falta, recrea el contenedor cuando está caído, parado o en loop de credencial). `Ctrl`/clic derecho = abrir DaemonCurier. Y **abre el visor solo** cuando arranca un job (`ghaWatch`). |

## Ver los logs (el visor)

El runner **no** vuelca la salida de los steps a su stdout: `docker logs` tiene solo el ciclo
de vida (qué job corre, cómo terminó) y la salida real queda adentro del contenedor. Por eso
el visor lee las dos fuentes:

| fuente | dónde | qué tiene |
|---|---|---|
| ciclo de vida | `docker logs gha-local-<id>` | `Running job: X` … `Job X completed with result: Succeeded/Failed` |
| salida del job | `_diag/Worker_*.log` dentro del contenedor | el detalle del step, incluido el `##[error]` de un fallo |

Cómo se abre:

- **Solo**: el widget mira cada 3 s si algún runner arrancó un job y abre la ventana
  (config `ghaWatch`, encendido por defecto). La ventana sigue el job, imprime el resultado
  al final y se queda abierta para que lo leas.
- **A mano**: `powershell -File scripts\gha-local.ps1 -Watch -Id cb2`.
- **Un agente / el LLM** (por el pipe, que es lo que hace recuperable un fallo que nadie miró):

  ```jsonc
  {"op":"gha"}                                              // estado + job en curso
  {"op":"gha-logs","id":"cb2","tail":200,"source":"both"}    // lifecycle + worker
  ```

  El `detail` del LED `GW` ya dice el job en curso (`job «Checkout + backend (Go)» hace 2m
  30s`): "simplemente saber que está corriendo" no necesita abrir nada.
- Desde GitHub, siempre: `gh run view <id> --log`.

## Uso

```powershell
# 1. Declarar el runner (escribe ghaRunners[] en el config del widget; hace backup)
powershell -File scripts\gha-local.ps1 -Add Xhuk/ContinuityBridge2.0

# 2. Construir la imagen (la primera vez tarda: Go + Node + CLI de Docker)
powershell -File scripts\gha-local.ps1 -Build

# 3. Levantarlo (acuña el token y hace docker run)
powershell -File scripts\gha-local.ps1 -Ensure

# Estado, log, parar
powershell -File scripts\gha-local.ps1 -Status
powershell -File scripts\gha-local.ps1 -Logs -Id cb2 -Follow
powershell -File scripts\gha-local.ps1 -Down
```

Con el widget abierto, ni hace falta el script: el LED `GW` en ámbar ya te está diciendo
que hay algo que arreglar, y el click lo arregla. El script es para cuando estás en la
terminal y no querés abrir la UI.

El workflow del proyecto tiene que apuntar al runner local:

```yaml
runs-on: [self-hosted, Linux, X64, local]
```

Copia [`deploy.yml`](deploy.yml) a `.github/workflows/deploy.yml` del proyecto como punto
de partida, y usa `scripts/build-and-deploy.ps1` para build + push + disparo.

## Deploy local (esta máquina)

El job corre en el runner y despliega **acá**, con tres modos:

| `mode` | Qué hace |
|---|---|
| `compose` (default) | `actions/checkout` + `docker compose up -d --remove-orphans` con el compose del repo. Usa la imagen ya construida: no compila nada. |
| `image` | `docker run -d --name <repo>` con la imagen nueva (lo más simple). |
| `command` | un `DEPLOY_COMMAND` tuyo (env del runner, ideal para el `docker service update` de Dokploy o cualquier caso raro). |

La imagen normalmente **ya está en el daemon** (se construyó acá), así que no hay `pull`
ni credenciales de por medio. Solo si el tag no está local hace falta un `GH_TOKEN` con
`read:packages` en el `secretEnv` (el push local, en cambio, ya funciona con
`gh auth token`).

Ojo con una trampa del modo `compose`: los **bind mounts relativos** (`./data:/data`) se
resuelven contra el contenedor del runner, no contra el host. Si tu compose los usa, pasá
el proyecto como volumen en `ghaRunners[].mounts` y usá `mode: command` con paths
absolutos.

## Credenciales: OpenBao, no GitHub Secrets

La fila del runner puede declarar `secretEnv` — la misma forma que usa la lista de
servidores MCP — y edgePass resuelve esos valores **desde OpenBao al arrancar el
contenedor**:

```jsonc
{
  "id": "cb2",
  "repo": "Xhuk/ContinuityBridge2.0",
  "secretEnv": [
    { "key": "CR_PAT",           "namespace": "", "path": "integrations/ghcr",       "field": "read_packages_token" },
    { "key": "SSH_PRIVATE_KEY",  "namespace": "", "path": "_projects/vps/ssh",        "field": "deploy_key" },
    { "key": "SSH_HOST",         "namespace": "", "path": "_projects/vps/ssh",        "field": "host" },
    { "key": "SSH_USER",         "namespace": "", "path": "_projects/vps/ssh",        "field": "user" }
  ]
}
```

Regla que no se negocia: **los valores no viajan en el argv**. El contenedor recibe
`-e KEY` y el valor va en el entorno del proceso `docker` (ver `gha::run_args`, con test
que lo fija). Así no aparecen en `ps`, ni en el log del shell, ni en el historial.

## El loop de la credencial vencida

Síntoma: el contenedor en `Restarting (1)` y el log con

```
Request to https://api.github.com/.../actions/runner-registration failed with 404 Not Found
```

Causa: los registration tokens caducan (~1 h) y el runner perdió su `.credentials`.
Consecuencia si nadie lo mira: los runs quedan **`queued` para siempre** y parece un bug
del código.

Qué hace el widget: el probe lo detecta (`credential_error`), el LED se pone ámbar con el
motivo, y el click acuña un token nuevo con `gh` y recrea el contenedor. Nada de
"reiniciar y ver si agarra".

Dos detalles que se aprendieron a golpes:

- **Config a medias** (`.runner` sin `.credentials`, lo que deja un cierre brusco): el
  entrypoint lo detecta, borra el estado a medias y se registra de nuevo. Antes el
  contenedor entraba en loop con `Cannot configure the runner because it is already
  configured` — el caso que más se parece a "está corriendo pero no toma trabajo".
- **Tras recrear, tarda ~1-2 min en volver**: GitHub conserva la sesión del runner viejo
  y el nuevo reintenta (`Runner connect error: Error: Conflict. Retrying until
  reconnected`). Eso es normal y se resuelve solo; GitHub lo muestra `offline` mientras
  tanto. Si querés forzarlo, `gha.repair` recrea y `-Logs -Follow` te deja mirando hasta
  el `Runner reconnected`.

## Los dos lados del ciclo (probado)

```powershell
# levantar
powershell -File scripts\gha-local.ps1 -Ensure      # token nuevo + docker run
# romper (simula el caso que deja runs en cola)
docker exec gha-local-<id> rm -f /home/runner/actions-runner/.credentials
docker restart gha-local-<id>                      # se auto-sana: limpia y re-registra
# ver
gh api repos/<owner>/<repo>/actions/runners        # status=online, labels=…,local
```

## Poner los repos en local (receta)

Son tres cosas, una vez por repo. La tercera es la que decide: **quién ejecuta un job lo
decide el `runs-on`**, y un runner solo toma el job si tiene **todos** los labels pedidos.

```powershell
# 1. Registrar el runner al repo (los runners son repo-scoped: una fila por repo)
powershell -File scripts\gha-local.ps1 -Add Xhuk/<repo>
powershell -File scripts\gha-local.ps1 -Ensure

# 2. La variable del repo, con el JSON de labels — POR LA API, con el body por stdin
$value = '["self-hosted","Linux","X64","local"]'
$payload = @{ name = 'RUNNER'; value = $value } | ConvertTo-Json -Compress
$payload | gh api -X POST "repos/Xhuk/<repo>/actions/variables" --input -

# 3. En cada workflow, una línea (con el fallback a ubuntu-latest)
#    runs-on: ${{ fromJSON(vars.RUNNER || '["ubuntu-latest"]') }}
```

Con eso, volver atrás es `gh variable delete RUNNER --repo Xhuk/<repo>` (o ponerle
`["ubuntu-latest"]`): los workflows siguen válidos y vuelven a hospedado.

## Sin GitHub del todo: `act`

El runner self-hosted **igual pasa por GitHub** (te saca los minutos, pero el run se crea y
se encola allá). El otro camino es `nektos/act`: lee el workflow del repo y lo corre en
contenedores de Docker acá, sin subir ni encolar nada.

```powershell
powershell -File scripts\act-local.ps1 -Repo C:\proyectos\<repo> -List
powershell -File scripts\act-local.ps1 -Repo C:\proyectos\<repo> `
  -Workflow .github/workflows/ci.yml -Job build -Event workflow_dispatch
```

El binario se baja solo la primera vez a `%LOCALAPPDATA%\edgePass\tools\act`. Ojo con dos
cosas: la primera corrida baja la imagen (`catthehacker/ubuntu:act-22.04`, ~1 GB), y
**`act` no es un Actions exacto** — hay acciones que asumen el entorno hospedado y los
secretos los tenés que dar vos (`-SecretsFile`, no los lee de OpenBao todavía).

## Trampas

- **`gh variable set --body '[…]'` desde PowerShell 5.1 guarda el valor SIN las comillas**
  (`[self-hosted,Linux,X64,local]`), que no es JSON: `fromJSON()` revienta y el run falla
  con *"This run likely failed because of a workflow file issue"* (el archivo está bien; el
  valor no). Se arregla poniendo la variable por la API con el JSON por **stdin**
  (`... | gh api -X POST repos/<owner>/<repo>/actions/variables --input -`), que además
  evita todo el infierno de comillas del shell. Vale para cualquier `--body`/`-f` con `"`.
- **`config.sh` se niega con uid 0**: `Must not run with sudo`. El contenedor corre como
  root a propósito (el socket de Docker que se monta es del root del host), así que el
  entrypoint exporta `RUNNER_ALLOW_RUNASROOT=1`, que es la salida que documenta el propio
  runner para contenedores.
- **Base Ubuntu 22.04, no 24.04**: este runner trae su .NET embebido y lo que necesita son
  las dependencias nativas. En 24.04 hay `libicu74` pero .NET 6 no la acepta, y el fallo
  es en runtime: `Couldn't find a valid ICU package`. Ojo: **`ldd` no lo detecta** (la
  lib está, es la versión), por eso el `Dockerfile` verifica con
  `Runner.Listener --version` y no deja construir una imagen que arranca y se cae.
- **El entrypoint respeta un comando explícito**: `docker run --rm gha-local bash` te da
  un shell en vez de arrancar el runner (para inspeccionar la caja sin una fila en la
  config).
- **El socket, en Git Bash**: `//var/run/docker.sock://var/run/docker.sock` (la doble
  barra evita que MSYS lo traduzca a `C:\Program Files\Git\var\...`). Desde el widget no
  hace falta: no pasa por bash.
- **La llave del VPS es un *path*, no contenido**: en OpenBao vive como
  `VPS_SSH_KEY_PATH` (`dokploy/global`). Un path es config de máquina, así que va en la
  fila como `mounts: ["C:\\...\\vps_key:/keys/vps:ro"]` y el workflow usa
  `SSH_KEY_FILE=/keys/vps`. Si algún día guardás el contenido en el vault, el mismo
  `deploy.yml` acepta `SSH_PRIVATE_KEY` y él la escribe a un archivo temporal.
- **Sobrevive al widget** (`--restart unless-stopped`): el runner no depende de que
  edgePass esté abierto; el widget solo lo vigila y lo repara.
- **Un repo por fila**: los runners se registran a nivel *repo* (`gh api
  repos/<owner>/<repo>/actions/runners/registration-token`). Para org hace falta admin de
  la org y `RUNNER_SCOPE=org` con la URL de la org.
- **Borrar la fila no desregistra**: `-Remove` limpia la config y mata el contenedor, pero
  el runner sigue en GitHub → Settings → Actions → Runners.
- **El widget viejo borra la fila**: un exe de edgePass anterior a `ghaRunners` ignora el
  campo, y al guardar Ajustes reescribe la config sin él. Si tocás Ajustes → *Guardar*
  con el exe viejo, volvé a correr `-Add`.
