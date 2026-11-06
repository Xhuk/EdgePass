# EdgePass - Funcionalidades para Skills

EdgePass es un widget de escritorio que gestiona secretos desde tu vault de OpenBao local. Las skills pueden interactuar con EdgePass para gestionar secretos de forma automatizada.

## Estado de la Ventana

EdgePass tiene dos estados principales:

- **Colapsado (Dock)**: Pequeña pastilla en el borde de la pantalla con el logo y estado del vault
- **Expandido**: Ventana completa con búsqueda, gestión y configuración

## Interfaz de Pipe (JSON)

Las skills pueden enviar comandos a EdgePass mediante pipe JSON en stdout:

```json
{"op":"ingest-form","namespace":"buenaidee","path":"projects/cb2/infisical","keys":["DATABASE_URL","DB_USER","DB_PASSWORD","API_TOKEN"]}
```

### Operaciones Disponibles

| Operación | Descripción | Parámetros |
|-----------|-------------|------------|
| `ping` | Comprueba que edgePass está vivo | — |
| `focus` | Trae la pastilla al frente (no expande) | — |
| `status` | Estado del vault + hold | — |
| `search` | Busca en el índice local: **solo nombres** (namespace, path, key), sin abrir el vault | `query?`, `limit?` (default 50, máx 200) |
| `meter` | **Abre el tab Batch con los datos del secreto**: resuelve un path (o una `query` contra el índice) y el widget lee sus campos y los muestra para copiar (ojo por fila · ⧉ copiar todo). El llamante **nunca ve valores** | `query?` **o** `path` (+ `namespace?`, `title?`) |
| `unseal` / `seal` | Abrir / cerrar el vault ahora | — |
| `hold` | No auto-sellar durante N segundos | `secs?` (default 3600, máx 86400) |
| `run` | Ejecuta un comando con los secretos **en el entorno del hijo** (scripts locales; sin popup) | `entries[]`, `cmd[]`, `cwd?`, `keys?`, `timeoutSecs?` (default 300, máx 3600) |
| `request` | GW: pide un `run` con popup Allow once / 2h / Deny. Respuesta inmediata `pending`/`running`. **Nunca valores.** | `action=run`, `actor?`, `namespace`, `path` (o `entries[]`), `cmd[]`, `keys?`, `reason?`, `cwd?`, `detach?` |
| `request-status` | Snapshot de un request (o lista si no hay `id`). Sin valores / stdout | `id?` |
| `request-wait` | Espera hasta `timeoutSecs` (máx 60) a que salga de pending/running | `id`, `timeoutSecs?` |
| `request-resolve` | Solo UI: `deny` / `allow-once` / `allow-2h` | `id`, `decision` |
| `ingest-form` | Abre formulario batch para ingresar secretos | `namespace`, `path`, `keys[]`, `title?` |
| `sync_index` | Reindexa el índice local ahora (equivale al `↻` de la UI) | — |
| `toggles` | Los LEDs del header **como datos**: leer la lista + el vocabulario (`catalog`), o reescribirla (validada: ids del catálogo, máx 5) | — · `set[]` |
| `toggle` | Dispara un LED (lo mismo que un click del header): la acción la resuelve Rust desde `config.json`, tú solo nombras el LED | `id`, `dir?` (`flip`\|`on`\|`off`\|`alt`) |

> `export-env` **ya no existe**: escribía los secretos en claro en la ruta que pidiera el
> cliente. Usa `run`, que se los pasa al proceso hijo y nunca toca el disco.

### Buscar antes de pedir (`search`)

La mitad que faltaba del flujo: `ingest-form` crea, `search` comprueba. Un agente puede ver
si el secreto ya existe — y con qué nombres — antes de molestar al humano, y sin desbloquear
nada (el índice guarda nombres, nunca valores):

```json
{"op":"search","query":"infisi cb2","limit":20}
```

