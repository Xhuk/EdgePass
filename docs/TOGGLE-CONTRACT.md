# The header toggle contract

The LEDs in edgePass's header (`KV`, `GW`, `GOV`…) are **data, not markup**. Each
one is a row in `config.json` → `toggles[]`, rendered by the widget from what the
backend publishes. There is no per-provider code in the UI: a LED says what it
*watches* and what a click *does*, and the backend resolves both.

That is the whole point of this document: a script, a skill, an agent or a human
contributor can read the vocabulary at runtime and add LEDs without touching the
frontend — and, for the built-in probes, without guessing an id.

```
Settings → Header toggles           ─┐
config.json → "toggles": [ … ]      ─┼─ same list, three views of it
{"op":"toggles","set":[ … ]}        ─┘
```

## 1. The spec

```jsonc
{
  "id":    "kv",             // stable key. Derived from the label when empty,
                             // made unique on save. What a click sends back.
  "label": "KV",             // 1-4 chars, uppercased. What the pill says.
  "probe": "vault.seal",     // how "is it on?" is answered   (see §2)
  "on":    "vault.unseal",   // action run when the probe says off
  "off":   "vault.seal",     // action run when the probe says on
  "alt":   null              // optional third action: Ctrl / right click
}
```

Rules the backend enforces on every write (`toggles::validate`, shared by Settings
and the pipe, so both take the same path):

| Rule | Why |
|---|---|
| at most **8** LEDs (`catalog.maxToggles`) | the header is one row; the collapsed pill grows with the LEDs it has (`floatCollapsedWidth`) and never wraps |
| label 1-4 chars, uppercased | eight 4-char LEDs still fit the widened pill next to the grip and the `edgePass` label |
| `probe` / `on` / `off` / `alt` must exist in the catalog | a spec that names nothing cannot light up or do anything |
| ids unique (auto-suffixed `-2`, `-3`…) | a click addresses one LED |
| `on` / `off` may be empty | a read-only light (a service with no actions) is a valid LED |
| an empty list is valid | "no LEDs" is a real choice, not an error |

An unknown id is refused **with the catalog in the reply**, so a caller can fix
itself in one round trip instead of reading the source.

## 2. The vocabulary (`catalog`)

Published by `get_status` (webview) and by every `toggles` pipe reply. `doc` is
what the tooltip and the settings hint show — it is the only copy for these
buttons.

### Probes — how a LED reads its state

| `probe` | `on` when | amber (`warn`) when | clickable (`live`) when |
|---|---|---|---|
| `vault.seal` | vault reachable **and** unsealed | vault unreachable or uninitialised | vault reachable |
| `qring.process` | `qringd` is running | the Q-ring pack is missing | the pack is there |
| `qring.governance` | `qringGoverned` is true | never | a Q-ring status could be read |
| `docker.desktop` | `docker info` answers | never (amber says "installed but stopped") | always — the click is what starts it |
| `gha.runner` | Docker answers **and** every declared runner is running, present and not in the credential loop | never | always — the click ensures it |
| `skills.sync` | the skill library is in sync (drift 0) | drift > 0, or the syncer is missing | the `skills_sync.py` script is where the config says |
| `skills.watch` | the hub watcher is running | never | the script is there |
| `mcp.servers` | every row of `mcpServers[]` is running **and** every named tunnel a row declares is connected | a row's binary is missing, or `cloudflared` is missing | the list is not empty |
| `mcp.<id>` *(derived)* | that one MCP row is running and its tunnel (if declared) is connected | that row's binary or `cloudflared` is missing | the row exists |
| `svc.<id>` *(derived)* | the service's `process`/`cmd` probe says on | the probe itself failed (missing binary, timeout, unexpected exit) | always; a read-only service still reports `live:false` in the view |

The two derived families exist only if the config does: one `svc.<id>` per
`services[]` row, one `mcp.<id>` per `mcpServers[]` row. They are the whole
extension point — the exe stays immutable, the inventory grows. `origin`
(`managed` / `external` / `down`) rides on the view for them: only an MCP row can
answer `managed`, because only edgePass spawns one.

A probe returns four bits plus the tooltip detail: `on`, `warn`, `ask` (a gateway
Allow is pending) and `live`. Probes never leave the process; only the vault's
seal status is an HTTP round trip, and the Q-ring probe (which shells out) is
only run when some LED actually asks for it.

Only `qring.governance` ever sets `ask` today: an Allow is a *policy* question
("may this run?"), so the pulse rides the LED that owns the policy, not the one
that watches the process. A header with no `GOV` still shows it — the `Allow`
chip in the header is not a LED.

### Actions — what a click does

