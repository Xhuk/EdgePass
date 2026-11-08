# edgePass

A desktop widget (Windows; macOS is a port, not a build flag). **It is not a server**: it
is an `.exe` that talks to your local OpenBao.

> Spanish translation: [OPERATIONS.es.md](OPERATIONS.es.md). English is the source.

## Vault behaviour (Rust)

1. **When you need a secret** (copy / sync / new) → if OpenBao is sealed, **Rust opens it
   by itself** using the keys in the local init file (it never shows them).
2. **Sealing is opt-in.** By default edgePass **does not re-seal** the vault: sealing cuts
   off *every* client (other projects, scripts, services), so it must not happen just
   because the widget sat idle. For the old behaviour set `autoSealIdleSecs` > 0 in
   Settings; `0` = never.
3. The **Open / Close** button on the pill forces the state now, when you decide.
4. **KV v1 or v2, mount by mount.** The index reads `sys/mounts` in every namespace and
   speaks the paths of whichever version `kv/` has (v2: `data`/`metadata`; v1: the path
   directly, no envelope). It used to assume v2 everywhere: a namespace with a v1 mount
   (or a different engine) contributed **not a single row**, silently. The log leaves one
   line per namespace: `kv: namespace <ns> has mount 'kv' at KV v1|v2`.

Old configs carrying `300` are migrated to `0` once at startup (noted in the log).

## When something does not start

Before touching anything, open the **Logs** tab: it says which binary is running, with
which build date, which WebView2 version is present, where the config, the database and
the **log** live, and the last lines of that log. *Copy report* puts it on the clipboard,
ready to paste.

- Log: `%LOCALAPPDATA%\edgePass\logs\edgepass.log` (rotates to `.1` at 4 MB).
- Startup errors and panics go there too: a release build has no console.

## LLM → secret form

With edgePass running, an agent can **ask first** whether the secret already exists — names
only, never values, and without opening the vault:

```json
{"op":"search","query":"infisi cb2","limit":20}
```

It answers `{ ok, count, hits: [{ namespace, path, key, useCount, lastUsedAt }] }`. If the
search misses something you know exists, the index is stale: `{"op":"sync_index"}`
rebuilds it (answers `{ ok, count }`) — it is the `↻` in the UI, without opening it. If it
is genuinely missing, ask for the form (which does not see values either):

```powershell
powershell -File scripts\edgepass-ingest-form.ps1 `
  -Namespace buenaidee `
  -Path "projects/cb2/infisical" `
  -Keys DATABASE_URL,DB_USER,DB_PASSWORD,API_TOKEN `
  -Title "CB2 Infisical"
```

The equivalent JSON over the named pipe `\\.\pipe\edgepass` (not stdin):

```json
{"op":"ingest-form","namespace":"buenaidee","path":"projects/cb2/infisical","keys":["DATABASE_URL","DB_USER","DB_PASSWORD","API_TOKEN"]}
```

The window grows to fit the rows (no scrolling). You paste the values and save the batch.

### Seeing a secret's data in Batch

`search` answers **what exists**; `meter` brings the **values to the screen**, which is what
you ask for when you tell an agent «use edgepass». The value **never crosses the pipe**: the
op only names a path, and the widget reads it with its own token and opens the **Batch** tab
with the rows already filled.

```json
{"op":"meter","query":"betty"}
```

```json
{"op":"meter","namespace":"prod","path":"projects/app/users"}
```

```powershell
powershell -File scripts\edgepass-meter.ps1 -Query "betty"
```

It answers `{ ok, id, namespace, path }` — names, never values. A `query` only opens if
**all** its hits land on the same path; if they land on several it answers `ambiguous` with
the `candidates` list instead of guessing.

Once in the panel: each row has **Variable** and **Value**, with `show`/`hide` for that row
and `⧉` to copy that value. The footer has `⧉ copy all` (the block as `KEY=value` lines) and
`show all`/`hide all`. Values open visible and **auto-mask after 8 s**; the eye reveals them
again. It is still the usual editor: `Enter` or ✓ saves a new version to OpenBao, and
*Discard* writes nothing.

That is why an agent asking for a secret **does not end up holding the value** (nothing to
paste into a chat, a log or a commit), and the remote channel (Grok / HTTP) still sees names
only. None of this writes values to disk: the log records the path and how many fields,
never which ones.

### Running a command with secrets

