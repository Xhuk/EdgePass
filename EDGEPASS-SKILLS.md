# edgePass — capabilities for skills

edgePass is a desktop widget that administers secrets from your local OpenBao
vault. Skills can talk to edgePass to manage secrets automatically.

## Window states

edgePass has two main states:

- **Collapsed (dock)**: a small pill on the screen edge with the logo and the
  vault status.
- **Expanded**: the full window with search, management and settings.

## Pipe interface (JSON)

Skills send JSON commands to edgePass over a named pipe:

```json
{"op":"ingest-form","namespace":"buenaidee","path":"projects/cb2/infisical","keys":["DATABASE_URL","DB_USER","DB_PASSWORD","API_TOKEN"]}
```

### Available operations

| Operation | Description | Parameters |
|-----------|-------------|------------|
| `ping` | Checks that edgePass is alive | — |
| `focus` | Brings the pill to the front (does not expand) | — |
| `status` | Vault state + hold | — |
| `search` | Searches the local index: **names only** (namespace, path, key), without opening the vault | `query?`, `limit?` (default 50, max 200) |
| `meter` | **Opens the Batch tab (`meter`) with the secret's data**: resolves a path (or a `query` against the index) and the widget reads its fields and shows them for copying (eye per row · ⧉ copy all). The caller **never sees values** | `query?` **or** `path` (+ `namespace?`, `title?`) |
| `unseal` / `seal` | Open / close the vault now | — |
| `hold` | Do not auto-seal for N seconds | `secs?` (default 3600, max 86400) |
| `run` | Runs a command with the secrets **in the child's environment** (local scripts; no popup) | `entries[]`, `cmd[]`, `cwd?`, `keys?`, `timeoutSecs?` (default 300, max 3600) |
| `request` | GW: asks for a `run` with an Allow once / 2h / Deny popup. Immediate `pending`/`running` reply. **Never values.** | `action=run`, `actor?`, `namespace`, `path` (or `entries[]`), `cmd[]`, `keys?`, `reason?`, `cwd?`, `detach?` |
| `request-status` | Snapshot of one request (or the list when there is no `id`). No values / stdout | `id?` |
| `request-wait` | Waits up to `timeoutSecs` (max 60) for it to leave pending/running | `id`, `timeoutSecs?` |
| `request-resolve` | UI only: `deny` / `allow-once` / `allow-2h` | `id`, `decision` |
| `ingest-form` | Opens the batch form to enter secrets | `namespace`, `path`, `keys[]`, `title?` |
| `sync_index` | Reindexes the local index now (same as the UI `↻`) | — |
| `toggles` | The header LEDs **as data**: read the list + the vocabulary (`catalog`), or rewrite it (validated: catalog ids, max 8) | — · `set[]` |
| `toggle` | Fires a LED (the same as a header click): Rust resolves the action from `config.json`, you only name the LED | `id`, `dir?` (`flip`\|`on`\|`off`\|`alt`) |
| `services` | The owner's services **as data**: read the list + the `svc.*` half of the catalog, or replace it (validated, max 24) | — · `set[]` |

> `export-env` **no longer exists**: it wrote secrets in clear text to whatever
> path the caller asked for. Use `run`, which hands them to a child process and
> never touches disk.

### Search before asking (`search`)

The missing half of the flow: `ingest-form` creates, `search` checks. An agent
can see whether the secret already exists — and under which names — before
bothering the human, and without unlocking anything (the index stores names,
never values):

```json
{"op":"search","query":"infisi cb2","limit":20}
```

```json
{"ok":true,"count":2,"hits":[
  {"namespace":"buenaidee","path":"projects/cb2/infisical","key":"DATABASE_URL","useCount":7,"lastUsedAt":"2026-09-14T18:02:11+00:00"},
  {"namespace":"buenaidee","path":"projects/cb2/infisical","key":"API_TOKEN","useCount":0,"lastUsedAt":null}
]}
```