```json
{"ok":true,"count":2,"hits":[
  {"namespace":"buenaidee","path":"projects/cb2/infisical","key":"DATABASE_URL","useCount":7,"lastUsedAt":"2026-09-14T18:02:11+00:00"},
  {"namespace":"buenaidee","path":"projects/cb2/infisical","key":"API_TOKEN","useCount":0,"lastUsedAt":null}
]}
```

`query` vacío devuelve lo más usado primero. Los términos se combinan con **AND** (todos deben
aparecer) y hay alias cortos: `infisi`→`infisical`, `bao`→`openbao`/`vault`, `cf`→`cloudflare`,
`sb`→`supabase`, `pg`/`db`→`postgres`, `dk`→`dokploy`, `vg`→`vetgroom`.

> El índice es local y puede estar viejo: si `search` no ve algo recién creado por fuera,
> manda `↻` en la UI o llama a `sync_index` antes de concluir que falta. `sync_index`
> responde `{ ok, count }` (o `{ ok: false, error }`), y sí abre el vault si hace falta:
> cuenta como actividad del widget.

> **KV v1 y v2**: el índice detecta por namespace si el mount `kv` es KV v1 o v2
> (`sys/mounts`) y usa las rutas de cada versión, así que los namespaces con el mount en v1
> (acá: `vetgroom`, `vetops`, `cloudflare`) también entran. Copiar, editar y borrar usan el
> mismo camino, así que en un mount v1 también funcionan. El log anota una línea por
> namespace: `kv: namespace <ns> tiene mount 'kv' en KV v1|v2`.

### Ver los datos de un secreto en el tab Batch (`meter`)

`search` contesta *qué existe*; `meter` trae los **valores** a la pantalla — es lo que un humano
pide cuando dice **«usa edge pass»** o «dame los datos de la cuenta de betty». Y lo hace sin que
el valor salga del widget: el op solo nombra un path, **el widget lo lee con su token** y rellena
el tab Batch. Quien llamó (un agente) no recibe ningún valor, así que no puede pegarlo en un chat,
un log ni un commit.

```json
{"op":"meter","query":"betty"}
```

```json
{"ok":true,"id":1789…,"namespace":"dokploy","path":"projects/lotly/demo-users"}
```

Cómo se direcciona:

| Forma | Qué abre |
|---|---|
| `{"query":"betty"}` | El path al que apuntan los resultados, **si todos caen en uno solo** |
| `{"namespace":"dokploy","path":"projects/lotly/demo-users"}` | Ese path, sin consultar el índice |
| `title?` | Título del panel (default «Batch · copiar») |

Reglas y errores, tal como salen:

- **Query → un solo path.** `betty` que cae en los 6 campos `BETTY_*` de un path abre ese path.
- **Ambigua**: si caen en varios paths responde `{ ok:false, error:"ambiguo…",
  candidates:[{namespace,path,key}…] }` — sin adivinar; repite con `path`.
- **Sin coincidencias**: `{ ok:false, error:"sin coincidencias en el índice local (puede estar
  viejo: prueba sync_index)" }`.
- **Path sin campos / sin token / vault sellado sin auto-unseal**: el error se ve en el widget
  (es él quien lee), no en la respuesta del pipe.
- Si el panel ya está abierto, `meter` lo **reemplaza** por el path nuevo (una sola ranura
  pendiente, igual que `ingest-form`).

Qué hace el humano en ese panel, ya dentro del widget:

- Cada fila: **Variable** + **Valor**. Los valores se abren **visibles** y se **auto-ocultan a los
  8s** (`METER_AUTO_MASK_MS`); el botón `ver`/`ocultar` de cada fila y `ver todo`/`ocultar todo`
  los vuelven a mostrar.
- **`⧉` por fila** copia ese valor al portapapeles (sin mostrarlo en ningún chat).
- **`⧉ copiar todo`** copia el bloque entero como líneas `KEY=valor` (formato `.env`), que es
  justo lo que un chat no puede llevarse.