`run` injects the secrets into the **child process environment**: they are never written to
disk.

```json
{"op":"run","entries":[{"namespace":"buenaidee","path":"projects/cb2/infisical"}],"cmd":["npm","run","migrate"]}
```

If the command hangs, edgePass kills it at `timeoutSecs` (300 by default, max 3600) and
answers `{ ok: false, timeout: true, … }` instead of leaving the caller waiting forever. The
kill goes to the process **tree** (`taskkill /T`): killing only `npm.cmd` would leave its
`node` alive, with the pipes open.

`cmd[0]` is resolved through `PATH` + `PATHEXT` before launching, because Windows only adds
`.exe` on its own and `npm` is really `npm.cmd`: the example above works as-is, without
wrapping it in `cmd /c`.

## Environment overrides (not persisted)

For a single run, without touching `config.json`:

| Variable | Effect |
|---|---|
| `EDGEPASS_AUTO_UNSEAL` | `1`/`true`/`yes`/`on` · `0`/`false`/`no`/`off` |
| `EDGEPASS_AUTO_SEAL_IDLE` | idle seconds before sealing (`0` = never) |
| `EDGEPASS_NAMESPACES` | comma-separated list; `.` = root |

The process logs at startup which overrides are in effect, and **Settings never persists
them**: saving from the UI writes what is on screen, not the forced value.

## Usage

- **Keyboard**: `↑` `↓` move the selection, `Enter` copies, `Esc` collapses.
- A click on a result copies the value to the clipboard (it is never shown on screen).
- Drag the pill near an edge to dock it; away, to let it float. It **snaps** to the edge as
  soon as it gets close (the morph happens while you drag), and the **pointer** decides on
  release: dropping it over the edge pins it there — even if you release just past the
  screen edge; to undock, pull more than 96 px inwards.
- **It remembers where it was**: edge and height when docked, or its spot when floating, and
  the next start opens exactly there (not at the system default). It is stored in
  `%LOCALAPPDATA%\edgePass\placement.json` **relative to the monitor**, so a resolution or
  DPI change moves the pill with the screen instead of leaving it out of sight; if the
  monitor it lived on is gone, it returns to the primary one.
- **Quit**: **Logs** tab → *Quit edgePass* (two steps; `Esc` cancels the first). The widget
  has no frame and no taskbar entry, so this is its only orderly exit. Quitting does **not**
  seal the vault: the **KV** LED is for that.
- Opening the `.exe` twice does not start a second instance: it hands focus to the running one.
- **Labels follow the machine**: tab names follow Windows' **regional format** —
  `Secrets · Batch · Credentials · Settings · Logs` in English, `Secretos · Batch ·
  Credenciales · Ajustes · Logs` in Spanish. The language is asked of Rust at startup
  (`GetUserDefaultLocaleName`), because UI language and regional format are different
  settings and the webview cannot tell them apart.
- **Credentials**: search by site, user or domain (`+ Add` opens an expanded row to type the
  credential; on save it joins the list). The list reads the metadata of each
  `personal/logins/*` so you can search by what is inside, not only by the slug: opening the
  tab needs the vault open (it unseals by itself, like everything else).

## Header toggles (KV / GW / GOV…)

The header buttons are **data**, not interface code: each one is a row in `config.json` →
`toggles[]` that says **what it watches** (`probe`) and **what it does** on click (`on` /
`off`, plus `alt` for Ctrl or right click).

```jsonc
{ "id": "kv", "label": "KV", "probe": "vault.seal",
  "on": "vault.unseal", "off": "vault.seal", "alt": null }
```

- **From Settings → Header toggles**: label (1-4 letters), probe, both actions (and the
  Ctrl/right-click one), `↑` `↓` to reorder, `×` to remove and `＋ Add toggle`. Maximum **6**,
  applied when you press *Save*.
- **From the pipe**, without opening the widget:

  ```jsonc
  {"op":"toggles"}                       // read the LEDs + the vocabulary
  {"op":"toggles","set":[ … ]}           // rewrite the list (validated)
  {"op":"toggle","id":"kv","dir":"flip"} // the click: flip | on | off | alt
  ```

  Helper: `powershell -File scripts\edgepass-toggles.ps1 -List` (or `-Catalog`,
  `-Fire kv -Dir off`, `-Set (Get-Content .\toggles.json -Raw)`).