An empty `query` returns the most used first. Terms are combined with **AND**
(all must appear) and there are short aliases: `infisi`/`infis`/`infiscal`→`infisical`,
`bao`→`openbao`/`vault`, `ob`→`openbao`, `cf`→`cloudflare`, `sb`→`supabase`,
`pg`/`db`→`postgres`.

> The index is local and can be stale: if `search` does not see something
> created outside, hit `↻` in the UI or call `sync_index` before concluding it is
> missing. `sync_index` replies `{ ok, count }` (or `{ ok: false, error }`), and
> it does open the vault when needed: it counts as widget activity.

> **KV v1 and v2**: the index detects per namespace whether the `kv` mount is
> KV v1 or v2 (`sys/mounts`) and uses that version's paths, so namespaces with a
> v1 mount also work. Copy, edit and delete use the same path, so they work on a
> v1 mount too. The log records one line per namespace:
> `kv: namespace <ns> tiene mount 'kv' en KV v1|v2`.

### Seeing a secret's data in the Batch tab (`meter`)

`search` answers *what exists*; `meter` brings the **values** to the screen —
what a human asks for when they say **"use edge pass"** or "give me betty's
account details". And it does so without the value leaving the widget: the op
only names a path, **the widget reads it with its token** and fills the Batch
tab. The caller (an agent) gets no value at all, so it cannot paste it into a
chat, a log or a commit.

```json
{"op":"meter","query":"betty"}
```

```json
{"ok":true,"id":1789…,"namespace":"prod","path":"projects/app/users"}
```

How it is addressed:

| Shape | What it opens |
|---|---|
| `{"query":"betty"}` | The path the results point at, **if they all fall in one** |
| `{"namespace":"prod","path":"projects/app/users"}` | That path, without consulting the index |
| `title?` | Panel title (default "Batch · copy") |

Rules and errors, as they come out:

- **Query → one path.** `betty` falling on the 6 `BETTY_*` fields of one path
  opens that path.
- **Ambiguous**: if they fall in several paths it replies
  `{ ok:false, error:"ambiguo…", candidates:[{namespace,path,key}…] }` — it does
  not guess; repeat with `path`.
- **No matches**: `{ ok:false, error:"sin coincidencias en el índice local
  (puede estar viejo: prueba sync_index)" }`.
- **Path without fields / no token / sealed vault with auto-unseal off**: the
  error appears in the widget (it is the one reading), not in the pipe reply.
- If the panel is already open, `meter` **replaces** it with the new path (one
  pending slot, like `ingest-form`).

What the human does in that panel, inside the widget:

- Each row: **Variable** + **Value**. Values open **visible** and
  **auto-mask after 8s** (`METER_AUTO_MASK_MS`); each row's `show`/`hide` button
  and `show all`/`hide all` bring them back.
- **`⧉` per row** copies that value to the clipboard (without showing it in any
  chat).
- **`⧉ copy all`** copies the whole block as `KEY=value` lines (`.env` format),
  which is exactly what a chat cannot take away.
- It is still the same editor: `Enter` or ✓ saves to OpenBao (creates a new
  version), and "Discard" closes without writing. Nothing auto-saves.

```powershell
# The wrapper for agents and humans
powershell -File scripts\edgepass-meter.ps1 -Query "betty"
powershell -File scripts\edgepass-meter.ps1 -Namespace prod -Path "projects/app/users"
powershell -File scripts\edgepass-meter.ps1 -Query "infisi cb2" -Title "CB2 Infisical"
```

> `meter` is **local pipe only**: it opens a window, so it is not part of the
> remote contract (`request` / HTTP with Grok) — an actor outside this machine
> still receives names only. The pipe is created with a single-ACE DACL (your
> SID), and none of this writes values to disk.

### Usage example

```powershell
# PowerShell — batch form (the human pastes the values, the agent never sees them)
powershell -File scripts\edgepass-ingest-form.ps1 `
  -Namespace buenaidee `
  -Path "projects/cb2/infisical" `
  -Keys DATABASE_URL,DB_USER,DB_PASSWORD,API_TOKEN `
  -Title "CB2 Infisical"
```