- Sigue siendo el mismo editor: `Enter` o ✓ guarda en OpenBao (crea versión nueva), y «Descartar»
  cierra sin escribir. Nada se auto-guarda.

```powershell
# El envoltorio para agentes y humanos
powershell -File scripts\edgepass-meter.ps1 -Query "betty"
powershell -File scripts\edgepass-meter.ps1 -Namespace dokploy -Path "projects/lotly/demo-users"
powershell -File scripts\edgepass-meter.ps1 -Query "infisi cb2" -Title "CB2 Infisical"
```

> `meter` es **solo pipe local**: abre una ventana, así que no forma parte del contrato remoto
> (`request` / HTTP con Grok) — un actor fuera de esta máquina sigue recibiendo únicamente
> nombres. El pipe se crea con DACL de un solo ACE (tu SID), y nada de esto escribe valores a
> disco.

### Ejemplo de Uso

```powershell
# PowerShell — formulario batch (el humano pega los valores, el agente no los ve)
powershell -File scripts\edgepass-ingest-form.ps1 `
  -Namespace buenaidee `
  -Path "projects/cb2/infisical" `
  -Keys DATABASE_URL,DB_USER,DB_PASSWORD,API_TOKEN `
  -Title "CB2 Infisical"
```

Pipe JSON directo (una línea JSON por petición, al named pipe `\\.\pipe\edgepass`):