| `action` | needs the vault | effect |
|---|---|---|
| `vault.unseal` | no | unseal with the keys from the init file, mark activity, re-index if `autoSync` |
| `vault.seal` | token | seal the vault — **this cuts off every other client of the OpenBao service** |
| `qring.on` | yes | start the Q-ring gateway. Leave the policy alone: turning the gateway on is not the same as letting rings spend |
| `qring.off` | no | stop the Q-ring gateway **and pull the policy with it** (`qringGoverned=false`) |
| `qring.restart` | yes | restart the Q-ring gateway (policy untouched) |
| `qring.govern.on` | yes, only if it has to start | let rings run (and spend). A stopped gateway is started first, on `qring.on`'s own path (vault included) |
| `qring.govern.off` | no | block rings: nothing runs, nothing is spent. The gateway keeps running — that combination, not the process, is what the policy is for |
| `docker.up` | no | start Docker Desktop and wait for the engine (up to 120 s). Idempotent |
| `gha.up` | only if a runner declares `secretEnv` | ensure the local runner: the engine, the image (built from `ghaDir` when missing) and the container — recreated with a **fresh registration token** (`gh api`) when it is missing, stopped or looping on a dead credential |
| `gha.down` | no | `docker stop` the runner container. Docker Desktop is deliberately untouched: stopping it would take every other stack with it |
| `gha.repair` | only if a runner declares `secretEnv` | recreate the container even when it looks healthy — the `offline`-on-GitHub case |
| `daemoncurier.open` | no | focus the live DaemonCurier over `\\.\pipe\daemoncurier`, or launch `daemoncurierExe` when nobody answers |
| `skills.sync` | no | re-link every agent's `skills/` from the hub (`~/.agents/skills`). Idempotent; never copies files |
| `skills.watch.on` | no | start the hub watcher: re-link by itself when a skill appears, is renamed or is deleted |
| `skills.watch.off` | no | stop the watcher (the manual button keeps working) |
| `mcp.on` | only if a row declares `secretEnv` | start every server of `mcpServers[]` **and** the named Cloudflare tunnel each row declares (`tunnel`) — idempotent |
| `mcp.off` | no | stop the servers and their connectors. Our own children by pid; a connector started by hand by pid matched on `tunnel run <name>`, never by image |
| `mcp.restart` | only if a row declares `secretEnv` | stop then start the whole chain (the LED's `alt`) |
| `svc.<id>.on` / `.off` *(derived)* | no | run exactly the command the service declared in `actions.on` / `.off`. Never synthesised: this module does not kill by image |
| `mcp.<id>.on` / `.off` / `.restart` *(derived)* | only if **that** row declares `secretEnv` | the same three operations, scoped to one server and its tunnel |

Gestures in the header: **click = flip** (the backend picks `on`/`off` from what
the probe sees right now), **Ctrl / Cmd / Alt / Shift-click or right click =
`alt`** (when the spec has one).

**Why `GW`, `QR` and `GOV` are three LEDs and not one.** They answer different
questions: *is the local GHA runner up?* (`gha.runner` — Docker Desktop plus the
container that deploys for us), *is the Q-ring gateway up?* (`qring.process`) and *may
rings run and spend?* (`qringGoverned`). The pack asks the last one before every
expensive path (`qring/governance.py`), which is why a policy can block spend with the
process still up. Two couplings keep the Q-ring pair from contradicting itself: stopping
the gateway writes the policy off, and turning the policy on starts a stopped gateway
first. The case left deliberate is *policy off over a running gateway* — nothing runs,
nothing is spent, and `qringd` is still there to be probed. `GW` is a different axis
entirely: it fixes the runner (minting a registration token when the old one died) and
leaves Docker Desktop's other stacks alone.

## 3. Talking to it

### The pipe (local, same user)

```jsonc
// read: the LEDs and the vocabulary
{"op":"toggles"}
// → {"ok":true,
//    "toggles":[{"id":"kv","label":"KV","probe":"vault.seal","on":"vault.unseal",
//                "off":"vault.seal","state":{"on":true,"warn":false,"ask":false,
//                "live":true,"detail":"…"},
//                "docs":{"probe":"…","on":"…","off":"…"}}],
//  "catalog":{"maxToggles":8,"maxLabelChars":4,"probes":[…],"actions":[…]},
//    "max":8}

// write: replace the whole list (validated; the reply echoes the stored list)
{"op":"toggles","set":[{"id":"kv","label":"KV","probe":"vault.seal",
                        "on":"vault.unseal","off":"vault.seal"}]}

// fire one LED — the same thing a click does
{"op":"toggle","id":"kv","dir":"flip"}   // flip | on | off | alt
// → {"ok":true,"id":"kv","action":"vault.unseal","detail":"…","toast":"Vault abierto"}

// the owner's services: read (services + their svc.* catalog half + the LEDs
// already pointing at them) or replace the whole list
{"op":"services"}
{"op":"services","set":[{"id":"mi-api","label":"API","probe":{"kind":"process","image":"mi-api.exe"}}]}
```

`{"op":"status"}` also carries `toggles`, so a single call shows the vault, the
runs, the grants and every LED. Helper for humans:
`scripts\edgepass-toggles.ps1` (see below).

**The caller names a LED, never an action.** `{"op":"toggle"}` resolves the
action id from `config.json`; nothing outside the widget can fire an action that
a LED does not own.

### The webview

`get_status` → `StatusDto.toggles[]` (spec + `state` + `docs`), `StatusDto.catalog`
and `StatusDto.services[]` (the raw inventory the Servicios editor edits). A click
calls the `fire_toggle` command with `{id, dir}`, and the toast is the action's
own `toast`.

## 4. Wiring a LED to OpenBao (or another Vault-compatible KV)

The seal/unseal pair is the canonical example, and it is what ships as `KV` (OpenBao
is a prerequisite, so the vault is the one light every install has something behind):

| Spec side | OpenBao API | Notes |
|---|---|---|
| probe `vault.seal` | `GET /v1/sys/seal-status` | `sealed` / `initialized`; a 503 means sealed |
| action `vault.unseal` | `POST /v1/sys/unseal` with the unseal keys | edgePass reads them from the init file — they never travel over the pipe or into the UI |
| action `vault.seal` | `PUT /v1/sys/seal` with the token | needs a token; the token lives in the OS keychain |

Anything that speaks the same `/v1/sys` contract (OpenBao, Vault OSS/Enterprise
in a compatible mode) maps the same way: point `baoAddr` at it and the `vault.*`
probe/actions work unchanged. Two things stay non-negotiable, and they are why
this is a *built-in* registry rather than a free-form HTTP field today:

- **loopback only** — `net::assert_local` refuses a non-local address before any
  token is read or sent;
- **no values, ever** — the unseal key and the token are read inside the process
  and are never returned, logged or carried by the pipe.

### Adding a new kind of LED

Most of the time you do **not** touch Rust: the service is data
(`services[]` → `svc.<id>`), and the skill `skills/edgepass-led` walks the
interview and the JSON. The Rust path is for a new *kind* of probe or action —
one that every install could use:

A *new* probe or action is one entry in `RAW_PROBES` / `RAW_ACTIONS` in
`src-tauri/src/toggles.rs` plus its arm in `probe_state` / `run_action`:

```rust
RawProbe { id: "vault.seal", doc: "…", needs_qring: false,
           needs_gha: false, on: "vault.unseal", off: "vault.seal", alt: None },
```

and `cargo test` keeps the table and the arms in step (`every_table_entry_resolves`,
`catalog_ids_are_unique_and_documented`). Nothing else has to change: the
catalog, the settings pickers, the tooltips and the pipe contract all read from
those tables. A contributor adding `svc.health` (a read-only probe) or a
`compose.up` action gets the whole surface for free.

**Next step, deliberately not built yet:** a declarative provider
(`"probe": "http.get:…"`, `"on": "http.post:…"`) so a LED can point at *any*
Vault-compatible endpoint without a Rust change. It is not here because it
widens the trust boundary — a spec would then choose the host, the method and
the body — so it needs its own review (destination allow-list, where the token
comes from, what a failed call reports). The built-in registry is the safe half
of that contract, and the ids are already shaped like it.

## 5. Defaults

| Install | LEDs |
|---|---|
| fresh (no `config.json` yet) | `KV` only — OpenBao is a prerequisite, so the vault is the one light every install has something behind. Everything else is opt-in |
| upgrading from a pre-`toggles` config | `KV`, `QR`, `GOV`, `GW` — the three the header hardcoded, with `GW` (the Q-ring process) moved to `QR` and handed its new job. The v3→v4 migration does it once, and only for the *stock* spec: a header edited by hand stays as it is |

`GW`/`QR`/`GOV` are one `＋ Add toggle` away in Settings (probes `gha.runner` /
`qring.process` / `qring.governance`); an install with no Q-ring pack and no Docker
Desktop is not forced to carry three amber lights. `×` removes a row, `↑`/`↓` reorder, and
everything applies on **Save**.

`SYNC`/`AUTO` (probes `skills.sync` / `skills.watch`) are the same deal: opt-in rows.
They stay out of the shipped defaults on purpose — they point at
`C:\tools\skills-sync\skills_sync.py`, a path that only exists on a machine that
has that tool (override it with `EDGEPASS_SKILLS_SYNC`). One row each:

```jsonc
{"id":"sync","label":"SYNC","probe":"skills.sync","on":"skills.sync","off":"skills.sync"}
{"id":"auto","label":"AUTO","probe":"skills.watch","on":"skills.watch.on","off":"skills.watch.off"}
```

`MCP` is the same deal, and it is the one LED that covers a **chain** rather than
one process: a row of `mcpServers[]` may name a Cloudflare tunnel (`"tunnel":
"edgepass-mcp"`), and then `mcp.on` brings up the server *and* `cloudflared tunnel
run <name>`, `mcp.off` takes both down, and the LED only says "on" when the
connector is connected — a green light over an origin nobody can reach is exactly
the lie this widget exists to avoid. `cloudflared` lives in the config
(`cloudflaredExe`, or `EDGEPASS_CLOUDFLARED`, or `PATH`: an MSI install does not
add it to `PATH`):

```jsonc
{"id":"mcp","label":"MCP","probe":"mcp.servers","on":"mcp.on","off":"mcp.off","alt":"mcp.restart"}
```