Direct JSON pipe (one JSON line per request, to the named pipe
`\\.\pipe\edgepass`):

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

### Running a command with secrets (without writing anything to disk)

```json
{"op":"run","entries":[{"namespace":"buenaidee","path":"projects/cb2/infisical"}],"cmd":["npm","run","migrate"]}
```

It replies `{ ok, code, env[], stdout, stderr }` — only the variable **names**,
never the values. `keys` limits what is injected.

`timeoutSecs` cuts the command (default 300, max 3600). When it expires, it
kills the process **tree** and replies:

```json
{"ok":false,"timeout":true,"timeoutSecs":300,"error":"timeout tras 300s: proceso terminado","code":null,"env":["DATABASE_URL"],"stdout":"…","stderr":"…"}
```

Why the timeout: a hung `run` left the caller waiting forever. The kill takes
the tree because on Windows `npm run x` is `npm.cmd` → `node`: killing only the
first leaves the second alive (with the pipes open).

`cmd[0]` is resolved through `PATH` + `PATHEXT` before launching, so
`["npm", "run", "migrate"]` works as written: Windows only adds `.exe` on its
own, and `npm` is `npm.cmd`. If the binary is not found, the error carries the
name you typed, not a strange resolved path.

## Vault states (seal status)

The vault state appears in the status chip and affects behaviour:

| State | Code | Description |
|-------|------|-------------|
| `open` | 0 | Vault open, secrets accessible |
| `sealed` | 1 | Vault sealed, secrets protected |
| `warn` | 2 | Vault initialised but with warnings |
| `down` | 3 | OpenBao offline or unreachable |

## Automatic behaviour

### Auto-unseal
- On start, if the vault is sealed, **it is unlocked automatically** by reading
  the keys from the local init file.
- The keys are never shown in the UI.
- Setting: `autoUnseal` (default: `true`).

### Auto-seal (opt-in, off by default)
- **By default the vault is never sealed.** Sealing affects the *whole* vault: it
  cuts off the other consumers (other projects, scripts, services). This widget
  being idle is not a valid signal to close their vault.
- If `autoSealIdleSecs` > 0 is configured, it seals again after that idle.
- Value `0` (default) = never auto-seal.
- A "Close" button in the UI seals manually, when you decide to.
- Old configs that carried `300` are migrated to `0` once at startup.

### Auto-sync
- Expanding the window makes edgePass **sync the index** automatically.
- Setting: `autoSync` (default: `true`).
- `↻` button to reindex manually.

### Auto-load token
- Loads the authentication token from the init JSON file automatically.
- Setting: `autoLoadToken` (default: `true`).

## Secret management

### Search
- Search input at the top.
- Indexed search across the configured namespaces.
- The result shows: key, namespace, path, recent use.

### Copy a secret
- Clicking a result copies the value to the clipboard.
- Shows a toast with the copied secret's label.
- Increments the use counter (`useCount`).

### New secret ("New" panel)
- Form to add a single secret.
- Fields: Namespace, Path, Variable, Value.
- Option: "Replace whole path" (overwrite existing).

### Batch panel
- Opened automatically through the `ingest-form` pipe, or filled from the vault
  through `meter`.
- Table of variables and values to fill in (the human pastes the values) or
  already read.
- Each row: `show`/`hide` the value and `⧉` to copy it · the footer:
  `⧉ copy all` (`KEY=value` lines) and `show all`/`hide all`.
- `meter` values auto-mask after 8s (`METER_AUTO_MASK_MS`) and the eye shows
  them again.
- Option: "Replace whole path".
- Save the batch to the vault with one click.

## Settings ("Config" panel)

