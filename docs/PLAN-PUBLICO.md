# edgePass público: un binario que crece sin recompilar

> Plan, no código todavía. Fecha: 2026-10-18. Estado: propuesta para decidir.

## 0. Premisa

Cualquier developer clona el repo, corre un `.exe` (o un instalador), escribe **un
archivo de configuración** y tiene el widget funcionando con lo que él tenga:
OpenBao instalado (prerequisito para las features de secreto), su propia lista de
servidores MCP, sus runners, y los **providers** que quiera enchufar — incluido el
nuestro (Q-Ring), que es cerrado y viaja aparte.

Dos cosas tienen que ser verdad para que eso funcione:

1. **Nada del código conoce esta máquina.** Hoy hay rutas y nombres nuestros
   sembrados como `DEFAULT_*` (ver §1).
2. **Un LED nuevo no es un cambio en Rust.** Hoy lo es: `PROBES` / `ACTIONS` en
   `src-tauri/src/toggles.rs` más un brazo en `probe_state` / `run_action`. El
   `docs/TOGGLE-CONTRACT.md` ya deja anotado que el provider declarativo quedó
   *deliberadamente* sin construir por el límite de confianza (§3 de este doc lo
   resuelve).

## 1. Lo que ya está bien (y no hay que tocar)

- **El repo no tiene secretos.** `git ls-files` no devuelve ningún `.exe`, `.log`,
  `.db`, `.pem`, `.key`, init file ni `config.json`. El archivo más grande es
  `src/App.tsx` (160 KB). Los secretos viven en OpenBao y el gate secret se lee
  **al arrancar el hijo** (`secretEnv`), nunca se escribe en config.
- **Ya hay precedente de "todo es una fila de config"**: `mcpServers[]` y
  `ghaRunners[]` son datos, con panel propio y validación compartida. Los LEDs son
  el único vocabulario que sigue siendo código.
- **Ya hay versionado de config** (`configVersion`, hoy 4) y validación
  (`toggles::validate`, `mcp::validate`).

## 2. Lo que hoy ata el binario a esta máquina

| Dónde | Qué | A dónde va |
|---|---|---|
| `qring.rs` | `DEFAULT_PACK = C:\proyectos\hermes-q-ring-pack` | manifiesto del provider |
| `skills.rs` | `DEFAULT_SCRIPT = C:\proyectos\skills-sync\skills_sync.py` | manifiesto del provider |
| `apps.rs` | `\\.\pipe\daemoncurier` + `DEFAULT_EXE` | manifiesto del provider |
| `gha.rs` | `C:\Program Files\Docker\Docker\Docker Desktop.exe`, `docker\gha-local` | config + detección, no constante |
| `config.rs` | `initPath` por defecto a `C:\proyectos\secrets\...` | `edgepass init` lo pregunta |
| `toggles.rs` | vocabulario de LEDs en una tabla Rust | manifiestos (§3) |
| — | no hay CLI: `main()` sólo abre la ventana | `edgepass init\|doctor\|validate\|provider` (§5) |

## 3. La pieza central: **provider manifest v1**

Un provider es una carpeta con un manifiesto y los comandos que sabe correr. El
binario lo **descubre**, valida y publica sus probes/acciones en el mismo
`catalog` que ya viaja por `get_status` y por `{"op":"toggles"}`. Ni la UI ni el
pipe saben si un LED es built-in o de un tercero.