The widget **publishes its vocabulary** (`get_status` and every `toggles` answer carry
`catalog`), so nobody — neither an agent nor a contributor — has to guess an id: an unknown
probe or action is rejected *with* the list of valid ones. An agent names the **LED**, never
the action: which action a click means is resolved in Rust from `config.json`.

- **Fresh install**: only the **KV** LED (the only guaranteed backend is the vault).
  `QR`/`GOV`/`GW` are added from Settings with the probes `qring.process` /
  `qring.governance` / `gha.runner` — so a machine without the Q-ring pack and without
  Docker Desktop does not drag amber lights along.
- **Upgrade**: keeps the usual three (`KV`, `GW`, `GOV`) and the old `GW` moves to **`QR`**
  (v3→v4), because `GW` now watches the local GHA runner: your header ends up
  `KV · QR · GOV · GW` without losing control of the gateway.
- **`GW` is no longer the Q-ring gateway**: `GW` watches **Docker Desktop + the local GitHub
  Actions runner** (the container that deploys for you, without spending hosted minutes).
  Click = a real fix: if Docker does not answer it starts it, if the image is missing it
  builds it, and if the container is down, stopped or looping on a credential, it **recreates
  it with a fresh registration token** (`gh api`). With everything healthy, the click only
  stops the container (`docker stop`: Docker Desktop stays put). `Ctrl`/right click = open
  DaemonCurier. Runbook: [`docker/gha-local/README.md`](../docker/gha-local/README.md) and
  `scripts\gha-local.ps1` for the manual path.
- **The Q-ring process lives in `QR`**: `QR` is the **process** (starts/stops/restarts the
  gateway, `Ctrl`/right click = restart) and `GOV` is the **policy** (whether the rings may
  run *and spend*). They are coupled in one direction so no state can lie: stopping the
  gateway turns the policy off (`qring.off` writes `qringGoverned=false`), and turning the
  policy on with the gateway stopped starts it first (with the vault, like `qring.on`). The
  reverse is legitimate, and it is why `GOV` exists: `GOV` off with `QR` on = gateway up,
  nobody spending. The **Allow pulse** (Allow once / 2h / Deny) lives in `GOV`; the header's
  `Allow` chip shows it even if you removed that LED.
- **Adding a new kind of LED** (probe or action) is a row in `src-tauri/src/toggles.rs` plus
  its arm; the catalog, the Settings pickers, the tooltips and the pipe contract all come
  from those tables.
- Full contract (fields, rules, glossary, mapping to OpenBao's `/v1/sys/*`, and what is
  missing to point a LED at another compatible KV): [`TOGGLE-CONTRACT.md`](TOGGLE-CONTRACT.md).

## Autostart (after the OpenBao service)

OpenBao runs **on its own** as a Windows service (`OpenBao`, NSSM). edgePass is the
**client**: it connects, unseals on use and only seals if you ask. Autostart launches the
widget once at logon, with a native scheduled task (no PowerShell wrapper, no
`-ExecutionPolicy Bypass`) and an 8 s delay for the logon burst. It does not wait for
OpenBao: if the vault is not there yet, the widget says so and offers the Svc button.

```powershell
powershell -File scripts\install-autostart.ps1
powershell -File scripts\install-autostart.ps1 -Remove
```

## Installs and restarts: always warn

Installing the product **kills the process serving the vault**. It happened on 6 November:
a silent 0.4.0 install closed the widget at 11:10:49 without an orderly exit and left the
pipe unserved. Reel found out because it was about to record.

Rules:

1. **No install without warning.** An installer that kills the process serving the vault is
   a service interruption, even if it lasts three minutes.
2. **Before installing, close the widget in an orderly way** and say so. If the installer
   kills the process, the log has no shutdown and it looks like a crash.
3. **After installing, verify in a separate call**: `ping` with the expected version and the
   process pointing at the exe you installed (installer:
   `%LOCALAPPDATA%\edgePass\edgepass.exe`; Scoop: `…\scoop\apps\edgepass\current`).
4. If the process does not start, `Start-ScheduledTask -TaskName edgePass` and verify again.
   Never `Start-Process`: the shell job kills it on close.

## Widget watchdog

There is no automatic watchdog: the `edgePass` task runs **once at logon**
(`MultipleInstances = IgnoreNew`). Relaunching every N minutes was tried and removed: a
widget that died comes back with `Start-ScheduledTask -TaskName edgePass`, and the single
instance rejects the second process before it opens the pipe.