```powershell
$json = '{"op":"ingest-form","namespace":"buenaidee","path":"projects/cb2/infisical","keys":["DATABASE_URL","DB_USER"]}'
$pipe = New-Object System.IO.Pipes.NamedPipeClientStream(".", "edgepass", [IO.Pipes.PipeDirection]::InOut)
$pipe.Connect(3000)
$bytes = [Text.Encoding]::UTF8.GetBytes($json + "`n")
$pipe.Write($bytes, 0, $bytes.Length); $pipe.Flush()
$reader = New-Object IO.StreamReader($pipe, [Text.Encoding]::UTF8, $false, 1024, $true)
$reader.ReadLine()
$pipe.Dispose()
```

### Ejecutar un comando con secretos (sin escribir nada a disco)

```json
{"op":"run","entries":[{"namespace":"buenaidee","path":"projects/cb2/infisical"}],"cmd":["npm","run","migrate"]}
```

Responde `{ ok, code, env[], stdout, stderr }` — solo los **nombres** de las variables, nunca
los valores. `keys` limita qué se inyecta.

`timeoutSecs` corta el comando (default 300, máx 3600). Al vencer, mata el **árbol** de
procesos y responde:

```json
{"ok":false,"timeout":true,"timeoutSecs":300,"error":"timeout tras 300s: proceso terminado","code":null,"env":["DATABASE_URL"],"stdout":"…","stderr":"…"}
```

Motivo del timeout: un `run` colgado dejaba al llamante esperando para siempre. El kill es
del árbol porque en Windows `npm run x` es `npm.cmd` → `node`: matar solo el primero deja vivo
al segundo (y con los pipes abiertos).

`cmd[0]` se resuelve por `PATH` + `PATHEXT` antes de lanzar, así que `["npm", "run", "migrate"]`
funciona tal cual: Windows solo añade `.exe` por su cuenta, y `npm` es `npm.cmd`. Si el binario
no aparece, el error lleva el nombre que escribiste, no una ruta resuelta rara.

## Estados del Vault (Seal Status)

El estado del vault se muestra en el chip de estado y afecta el comportamiento:

| Estado | Código | Descripción |
|--------|--------|-------------|
| `open` | 0 | Vault abierto, secretos accesibles |
| `sealed` | 1 | Vault sellado, secretos protegidos |
| `warn` | 2 | Vault inicializado pero con advertencias |
| `down` | 3 | OpenBao offline o no reachable |

## Comportamiento Automático

### Auto-Unseal
- Al iniciar EdgePass, si el vault está sellado, **se desbloquea automáticamente** leyendo las keys del init local
- Las keys nunca se muestran en la UI
- Configuración: `autoUnseal` (default: `true`)

### Auto-Seal (opt-in, por defecto desactivado)
- **Por defecto NO se sella nunca.** Sellar es un efecto sobre *todo* el vault: corta a los
  demás consumidores (otros proyectos, scripts, servicios). Que este widget esté quieto no
  es una señal válida para cerrarles el vault.
- Si se configura `autoSealIdleSecs` > 0, se vuelve a sellar tras ese idle
- Valor `0` (default) = nunca sellar automáticamente
- Botón "Cerrar" en la UI para sellar manualmente, cuando tú lo decides
- Las configs antiguas que traían `300` se migran a `0` una sola vez al arrancar

### Auto-Sync
- Al expandir la ventana, EdgePass **sincroniza el índice** de secretos automáticamente
- Configuración: `autoSync` (default: `true`)
- Botón `↻` para reindexar manualmente

### Auto-Load Token
- Carga el token de autenticación desde el archivo init JSON automáticamente
- Configuración: `autoLoadToken` (default: `true`)

## Gestión de Secretos

### Búsqueda
- Input de búsqueda en la parte superior
- Búsqueda indexada en namespaces configurados
- Resultado muestra: clave, namespace, path, uso reciente

### Copiar Secreto
- Click en un resultado copia el valor al clipboard
- Muestra toast con la label del secreto copiado
- Incrementa contador de uso (`useCount`)

### Nuevo Secreto (Panel "Nuevo")
- Formulario para agregar un secreto individual
- Campos: Namespace, Path, Variable, Valor
- Opción: "Reemplazar path completo" (overwrite existing)

### Lote / Batch (Panel "Lote")
- Abierto automáticamente vía `ingest-form` pipe, o relleno desde el vault vía `meter`
- Tabla con variables y valores para completar (el humano pega los valores) o ya leídos
- Cada fila: `ver`/`ocultar` el valor y `⧉` para copiarlo · el pie: `⧉ copiar todo` (líneas
  `KEY=valor`) y `ver todo`/`ocultar todo`
- Los valores de `meter` se auto-ocultan a los 8s (`METER_AUTO_MASK_MS`) y el ojo los vuelve a mostrar
- Opción: "Reemplazar path completo"
- Guardar lote en vault con un click

## Configuración (Panel "Config")

| Parámetro | Descripción | Default |
|-----------|-------------|---------|
| `baoAddr` | URL del servicio OpenBao | `http://127.0.0.1:8200` |
| `mount` | Mount point del vault | `kv` |
| `namespaces` | Namespaces a indexar (comma-separated). `.` = root **+ todos los hijos** vía `sys/namespaces` | `.` |
| `initPath` | Ruta al JSON de init (token+keys) | `` |
| `autoSealIdleSecs` | Segundos de idle antes de sellar (0 = nunca) | `0` |
| `token` | Token manual (si no se usa initPath) | `` |

### Botones de Acción (LEDs del header)

Los LEDs del header (`KV`, `GW`, `GOV`…) **son datos**: cada uno es una fila de
`config.json` → `toggles[]` con `probe` (qué mira) y `on` / `off` / `alt` (qué
hace un click · `alt` = Ctrl o clic derecho). Se editan en **Ajustes → Toggles del
header** (máx **6**, etiqueta de 1-4 letras, `＋ Agregar un toggle`) o por el pipe.

Vocabulario (lo publica el propio widget en `catalog`, nunca lo adivines):

| `probe` | ON cuando |
|---------|-----------|
| `vault.seal` | vault alcanzable y abierto (ámbar: no responde o sin inicializar) |
| `qring.process` | `qringd` vivo (ámbar: falta el pack) — el LED se llama **`QR`** |
| `qring.governance` | los rings pueden correr (y gastar) — **y es el LED que pulsa cuando hay un Allow esperando** |
| `docker.desktop` | el motor de Docker Desktop responde (ámbar: instalado pero parado) |
| `gha.runner` | Docker arriba **y** el runner local de GitHub Actions sano — el LED **`GW`** |
| `skills.sync` | la biblioteca de skills está en fase: 0 enlaces por arreglar (ámbar: deriva, o falta el sincronizador) |
| `skills.watch` | el vigilante del hub está corriendo (auto-sincronización) |