| Parameter | Description | Default |
|-----------|-------------|---------|
| `baoAddr` | OpenBao service URL | `http://127.0.0.1:8200` |
| `mount` | Vault mount point | `kv` |
| `namespaces` | Namespaces to index (comma-separated). `.` = root **+ all children** through `sys/namespaces` | `.` |
| `initPath` | Path to the init JSON (token+keys) | `` |
| `autoSealIdleSecs` | Idle seconds before sealing (0 = never) | `0` |
| `token` | Manual token (when `initPath` is not used) | `` |

### Action buttons (header LEDs)

The header LEDs (`KV`, `GW`, `GOV`…) **are data**: each one is a row of
`config.json` → `toggles[]` with a `probe` (what it watches) and `on` / `off` /
`alt` (what a click does · `alt` = Ctrl or right click). They are edited in
**Settings → Header toggles** (max **8**, label 1-4 letters, `＋ Add a toggle`)
or over the pipe.

Vocabulary (the widget publishes it in `catalog`, never guess it):

| `probe` | ON when |
|---------|---------|
| `vault.seal` | vault reachable and open (amber: no answer or not initialised) |
| `qring.process` | `qringd` alive (amber: pack missing) — the LED is called **`QR`** |
| `qring.governance` | rings may run (and spend) — **and it is the LED that pulses when an Allow is waiting** |
| `docker.desktop` | the Docker Desktop engine answers (amber: installed but stopped) |
| `gha.runner` | Docker up **and** the local GitHub Actions runner healthy — the **`GW`** LED |
| `skills.sync` | the skill library is in phase: 0 links to fix (amber: drift, or the syncer is missing) |
| `skills.watch` | the hub watcher is running (auto-sync) |
| `mcp.servers` | every `mcpServers[]` row is up and every declared tunnel is connected |
| `mcp.<id>` *(derived)* | that one MCP row is up and its tunnel (if declared) is connected |
| `svc.<id>` *(derived)* | that `services[]` row's `process`/`cmd` probe says on |

| `action` | Effect |
|----------|--------|
| `vault.unseal` / `vault.seal` | open (init keys) / seal (token; cuts off every client) |
| `qring.on` / `qring.off` / `qring.restart` | start / stop / restart the gateway. **Stopping also turns the policy off** (a stopped gateway governs nothing); starting does not turn it on |
| `qring.govern.on` / `qring.govern.off` | allow / block the rings from running. Turning it on with the gateway stopped **starts it first** (vault included); turning it off is valid with the gateway up, which is the case `GOV` covers |
| `docker.up` | start Docker Desktop and wait for the engine (up to 120 s) |
| `gha.up` | ensure the local runner: Docker, image (built if missing) and container — **recreates it with a fresh token** if it is down, stopped or in the credential loop |
| `gha.down` | stop the runner container (Docker Desktop is left alone) |
| `gha.repair` | recreate the runner even when it looks healthy (if GitHub sees it `offline`) |
| `daemoncurier.open` | open/focus DaemonCurier (the WSL2/system HUD) |
| `skills.sync` | re-link the agents from the skills hub (does not copy files) |
| `skills.watch.on` / `skills.watch.off` | turn the hub auto-sync on / off |
| `mcp.on` / `mcp.off` / `mcp.restart` | the whole MCP list and its tunnels |
| `mcp.<id>.on\|off\|restart` *(derived)* | one MCP row and its tunnel |
| `svc.<id>.on\|off` *(derived)* | exactly the command that service declared in `actions` |

```jsonc
{"op":"toggles"}                                     // LEDs + catalog
{"op":"toggles","set":[{"id":"kv","label":"KV","probe":"vault.seal",
                        "on":"vault.unseal","off":"vault.seal"}]}
{"op":"toggle","id":"kv","dir":"on"}                 // flip | on | off | alt

{"op":"services"}                                    // services + svc.* vocabulary
{"op":"services","set":[{"id":"mi-api","label":"API",
                         "probe":{"kind":"process","image":"mi-api.exe"}}]}
```