```yaml
# providers/qring/provider.yaml
manifest: 1
id: qring
label: Q-Ring
homepage: https://…            # informativo
description: Gateway de Q-Ring (pack cerrado, se instala aparte)

probes:
  - id: qring.process           # namespace obligatorio: <provider>.<algo>
    doc: "Gateway Q-ring: proceso vivo o parado."
    cmd: ["{dir}/venv/Scripts/python.exe", "{dir}/qring.py", "status", "--json"]
    timeout_ms: 3000
    # Contrato de salida: rc 0 = on, rc 2 = off, rc 3 = warn/roto.
    exits: { 0: on, 2: off, 3: warn }
    parse: json                 # stdout: {"detail": "…", "live": true}
    poll: slow                  # slow = se pide sólo si algún LED lo usa

actions:
  - id: qring.on
    doc: "Arrancar el gateway Q-ring (no toca la política)."
    needs_vault: true           # el binario abre el vault; el provider no ve el token
    cmd: ["{dir}/venv/Scripts/python.exe", "{dir}/qring.py", "enable"]
    toast: "Q-Ring encendido"
  - id: qring.off
    cmd: ["{dir}/venv/Scripts/python.exe", "{dir}/qring.py", "disable"]
  - id: qring.govern.on
    needs_vault: true
    cmd: ["{dir}/venv/Scripts/python.exe", "{dir}/qring.py", "govern", "on"]

defaults:                      # lo que `edgepass provider add` propone como LEDs
  toggles:
    - { id: qr, label: QR, probe: qring.process, on: qring.on, off: qring.off, alt: qring.restart }
    - { id: gov, label: GOV, probe: qring.governance, on: qring.govern.on, off: qring.govern.off }
```

Reglas del contrato (las pone el binario, no el manifiesto):

- `{dir}` se sustituye por la carpeta del provider; prohibido `..` y rutas
  absolutas en `cmd` (se resuelven contra `{dir}` o el `PATH`).
- **Sin red en v1.** No hay campo `url`/`method`/`body`. Un LED no puede hablar
  con nadie; si algún día hace falta, se abre con allow-list de destinos y
  revisión, como ya advierte `TOGGLE-CONTRACT.md`.
- **Secretos sólo por referencia**: `secretEnv: [{env: FOO, vault: {path, field}}]`.
  El binario es el único que lee OpenBao y los inyecta en el entorno del hijo; el
  valor nunca viaja al manifiesto, a la UI ni al log (regla que ya rige).
- Salida con tope de bytes y redacción: si un comando imprime algo que parece un
  secreto, se corta y se avisa.
- **Confianza explícita**: instalar un provider es un acto (`edgepass provider add
  <dir>`), se guarda el hash del manifiesto en la config y el binario avisa si
  cambió. Un provider no se auto-instala desde la red.

Esto es exactamente lo que pediste para el QR: **el pack trae su `provider.yaml`**;
el usuario apunta la carpeta (o corre `edgepass provider add C:\...\pack`) y los
LEDs `QR`/`GOV` aparecen disponibles como los de MCP. El binario no sabe que
Q-Ring existe.

## 4. Configuración: un archivo, versionado, con esquema

`%APPDATA%\edgePass\config.yaml` (JSON se sigue aceptando: se detecta por la
primera llave y se migra). Secciones:

```yaml
version: 1

vault:                      # opcional: sin esto, sólo se apagan las features de secreto
  addr: http://127.0.0.1:8200
  mount: kv
  namespaces: [""]
  initFile: ~/.config/edgepass/openbao-init.json
  autoUnseal: true

ui:                         # lo que hoy vive en Ajustes
  language: es
  dock: left
  autoCollapseIdleSecs: 10
  maxToggles: 6
  toggles: []               # los LEDs, con ids del catálogo (built-in o provider)

providers:                  # de dónde salen los vocabularios
  - path: providers/qring
    hash: sha256:…          # pinneado al instalar
  - path: providers/gha
  - path: providers/skills

mcp:                        # lo que hoy es mcpServers[]
  - id: reasonixmcp
    command: …
    publicUrl: https://…
    tunnel: edgepass-mcp
    secretEnv: [{ key: REASONIXMCP_GATE_SECRET, path: personal/reasonixmcp, field: GATE_SECRET }]
    autostart: true

runners: []                 # lo que hoy es ghaRunners[]
```

- Se publica `schema/edgepass.schema.json`: autocompletado en VS Code y validación
  en `edgepass validate` (mismo código, un solo lugar).