| `action` | Efecto |
|----------|--------|
| `vault.unseal` / `vault.seal` | abrir (keys del init) / sellar (token; corta a todos los clientes) |
| `qring.on` / `qring.off` / `qring.restart` | arrancar / parar / reiniciar el gateway. **Parar apaga también la política** (un gateway parado no gobierna nada); arrancar no la enciende |
| `qring.govern.on` / `qring.govern.off` | permitir / bloquear que los rings corran. Encender con el gateway parado **lo arranca primero** (con el vault); apagar vale con el gateway arriba, que es el caso que `GOV` cubre |
| `docker.up` | arrancar Docker Desktop y esperar el motor (hasta 120 s) |
| `gha.up` | asegurar el runner local: Docker, imagen (la construye si falta) y contenedor — lo **recrea con token nuevo** si está caído, parado o en loop de credencial |
| `gha.down` | parar el contenedor del runner (Docker Desktop queda quieto) |
| `gha.repair` | recrear el runner aunque parezca sano (si GitHub lo ve `offline`) |
| `daemoncurier.open` | abrir/enfocar DaemonCurier (el HUD de WSL2/sistema) |
| `skills.sync` | re-enlazar los 50 agentes desde el hub de skills (no copia archivos) |
| `skills.watch.on` / `skills.watch.off` | encender / apagar la auto-sincronización del hub |

```jsonc
{"op":"toggles"}                                     // LEDs + catálogo
{"op":"toggles","set":[{"id":"kv","label":"KV","probe":"vault.seal",
                        "on":"vault.unseal","off":"vault.seal"}]}
{"op":"toggle","id":"kv","dir":"on"}                 // flip | on | off | alt
```

`{"op":"status"}` también devuelve `toggles`. Un id desconocido se rechaza **con
el catálogo en la respuesta**. Helper:
`powershell -File scripts\edgepass-toggles.ps1 -List` (`-Catalog`, `-Fire kv -Dir off`,
`-Set <json>`). Contrato completo: `docs/TOGGLE-CONTRACT.md`.

- **Instalación nueva**: solo `KV` (el único backend garantizado es el vault).
- **Actualización**: conserva los tres de siempre (`KV`, `GW`, `GOV`) y el `GW` viejo
  —que era el proceso del Q-ring— pasa a `QR` (v3→v4), porque `GW` ahora vigila el runner
  local de GHA. Queda `KV · QR · GOV · GW`.
- **Runner local de GHA (LED `GW`)**: imagen `gha-local:latest`
  (`docker/gha-local/`), el contenedor se registra a un repo con labels
  `self-hosted,Linux,X64,local` y el workflow lo usa con `runs-on: [self-hosted, Linux,
  X64, local]`. Las filas viven en `config.json` → `ghaRunners[]` (id, repo, labels,
  image, `secretEnv` con credenciales de OpenBao), y el widget le acuña el registration
  token con `gh api` al arrancar. Camino manual: `scripts\gha-local.ps1`
  (`-Add`, `-Build`, `-Ensure`, `-Status`, `-Logs`, `-Down`, `-Watch`). Runbook:
  `docker/gha-local/README.md`; el pipeline local (build → GHCR → deploy fino) está en
  `scripts\build-and-deploy.ps1` + `docker/gha-local/deploy.yml`.
- **Ver qué corre y por qué falló**: el runner no vuelca la salida de los steps a su
  stdout, así que hay dos fuentes: `docker logs gha-local-<id>` (ciclo de vida: qué job
  corre, cómo terminó) y `_diag/Worker_*.log` adentro del contenedor (el detalle del
  step). El `detail` del LED `GW` dice el job en curso, y por el pipe:
  `{"op":"gha"}` (estado + job) y `{"op":"gha-logs","id":"cb2","tail":200,"source":"both"}`.
  El widget abre el visor solo cuando arranca un job (`ghaWatch`), o
  `scripts\gha-local.ps1 -Watch -Id cb2` a mano.