`{"op":"status"}` also returns `toggles`. An unknown id is rejected **with the
catalog in the reply**. Helper:
`powershell -File scripts\edgepass-toggles.ps1 -List` (`-Catalog`,
`-Fire kv -Dir off`, `-Set <json>`, `-Services`, `-SetServices <json>`). Full
contract: `docs/TOGGLE-CONTRACT.md`.

- **Fresh install**: only `KV` (the only guaranteed backend is the vault).
- **Upgrade**: keeps the usual three (`KV`, `GW`, `GOV`) and the old `GW` — which
  was the Q-ring process — becomes `QR` (v3→v4), because `GW` now watches the
  local GHA runner. The result is `KV · QR · GOV · GW`.
- **Local GHA runner (`GW` LED)**: image `gha-local:latest`
  (`docker/gha-local/`), the container registers to a repo with labels
  `self-hosted,Linux,X64,local` and the workflow uses it with
  `runs-on: [self-hosted, Linux, X64, local]`. The rows live in `config.json` →
  `ghaRunners[]` (id, repo, labels, image, `secretEnv` with OpenBao
  credentials), and the widget mints the registration token with `gh api` at
  startup. Manual path: `scripts\gha-local.ps1` (`-Add`, `-Build`, `-Ensure`,
  `-Status`, `-Logs`, `-Down`, `-Watch`). Runbook: `docker/gha-local/README.md`;
  the local pipeline (build → GHCR → thin deploy) is in
  `scripts\build-and-deploy.ps1` + `docker/gha-local/deploy.yml`.
- **Seeing what runs and why it failed**: the runner does not dump step output
  to its stdout, so there are two sources: `docker logs gha-local-<id>`
  (lifecycle: which job runs, how it ended) and `_diag/Worker_*.log` inside the
  container (the step detail). The `GW` LED's `detail` says the job in flight,
  and over the pipe: `{"op":"gha"}` (state + job) and
  `{"op":"gha-logs","id":"cb2","tail":200,"source":"both"}`. The widget opens
  the viewer by itself when a job starts (`ghaWatch`), or
  `scripts\gha-local.ps1 -Watch -Id cb2` by hand.
- **A repo builds locally but not on GitHub**: `runs-on` decides. Register the
  runner to the repo (`gha-local.ps1 -Add`/`-Ensure`) + the repo variable
  `RUNNER` with `["self-hosted","Linux","X64","local"]` (through the API with the
  JSON on **stdin**: with `gh variable set --body`, PowerShell 5.1 stores the
  value without the quotes and `fromJSON()` blows up) +
  `runs-on: ${{ fromJSON(vars.RUNNER || '["ubuntu-latest"]') }}`. Reverting means
  deleting the variable. And for **zero GitHub** in the middle (queueing
  nothing): `scripts\act-local.ps1` (nektos/act in Docker).
- **A new kind of LED**: one row in `src-tauri/src/toggles.rs` (`RAW_PROBES` /
  `RAW_ACTIONS`) plus its arm in `probe_state` / `run_action`; the rest follows.
  A *service* LED (the common case) needs no Rust: it is `services[]` data and
  the `svc.<id>` probe, see `skills/edgepass-led`.
- **Skills (`SYNC` / `AUTO`)**: opt-in, not in the defaults because they point
  at `C:\proyectos\skills-sync\skills_sync.py` (a machine tool, not a product
  one; it can be changed with `EDGEPASS_SKILLS_SYNC`). The bridge is
  `src-tauri/src/skills.rs`: the LED reads a 15 s cache (a full `status` takes
  ~1 s and must not block the UI) and the actions run the script in
  `spawn_blocking`. The watcher is a **child process** of edgePass: it dies with
  it (to have it live from startup, `skills_sync.py install-autostart`).

## Events and listeners

edgePass emits events that skills can listen to:

### `ingest-form`
- Emitted when a batch form is waiting to be filled in.
- Payload: `{ id, namespace, path, keys[], title? }`.
- The window expands automatically to show the form.

