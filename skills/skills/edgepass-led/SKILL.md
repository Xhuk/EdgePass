---
name: edgepass-led
description: Use when a human or an agent wants to add, change or remove a header LED in edgePass — to watch a local service, an MCP server, a container, a Windows service or a policy flag, and to start and stop it with a click. Covers the interview, the probe kinds, the canonical JSON, the LED budget and the verification. Trigger on "agregar un led", "dar de alta un led", "monitorear esto en edgepass", "que se vea si X está corriendo", "sacar un led", "leds del header".
---

# Registering a LED

A LED answers three questions and nothing else:

1. **Is it up?** — the `probe`.
2. **What does a click do?** — the `on` / `off` / `alt` actions.
3. **Who started it?** — the reported `origin`, which decides whether stopping it is safe.

Today's LED is a row in `config.json` → `toggles[]`. The vocabulary behind it (which
probes exist, what each one reads) is published by the widget itself. Read it, do not
guess it.

## Rule zero: the binary is immutable, the inventory is not

This is a per-widget install, so the question is always "do I rebuild the exe or do I
add data?". The answer is data. Three layers, and only one of them changes per machine:

| Layer | Decides | Who writes it | Per machine? |
|---|---|---|---|
| **Engine** (the exe) | how to read a process, how to run a command, the UI | the release | no |
| **Inventory** (config) | which services, labels, click actions | the human, or you | **yes** |
| **Provider** (pack) | a new *kind* of probe or action | a third party, hash-pinned | optional |

You add **services**, never **kinds**. If the kind you need is not in `catalog`, the
build cannot do it: say so, or point at a provider. Never patch the exe, never invent a
probe id, never claim a light you did not register.

## Step 0 — read the catalog

```powershell
powershell -File scripts\edgepass-toggles.ps1 -Catalog
```

Or over the pipe:

```powershell
function Invoke-EdgePass {
  param([Parameter(Mandatory)][string]$Json, [int]$TimeoutMs = 3000)
  $pipe = New-Object System.IO.Pipes.NamedPipeClientStream('.', 'edgepass', [IO.Pipes.PipeDirection]::InOut)
  try {
    $pipe.Connect($TimeoutMs)
    $bytes = [Text.Encoding]::UTF8.GetBytes(($Json.Trim()) + "`n")
    $pipe.Write($bytes, 0, $bytes.Length); $pipe.Flush()
    $reader = New-Object IO.StreamReader($pipe, [Text.Encoding]::UTF8, $false, 1024, $true)
    $reader.ReadLine()
  } finally { $pipe.Dispose() }
}