- `configVersion` sigue: migraciones idempotentes y un `edgepass migrate`.
- Claves desconocidas: se **conservan** (no se pisan) y se avisan, para que un
  downgrade no borre la config de un provider nuevo.

## 5. CLI (lo que convierte "clonar" en "usar")

```
edgepass                       # la ventana (lo que hoy hace el .exe)
edgepass init [--dir .]        # escribe config.yaml mínimo + elige vault
edgepass doctor                # el verify-mcp.ps1 de hoy, generalizado y en Rust
edgepass validate [config]     # esquema + ids + providers
edgepass provider add|list|remove|verify
edgepass run <toggle-id>       # lo mismo que {"op":"toggle"}, para scripts/CI
edgepass migrate
```

`doctor` es la pieza que hace que "sólo falte la config" sea verdad: recorre la
cadena (binario del vault, servicio, seal-status, mount v1/v2, token, puerto del
server MCP, proceso, túnel, endpoint 401) y dice **qué eslabón falta y con qué
comando se arregla**, sin abrir la UI.

## 6. Distribución

- `.exe` autocontenido (ya) + MSI/NSIS **firmados**; `winget` y `scoop` con
  manifiestos en el repo; Releases con `SHA256SUMS`.
- La app **no se autoactualiza** en v1: un widget con acceso al vault no cambia de
  binario sin firma. `edgepass update --check` informa.
- `SECURITY.md` con el modelo de amenaza (loopback only, sin valores por el pipe,
  providers con hash) y cómo reportar.
- Licencia: MIT (o Apache-2.0) para el binario; el pack de Q-Ring es propietario y
  se instala aparte — el README lo dice en la primera línea para que nadie se
  confunda.

## 7. Qué NO se abre nunca

El núcleo de confianza se queda en Rust, con tests, y los providers lo **piden**
pero no lo implementan: apertura/cierre del vault (`net::assert_local` +
keys del init file), el Allow/`request` del comité, la lectura de secretos, la
supervisión de procesos (pid propio primero, imagen sólo si es distintiva) y la
inyección de `secretEnv`. Un provider puede *decidir* cuándo un LED está on; no
puede leer un secreto ni hablar por el pipe.

## 8. Fases

| Fase | Qué | Por qué en este orden |
|---|---|---|
| **F0** (~1 día) | LICENSE, SECURITY, CONTRIBUTING; sacar `DEFAULT_*` a config/env; `docs/` ordenado; README de "instalá OpenBao, corré esto" | sin esto el repo no es publicable, y es barato |
| **F1** | `config.yaml` + `schema/edgepass.schema.json` + `edgepass init/validate/doctor` + migración desde `config.json` | es la premisa literal: "lo único que necesito es la config" |
| **F2** | contrato de provider v1 + loader + `provider add/verify` + **Q-Ring como primer provider** | prueba de fuego: si QR entra sin tocar Rust, el diseño está bien |
| **F3** | portar `gha`, `skills`, `apps` a providers; `vault` y `docker` quedan built-in (son primitivas, no plugins) | cierra la deuda de "dos maneras de hacer un LED" |
| **F4** | firma de binarios, winget/scoop, `edgepass update --check`, docs "tu primer LED en 10 minutos" | recién con F1–F3 estable |

## 9. Riesgos

- **Manifiesto = código ejecutable.** Lo mitiga el pinneo por hash + acto
  explícito + no-red + secretos por referencia. Aun así, `provider add` tiene que
  mostrar *qué comandos* va a correr antes de aceptar.
- **Dos rutas de LED (built-in y provider) durante F2–F3** pueden divergir. Se
  evita portando los built-ins a manifiestos internos en F3, no después.
- **El vault como prerrequisito** mata la primera impresión si es obligatorio (ver
  decisión abierta).
- **Windows-first**: el manifiesto es portable, pero `scheduled task`, keychain y
  pipes son por plataforma; cada uno necesita su adaptador para que el "cualquier
  developer" incluya macOS.