- **Que un repo compile en local y no en GitHub**: el `runs-on` decide. Registrar el runner
  al repo (`gha-local.ps1 -Add`/`-Ensure`) + la variable del repo `RUNNER` con
  `["self-hosted","Linux","X64","local"]` (por la API con el JSON por **stdin**: con
  `gh variable set --body` PowerShell 5.1 guarda el valor sin las comillas y `fromJSON()`
  revienta) + `runs-on: ${{ fromJSON(vars.RUNNER || '["ubuntu-latest"]') }}`. Volver atrás
  es borrar la variable. Y para **cero GitHub** en el medio (sin encolar nada):
  `scripts\act-local.ps1` (nektos/act en Docker).
- **Nuevo tipo de LED**: una fila en `src-tauri/src/toggles.rs` (`PROBES` /
  `ACTIONS`) + su rama en `probe_state` / `run_action`; el resto sale solo.
- **Skills (`SYNC` / `AUTO`)**: opt-in, no vienen en los defaults porque apuntan a
  `C:\proyectos\skills-sync\skills_sync.py` (herramienta de la máquina, no del producto;
  se puede cambiar con `EDGEPASS_SKILLS_SYNC`). El puente es `src-tauri/src/skills.rs`:
  el LED lee una caché de 15 s (un `status` completo tarda ~1 s y no puede bloquear la UI)
  y las acciones corren el script en `spawn_blocking`. El vigilante es un **proceso hijo**
  de edgePass: muere con él (para que viva desde el arranque, `skills_sync.py
  install-autostart`).

## Eventos y Listeners

EdgePass emite eventos que las skills pueden escuchar:

### `ingest-form`
- Emitido cuando un formulario batch está pendiente de completar
- Payload: `{ id, namespace, path, keys[], title? }`
- La ventana se expande automáticamente para mostrar el formulario

### `grant-request`
- Emitido cuando Grok/GW pide un `run` y no hay grant 2h para ese path
- Payload: `{ id, actor, action, namespace, path, keys[], cmd[], reason, expiresAt }` (nombres; nunca valores)
- Overlay Allow once / 2 horas / Deny; Esc = deny

### `get_status`
- Retorna el estado actual del vault y configuración
- Payload completo: `StatusDto` (ver tipo en código)

## Integración con LLMs/Agents

### GW (Grok) — search + run con Allow

Grok **nunca recibe valores**. Lista nombres; para usar un secreto pide un `run` en este nodo. El humano ve Allow once / 2 horas / Deny. El env vive solo en el proceso hijo.

HTTP (Bearer `EDGEPASS_OPERATOR_TOKEN`, Tailscale `:7437`):

1. `GET /v1/secrets/search?query=infisi+cb2` — nombres. Sin popup.
2. `POST /v1/secrets/request` `{actor:"grok", action:"run", namespace, path, keys?, cmd:[], reason}` → `202` + `requestId` (`pending` o `running` si hay grant 2h del mismo path).
3. Poll `GET /v1/secrets/request?id=` cada 2–5s (máx ~5 min) hasta `granted|denied|timeout`.
4. Si `granted`: lee `ok`, `env[]` (nombres), `code`. **No** scrapes stdout (ni viene en este canal).
5. Si `denied` / `timeout` / `sealed`: para. No inventes valores. No pidas que te los peguen.

Pipe local (mismo contrato, ops `search` / `request` / `request-status`). El `run` directo del pipe **no** abre popup (scripts).

> `meter` **no** entra en este contrato: abre la ventana del widget y por eso vive solo en el
> pipe local, para agentes que corren en tu misma máquina (ver *Ver los datos de un secreto en
> el Meter*).