$r = Invoke-EdgePass '{"op":"toggles"}' | ConvertFrom-Json
$r.catalog.probes  | Select-Object id, on, off, alt
$r.catalog.actions | Select-Object id, needsVault
```

The reply is the truth **for this build**: `maxToggles`, `maxLabelChars`, every probe
id, every action id, and the `doc` that the tooltip shows. If the id you need is not in
that list, stop and say so.

## Step 1 — the interview, before any JSON

Ask per candidate, and do not assume the answer:

1. What is it, in one line?
2. How does a script know it is up — a process, a container, a service, a flag?
3. What should a click do: start it, stop it, both, or nothing?
4. **Would anybody act on this light?** If the answer is no, do not register it. A LED
   nobody clicks is noise in a 460 px window.

Then the rule that saves a bad install: **do not register what the machine does not
have.**

| Do not register | Unless |
|---|---|
| `KV` (`vault.seal`) | something answers `/v1/sys/health` on `baoAddr` and a token lists the mount |
| `QR`, `GOV` (`qring.*`) | the Q-Ring pack exists on this machine |
| `SYNC`, `AUTO` (`skills.*`) | the skills hub and its script exist |
| `GW` (`gha.runner`) | there is a local runner to supervise |

**Docker is not Docker Desktop.** `docker.desktop` is a built-in for one specific
installer. If the machine runs Podman, nerdctl, a plain engine or a remote daemon, the
LED is a `cmd` probe over that engine (see the recipes), not that id. Never tell a human
to install Docker Desktop just to light a LED.

MCP is the same deal: a server does not deserve a light because it exists. Ask what it
is for and what a click should do. Zero MCP rows is a valid install.

## Step 2 — pick the source

| You want to watch | Source | What it costs |
|---|---|---|
| A built-in (vault, qring, gha, skills) | a `catalog` probe | nothing |
| One `mcpServers[]` row | a derived `mcp.<id>` probe | one MCP row |
| Anything else — own process, service, container, tunnel, policy flag | `services[]` | one JSON object |
| A new *kind* of probe, shared with other people | a provider manifest | a manifest and its trust review |

## Step 3 — describe the service (`services[]`)

Four answers become one object. The comments are for reading, not for the file.

```jsonc
{
  "id": "my-api",                     // stable key, no spaces, unique
  "label": "API",                     // what the LED says: 1-4 chars, uppercased
  "name": "my-api on 8788",        // the tooltip's first line
  "probe": {
    "kind": "process",                // "process" | "cmd"
    "image": "my-api.exe",            // process: the image to look for
    "match": "my-api"                 // optional: argv substring, to tell interpreters apart
  },
  "actions": {
    "on":  { "cmd": ["C:\\tools\\my-api.exe", "serve"] },
    "off": { "cmd": ["C:\\tools\\my-api.exe", "stop"] }
  }
}
```

`kind: "process"` runs nothing. It matches an image (and, when `match` is set, a command
line) through the OS process list. It is the cheap kind: no shell, no output, no trust
surface. Use it when the service is a real process with a stable image.

`kind: "cmd"` runs a command and reads the exit code:

| Exit | State | Use for |
|---|---|---|
| `0` | on | the container is running, the service is up, the flag is set |
| `2` | off | not running, not set — a click is all it needs |
| `3` | warn (amber) | installed but broken: missing binary, dead credential |
| anything else | warn | the probe itself failed; the detail says why |

The first line of stdout becomes the tooltip detail. Output is byte-capped and
redacted: a probe that prints something that looks like a secret is cut off, and the
value is treated as compromised.

Use `cmd` for anything without its own process: a container, a Windows service, a
tunnel, a policy flag. Omit `actions.off` and the LED becomes read-only — it reports,
a click does nothing.

**No `http` probe in v1.** A LED cannot declare a host, a method and a body. If a
service only answers on a URL, run a health check that already exists on the machine
(`curl.exe` in a `cmd`), and keep it loopback.

`origin` is reported, not configured: `managed` (edgePass spawned it, pid known),
`external` (alive but not ours) or `down`. It is what protects the click: an external
process is never killed by a generic image such as `node.exe` or `python.exe`.

## Step 4 — the LED row

The row is what the header shows; the service is what it watches. They are separate on
purpose: two LEDs can watch the same service with different labels.

```jsonc
{
  "id": "api",                 // unique; what a click sends back
  "label": "API",              // 1-4 chars, uppercased
  "probe": "svc.my-api",       // a catalog probe, or svc.<id> / mcp.<id>
  "on":  "svc.my-api.on",      // action when the probe says off
  "off": "svc.my-api.off",     // action when the probe says on
  "alt": null                  // optional: Ctrl / right click
}
```

Rules the backend enforces on every write:

| Rule | Why |
|---|---|
| at most **8** LEDs | the header is one row in a narrow window |
| label 1-4 chars, uppercased | eight 4-char labels still fit; a word does not |
| `probe` / `on` / `off` / `alt` must exist in the catalog | a spec that names nothing cannot light up |
| ids unique | a click addresses one LED |
| an empty list is valid | "no LEDs" is a real choice |

## Step 5 — write it

The pipe is the safe path: it validates, saves, and the widget picks the change up in
its next snapshot (5 s). It replaces the **whole** list, so read first, append, write
back.

```powershell
$r = Invoke-EdgePass '{"op":"toggles"}' | ConvertFrom-Json
if (-not $r.ok) { throw $r.error }

# Keep exactly the four/five fields the spec has; state and docs are not writable.
$specs = @($r.toggles | ForEach-Object {
  [pscustomobject]@{ id=$_.id; label=$_.label; probe=$_.probe; on=$_.on; off=$_.off; alt=$_.alt }
})

if ($specs.Count -ge $r.catalog.maxToggles) {
  throw "no room: $($specs.Count)/$($r.max) LEDs. Remove one first, or ask which to drop."
}

$specs += [pscustomobject]@{
  id="api"; label="API"; probe="svc.my-api"; on="svc.my-api.on"; off="svc.my-api.off"; alt=$null
}

$json = @{ op="toggles"; set=$specs } | ConvertTo-Json -Depth 8 -Compress
Invoke-EdgePass $json
```

`services[]` has its own pipe operation, the same read-append-write shape:

```powershell
# 1. The service: what to watch, and what a click may do.
$r = Invoke-EdgePass '{"op":"services"}' | ConvertFrom-Json
if (-not $r.ok) { throw $r.error }

$list = @($r.services) + [pscustomobject]@{
  id      = "my-api"
  label   = "API"
  name    = "my-api on 8788"
  probe   = @{ kind = "process"; image = "my-api.exe" }
  actions = @{
    on  = @{ cmd = @("C:\tools\my-api.exe", "serve") }
    off = @{ cmd = @("C:\tools\my-api.exe", "stop") }
  }
}
Invoke-EdgePass (@{ op = "services"; set = $list } | ConvertTo-Json -Depth 8 -Compress)