### `grant-request`
- Emitted when Grok/GW asks for a `run` and there is no 2h grant for that path.
- Payload: `{ id, actor, action, namespace, path, keys[], cmd[], reason, expiresAt }` (names; never values).
- Allow once / 2 hours / Deny overlay; Esc = deny.

### `get_status`
- Returns the current vault state and configuration.
- Full payload: `StatusDto` (see the type in code).

## LLM/agent integration

### GW (Grok) — search + run with Allow

Grok **never receives values**. It lists names; to use a secret it asks for a
`run` on this node. The human sees Allow once / 2 hours / Deny. The env lives
only in the child process.

HTTP (Bearer `EDGEPASS_OPERATOR_TOKEN`, Tailscale `:7437`):

1. `GET /v1/secrets/search?query=infisi+cb2` — names. No popup.
2. `POST /v1/secrets/request` `{actor:"grok", action:"run", namespace, path, keys?, cmd:[], reason}` → `202` + `requestId` (`pending`, or `running` when a 2h grant exists for the same path).
3. Poll `GET /v1/secrets/request?id=` every 2–5s (max ~5 min) until `granted|denied|timeout`.
4. If `granted`: read `ok`, `env[]` (names), `code`. **Do not** scrape stdout (it does not come over this channel).
5. If `denied` / `timeout` / `sealed`: stop. Do not invent values. Do not ask for them to be pasted.

Local pipe (same contract, `search` / `request` / `request-status` ops). The
pipe's direct `run` **does not** open a popup (scripts).