Grant 2h: mismo `actor + namespace + path`. Otro path = otro Allow. Se pierde al matar el widget.

### Flujo Recomendado para Ingresar Secretos

1. **LLM identifica** que necesita secretos para una tarea
2. **LLM pregunta primero** con `op: "search"` — si ya existen, no molesta a nadie
3. **LLM envía** pipe JSON con `op: "ingest-form"` para lo que falte
4. **EdgePass** abre formulario batch automáticamente
5. **Usuario** completa los valores en la UI (la LLM nunca los ve)
6. **Usuario** hace click en "Guardar lote en vault"
7. **EdgePass** guarda todos los secretos y cierra el formulario

### Ejemplo Completo

```powershell
# 1. EdgePass corriendo en background
.\edgepass.exe

# 2. LLM comprueba si ya existen (nombres solamente, sin abrir el vault)
$json = '{"op":"search","query":"infisi cb2","limit":20}'
# → {"ok":true,"count":0,"hits":[]}

# 3. LLM envía comando para ingresar lo que falta
$json = @{
    op = "ingest-form"
    namespace = "buenaidee"
    path = "projects/cb2/infisical"
    keys = @("DATABASE_URL", "DB_USER", "DB_PASSWORD", "API_TOKEN")
    title = "CB2 Infisical"
} | ConvertTo-Json

$json | .\edgepass.exe

# 4. EdgePass abre el formulario, el usuario completa los valores y guarda
```

## Variables de Entorno

Overrides de una ejecución, **sin escribir `config.json`** (nunca se persisten: guardar en
Config escribe lo que está en pantalla, no el valor forzado). El proceso los anota en el log
al detectarlos.

| Variable | Descripción |
|----------|-------------|
| `EDGEPASS_AUTO_UNSEAL` | Sobrescribe `autoUnseal` (`1`/`true`/`yes`/`on` · `0`/`false`/`no`/`off`) |
| `EDGEPASS_AUTO_SEAL_IDLE` | Sobrescribe `autoSealIdleSecs` (segundos; `0` = nunca sellar) |
| `EDGEPASS_NAMESPACES` | Sobrescribe `namespaces` (separados por comas; `.` = root + todos los hijos) |

```powershell
$env:EDGEPASS_NAMESPACES = "., buenaidee"
$env:EDGEPASS_AUTO_SEAL_IDLE = "0"
.\edgepass.exe
```

Valores inválidos (un texto donde va un número, un booleano que no es booleano) se ignoran
con un warning en el log: un typo no puede dejar el widget sin config.

## Seguridad

- Las keys del init JSON **nunca se muestran** en la UI
- El token se carga automáticamente pero no se expone, y vive en el keyring del OS
- Sellar el vault es **opt-in**: por defecto edgePass no se lo cierra a nadie (ver Auto-Seal)
- `baoAddr` tiene que ser loopback; cualquier otro host se rechaza **antes** de enviar el token
- Ninguna operación escribe secretos a disco: el valor sale solo por el portapapeles (clics
  tuyos en el widget) o por el entorno de un proceso hijo (`run`)
- Ningún op **devuelve** un valor al llamante: `meter` solo nombra un path y el widget lee los
  campos para mostrarlos en pantalla, así que un agente no acaba con el secreto en la mano. El
  contrato remoto (`request` / HTTP) sigue siendo de nombres, y nunca se registra un valor en el
  log (el log anota path y cuántos campos, no cuáles)
- El named pipe `\\.\pipe\edgepass` se crea con una **DACL de un solo ACE**: acceso total para
  el SID del usuario que corre edgePass y nada más (ni `Everyone`, ni `Authenticated Users`,
  ni ACEs heredadas). Cualquier proceso de *tu* usuario puede hablar por el pipe — eso es la
  frontera de confianza, y es la misma de siempre; lo que ya no pasa es que lo pueda abrir
  otra cuenta de la máquina. Si la DACL no se puede construir, el pipe se crea sin restringir
  y queda anotado en el log (nunca se finge que está restringido)