# 2. The LED that watches it — Step 5, with probe "svc.my-api".
```

The reply to `{"op":"services"}` carries `services`, the `svc.*` half of the
catalog (`probes`/`actions`) and the `leds` already pointing at those services,
so you can see what a change would break before writing it.

Traps, both real:

- **`ConvertTo-Json` defaults to depth 2** in Windows PowerShell 5.1. A nested
  `services[]` object is silently flattened. Always `-Depth 8`.
- **Do not register while Settings is open.** The panel holds a draft of the whole list
  and its *Guardar* writes that draft back, undoing your change.

Removing a service does **not** remove its LED: the LED turns amber with "probe
desconocida" until you take the row out of `toggles[]`. Remove both, in that
order (`toggles` first if you are replacing the service).

The file is still the other door: `%APPDATA%\edgePass\config.json`, re-read on
every poll. Read, merge, keep every key you did not touch — unknown keys are
preserved on purpose so a downgrade does not delete someone's config.

## Step 6 — verify, then hand over

1. `{"op":"toggles"}` returns the new row with `state.on`, `state.warn`,
   `state.live` and a `state.detail` that reads like a sentence.
2. Fire it once: `{"op":"toggle","id":"api","dir":"flip"}`. The reply names the
   action that actually ran. Flip it back.
3. Open the widget (or tell the human it is there) and read the tooltip out loud: it
   is the probe's detail plus what the click will do.

| State | Meaning | What to do |
|---|---|---|
| green | probe says on | nothing |
| off, `live:true` | probe says off, click is available | the click is the fix |
| amber | installed but broken: missing binary, missing pack, dead credential | fix the cause, not the LED |
| `live:false` | the probe cannot read anything (no vault, no config, no rows) | finish the install first |

## Recipes

| Watch | Probe | Command / image |
|---|---|---|
| a process | `process` | `image`: the exe name. Add `match` when the image is `python.exe` or `node.exe` |
| a container | `cmd` | `docker ps --filter name=^/NAME$ --format {{.Names}}` → exit 0 when it prints a name |
| a container engine that is not Docker Desktop | `cmd` | the same with `podman` or `nerdctl`; never the `docker.desktop` id |
| a Windows service | `cmd` | `powershell -NoProfile -Command "if ((Get-Service NAME).Status -eq 'Running') { exit 0 } else { exit 2 }"` |
| a tunnel | `cmd` | `cloudflared tunnel info NAME`, or `process` on `cloudflared.exe` plus `match` |
| a policy flag | `cmd` | a command that exits 0 when the flag is on and 2 when it is off |
| an MCP server | derived | do not write JSON: add the row in the MCP panel, the LED is `mcp.<id>` |

`sc query` is a trap: it exits 0 even when the service is stopped. Use `Get-Service`.

## Removing a LED

Take the row out of the list with the same `toggles.set` call, then verify. If the
service is gone from the machine, remove its `services[]` entry too: an orphan LED is
an amber light nobody can fix. Keep `KV` out of a machine with no vault, and keep a
fresh install's list empty rather than shipping lights with nothing behind them.

## Failure modes

| Symptom | Cause |
|---|---|
| `{"ok":false,"error":…,"catalog":…}` | unknown probe/action id, or over the cap. The reply carries the vocabulary: fix from it |
| the LED never appears | the id is not in `catalog.probes` — this build predates it |
| amber with "binary missing" | the `command`/`image` does not exist on this machine |
| click does nothing, detail unchanged | the action ran but the probe disagrees: fix the probe, not the click |
| the list reverted | Settings was open and its draft was saved over your change |
| `ConvertTo-Json` lost fields | missing `-Depth 8` |

## What not to do

- Do not put a secret in `cmd`, `env`, a label or the JSON. Reference the vault; a
  value in a command line is a value in the process list.
- Do not kill by generic image. `node.exe` and `python.exe` belong to other people's work.
- Do not poll the network from a probe. Loopback health checks only, and only through a
  command that already exists.
- Do not add `KV`, `QR`, `GOV`, `SYNC` or `GW` "because they ship". Each one is a light
  with something specific behind it.
- Do not exceed 8 LEDs. Choose what the human will act on; the rest belongs in a panel.

## Status

Built-in `catalog` probes work in every build. `services[]` (`svc.<id>`) and the
derived `mcp.<id>` probes are the v1 extension of this contract and ship with
this build. The runtime check is still the catalog, never a version number: if
`catalog.probes` does not list the id you need, this build cannot light it.