> `meter` is **not** part of this contract: it opens the widget window and
> therefore lives only on the local pipe, for agents running on your own machine
> (see *Seeing a secret's data in the Batch tab*).

2h grant: same `actor + namespace + path`. Another path = another Allow. It is
lost when the widget dies.

### Recommended flow for entering secrets

1. **The LLM identifies** that it needs secrets for a task.
2. **The LLM asks first** with `op: "search"` — if they already exist, it
   bothers nobody.
3. **The LLM sends** JSON pipe with `op: "ingest-form"` for what is missing.
4. **edgePass** opens the batch form automatically.
5. **The user** fills in the values in the UI (the LLM never sees them).
6. **The user** clicks "Save batch to vault".
7. **edgePass** stores every secret and closes the form.

### Full example

```powershell
# 1. edgePass running in the background
.\edgepass.exe

# 2. The LLM checks whether they already exist (names only, without opening the vault)
$json = '{"op":"search","query":"infisi cb2","limit":20}'
# → {"ok":true,"count":0,"hits":[]}

# 3. The LLM sends the command to enter what is missing
$json = @{
    op = "ingest-form"
    namespace = "buenaidee"
    path = "projects/cb2/infisical"
    keys = @("DATABASE_URL", "DB_USER", "DB_PASSWORD", "API_TOKEN")
    title = "CB2 Infisical"
} | ConvertTo-Json

$json | .\edgepass.exe

# 4. edgePass opens the form, the user fills in the values and saves
```

## Environment variables

Per-run overrides, **without writing `config.json`** (they are never persisted:
saving in Config writes what is on screen, not the forced value). The process
logs them when it detects them.

| Variable | Description |
|----------|-------------|
| `EDGEPASS_AUTO_UNSEAL` | Overrides `autoUnseal` (`1`/`true`/`yes`/`on` · `0`/`false`/`no`/`off`) |
| `EDGEPASS_AUTO_SEAL_IDLE` | Overrides `autoSealIdleSecs` (seconds; `0` = never seal) |
| `EDGEPASS_NAMESPACES` | Overrides `namespaces` (comma-separated; `.` = root + all children) |

```powershell
$env:EDGEPASS_NAMESPACES = "., buenaidee"
$env:EDGEPASS_AUTO_SEAL_IDLE = "0"
.\edgepass.exe
```

Invalid values (text where a number goes, a boolean that is not a boolean) are
ignored with a warning in the log: a typo cannot leave the widget without
config.

## Security

- The init JSON keys are **never shown** in the UI.
- The token is loaded automatically but not exposed, and it lives in the OS keyring.
- Sealing the vault is **opt-in**: by default edgePass does not close it on
  anyone (see Auto-seal).
- `baoAddr` must be loopback; any other host is rejected **before** the token is
  sent.
- No operation writes secrets to disk: the value leaves only through the
  clipboard (your clicks in the widget) or a child process's environment (`run`).
- No op **returns** a value to the caller: `meter` only names a path and the
  widget reads the fields to show them on screen, so an agent does not end up
  holding the secret. The remote contract (`request` / HTTP) remains names-only,
  and a value is never logged (the log records path and how many fields, not
  which ones).
- The named pipe `\\.\pipe\edgepass` is created with a **single-ACE DACL**: full
  access for the SID running edgePass and nothing else (no `Everyone`, no
  `Authenticated Users`, no inherited ACEs). Any process of *your* user can talk
  over the pipe — that is the trust boundary, and it is the same as always; what
  no longer happens is another account on the machine being able to open it. If
  the DACL cannot be built, the pipe is created unrestricted and the log records
  it (it is never pretended to be restricted).
- Only secrets from configured namespaces are indexed (`.` discovers and indexes
  all of them).
- The index stores **names** (namespace, path, variable), never values — and
  `search` never answers with more than those names.
- `run` kills the process **tree** when its `timeoutSecs` expires (default
  300s), so a hung command does not leave the caller hanging.
- The UI has a **Logs** panel with the running binary, the log path, the vault
  state and the last log lines.
- Closing the widget (Logs → *Quit edgePass*, two steps) **does not** seal the
  vault, and vice versa: sealing does not close the app.
- Opening the `.exe` twice does not create a second instance: it focuses the
  running one.

## UI shortcuts

| Shortcut | Action |
|----------|--------|
| Click the logo | Expand/collapse the window |
| Click a result | Copy the value to the clipboard |
| Click "Open"/"Close" | Force unseal/seal the vault |
| Click "Svc" | Start the OpenBao service |
| Click `↻` | Reindex secrets |
| Drag the window near an edge | Dock to that edge |
| Drag far from the edge | Float as a pill |
| `↑` `↓` + `Enter` | Move through the results and copy the selected one |
| `Esc` | Cancel the notice / disarm "Quit" / collapse the panel |
| Logs → *Quit edgePass* | Close the app (two steps; the vault stays as it is) |

## Script files

- `scripts\edgepass-ingest-form.ps1` — PowerShell wrapper for ingest-form
- `scripts\edgepass-meter.ps1` - PowerShell wrapper for meter (opens the Batch tab with one secret's data)
- `scripts\edgepass-toggles.ps1` — the header LEDs and `services[]` over the pipe
- `scripts\launch-edgepass.ps1` — launch edgePass
- `scripts\install-autostart.ps1` — install autostart after the OpenBao service

## Data type: StatusDto

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
  // The header LEDs (spec + state + docs) and the vocabulary they are written in.
  toggles: ToggleView[];
  catalog: ToggleCatalog;
  // The MCP list with live process state.
  mcp: McpStatus;
  // The owner-defined services behind `svc.<id>`.
  services: ServiceSpec[];
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

## Data type: SearchHit

```typescript
// No `id`: the index is rewritten on every sync, so a rowid could end up
// pointing at another secret. The stable identity is (namespace, path, key).
type SearchHit = {
  namespace: string;
  path: string;
  key: string;
  useCount: number;
  lastUsedAt: string | null;
  score: number;
};
```

When copying, the frontend sends `{ namespace, path, key }` to
`reveal_and_copy`.

## Data type: SearchReply (reply to `op: "search"`)

```typescript
// The pipe index does NOT return `score` (it is internal ordering): it returns
// the names and usage, already sorted. `namespace` comes as "root" when empty.
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

## Data type: IngestForm

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

**Last updated**: 2026-11-06
**Version**: 0.4.2