- Solo secretos de namespaces configurados son indexados (`.` descubre e indexa todos)
- El índice guarda **nombres** (namespace, path, variable), nunca valores — y `search` nunca
  responde con más que esos nombres
- `run` mata el **árbol** de procesos al vencer su `timeoutSecs` (default 300s), para que un
  comando colgado no deje colgado al llamante
- La UI tiene un panel **Logs** con el binario en ejecución, la ruta del log, el estado del
  vault y las últimas líneas de log
- Cerrar el widget (Logs → *Salir de edgePass*, dos pasos) **no** sella el vault, y viceversa:
  sellar no cierra la app
- Abrir el `.exe` dos veces no crea una segunda instancia: le pasa el foco a la que corre

## Atajos de Interfaz

| Atajo | Acción |
|-------|--------|
| Click en logo | Expandir/colapsar ventana |
| Click en resultado | Copiar valor al clipboard |
| Click en "Abrir"/"Cerrar" | Forzar desbloquear/sellar vault |
| Click en "Svc" | Iniciar servicio OpenBao |
| Click en `↻` | Reindexar secretos |
| Arrastrar ventana cerca de borde | Anclar a ese borde (dock) |
| Arrastrar lejos del borde | Flotar como pastilla |
| `↑` `↓` + `Enter` | Moverse por los resultados y copiar el seleccionado |
| `Esc` | Cancelar el aviso / desarmar el "Salir" / colapsar el panel |
| Logs → *Salir de edgePass* | Cerrar la app (dos pasos; el vault queda como esté) |

## Archivos de Script

- `scripts\edgepass-ingest-form.ps1` - PowerShell wrapper para ingest-form
- `scripts\edgepass-meter.ps1` - PowerShell wrapper para meter (abre el tab Batch con los datos de un secreto)
- `scripts\launch-edgepass.ps1` - Lanzar EdgePass
- `scripts\install-autostart.ps1` - Instalar autostart tras servicio OpenBao

## Tipo de Datos: StatusDto

```typescript
type StatusDto = {
  baoAddr: string;
  mount: string;
  namespaces: string[];
  hasToken: boolean;
  indexed: number;
  lastSync: string | null;
  autoUnseal: boolean;
  autoSealIdleSecs: number;
  autoSync: boolean;
  autoLoadToken: boolean;
  initPath: string;
  seal: SealStatus;
  idleSecs: number;
  service: {
    name: string;
    installed: boolean;
    running: boolean;
    state: string;
    message: string;
  };
};

type SealStatus = {
  reachable: boolean;
  healthCode: number;
  initialized: boolean;
  sealed: boolean;
  progress: number;
  threshold: number;
  version: string | null;
  message: string;
};
```

## Tipo de Datos: SearchHit

```typescript
// Sin `id`: el índice se reescribe en cada sync, así que un rowid podría acabar
// apuntando a otro secreto. La identidad estable es (namespace, path, key).
type SearchHit = {
  namespace: string;
  path: string;
  key: string;
  useCount: number;
  lastUsedAt: string | null;
  score: number;
};
```

Al copiar, el frontend manda `{ namespace, path, key }` a `reveal_and_copy`.

## Tipo de Datos: SearchReply (respuesta de `op: "search"`)

```typescript
// El índice por pipe NO devuelve `score` (es orden interno): devuelve los
// nombres y el uso, ya ordenados. `namespace` viene como "root" si es el vacío.
type SearchReply = {
  ok: boolean;
  count: number;
  hits: {
    namespace: string;
    path: string;
    key: string;
    useCount: number;
    lastUsedAt: string | null;
  }[];
};
```

## Tipo de Datos: IngestForm

```typescript
type IngestForm = {
  id: number;
  namespace: string;
  path: string;
  keys: string[];
  title?: string | null;
};
```

---

**Última actualización**: 2026-09-19  
**Versión**: 0.1.0
