# Public edgePass: a binary that grows without recompiling

> A plan, not code yet. Date: 2026-10-18. Status: proposal for decision.
>
> **Update (2026-11):** point 2 below is no longer debt. `config.services[]`
> publishes `svc.<id>` in the catalog, so a new LED is data, not Rust; the
> declarative "provider" is still deliberately unbuilt (§4). See
> `docs/TOGGLE-CONTRACT.md` §2 and `skills/edgepass-led`.

## 0. Premise

Any developer clones the repo, runs an `.exe` (or an installer), writes **one
configuration file**, and gets the widget working with what they have: OpenBao
installed (a prerequisite for the secret features), their own list of MCP
servers, their runners, and the **providers** they want to plug in — including
ours (Q-Ring), which is closed and ships separately.

Two things have to be true for that to work:

1. **No code knows this machine.** Today some of our paths and names are baked
   in as `DEFAULT_*` (see §1).
2. **A new LED is not a Rust change.** Today it is: `PROBES` / `ACTIONS` in
   `src-tauri/src/toggles.rs` plus an arm in `probe_state` / `run_action`.
   `docs/TOGGLE-CONTRACT.md` already records that the declarative provider was
   *deliberately* left unbuilt because of the trust boundary (§3 of this doc
   resolves it).

## 1. What is already right (and should not be touched)

- **The repo has no secrets.** `git ls-files` returns no `.exe`, `.log`, `.db`,
  `.pem`, `.key`, init file or `config.json`. The largest file is `src/App.tsx`
  (160 KB). Secrets live in OpenBao, and the gate secret is read **when the
  child starts** (`secretEnv`), never written to config.
- **There is already precedent for "everything is a config row"**: `mcpServers[]`
  and `ghaRunners[]` are data, with their own panel and shared validation. LEDs
  are the only vocabulary still expressed as code.
- **Config is already versioned** (`configVersion`, today 4) and validated
  (`toggles::validate`, `mcp::validate`).

## 2. What currently ties the binary to this machine

| Where | What | Where it goes |
|---|---|---|
| `qring.rs` | `DEFAULT_PACK = C:\path\to\q-ring-pack` | provider manifest |
| `skills.rs` | `DEFAULT_SCRIPT = C:\path\to\skills_sync.py` | provider manifest |
| `apps.rs` | `\\.\pipe\daemoncurier` + `DEFAULT_EXE` | provider manifest |
| `gha.rs` | `C:\Program Files\Docker\Docker\Docker Desktop.exe`, `docker\gha-local` | config + detection, not a constant |
| `config.rs` | `initPath` defaulting to `C:\path\to\secrets\...` | `edgepass init` asks for it |
| `toggles.rs` | LED vocabulary in a Rust table | manifests (§3) |
| — | there is no CLI: `main()` only opens the window | `edgepass init\|doctor\|validate\|provider` (§5) |

## 3. The core piece: **provider manifest v1**

A provider is a folder with a manifest and the commands it knows how to run.
The binary **discovers** it, validates it, and publishes its probes/actions in
the same `catalog` that already travels through `get_status` and
`{"op":"toggles"}`. Neither the UI nor the pipe knows whether an LED is built in
or third-party.

```yaml
# providers/qring/provider.yaml
manifest: 1
id: qring
label: Q-Ring
homepage: https://…            # informational
description: Q-Ring gateway (closed pack, installed separately)

probes:
  - id: qring.process           # mandatory namespace: <provider>.<thing>
    doc: "Q-Ring gateway: process alive or stopped."
    cmd: ["{dir}/venv/Scripts/python.exe", "{dir}/qring.py", "status", "--json"]
    timeout_ms: 3000
    # Exit contract: rc 0 = on, rc 2 = off, rc 3 = warn/broken.
    exits: { 0: on, 2: off, 3: warn }
    parse: json                 # stdout: {"detail": "…", "live": true}
    poll: slow                  # slow = only asked for when some LED uses it

actions:
  - id: qring.on
    doc: "Start the Q-Ring gateway (does not touch policy)."
    needs_vault: true           # the binary opens the vault; the provider never sees the token
    cmd: ["{dir}/venv/Scripts/python.exe", "{dir}/qring.py", "enable"]
    toast: "Q-Ring on"
  - id: qring.off
    cmd: ["{dir}/venv/Scripts/python.exe", "{dir}/qring.py", "disable"]
  - id: qring.govern.on
    needs_vault: true
    cmd: ["{dir}/venv/Scripts/python.exe", "{dir}/qring.py", "govern", "on"]

defaults:                      # what `edgepass provider add` proposes as LEDs
  toggles:
    - { id: qr, label: QR, probe: qring.process, on: qring.on, off: qring.off, alt: qring.restart }
    - { id: gov, label: GOV, probe: qring.governance, on: qring.govern.on, off: qring.govern.off }
```

Contract rules (set by the binary, not the manifest):

- `{dir}` is replaced by the provider folder; `..` and absolute paths are
  forbidden in `cmd` (they resolve against `{dir}` or `PATH`).
- **No network in v1.** There is no `url`/`method`/`body` field. An LED cannot
  talk to anyone; if that is ever needed, it opens with a destination allow-list
  and review, as `TOGGLE-CONTRACT.md` already warns.
- **Secrets by reference only**: `secretEnv: [{env: FOO, vault: {path, field}}]`.
  The binary is the only one that reads OpenBao and injects the value into the
  child's environment; the value never travels to the manifest, the UI or the
  log (a rule that already applies).
- Output is byte-capped and redacted: if a command prints something that looks
  like a secret, it is cut and reported.
- **Explicit trust**: installing a provider is an act (`edgepass provider add
  <dir>`), the manifest hash is stored in config, and the binary warns when it
  changes. A provider never self-installs from the network.

This is exactly what you asked for with QR: **the pack brings its own
`provider.yaml`**; the user points at the folder (or runs `edgepass provider add
C:\...\pack`) and the `QR`/`GOV` LEDs become available just like the MCP ones.
The binary does not know Q-Ring exists.

## 4. Configuration: one file, versioned, with a schema

`%APPDATA%\edgePass\config.yaml` (JSON is still accepted: detected by the first
brace and migrated). Sections:

```yaml
version: 1

vault:                      # optional: without it, only secret features are off
  addr: http://127.0.0.1:8200
  mount: kv
  namespaces: [""]
  initFile: ~/.config/edgepass/openbao-init.json
  autoUnseal: true

ui:                         # what lives in Settings today
  language: auto            # auto = follow the system locale, English fallback
  dock: left
  autoCollapseIdleSecs: 10
  maxToggles: 8
  toggles: []               # the LEDs, with catalog ids (built-in or provider)

providers:                  # where the vocabularies come from
  - path: providers/qring
    hash: sha256:…          # pinned on install
  - path: providers/gha
  - path: providers/skills

mcp:                        # what is mcpServers[] today
  - id: mi-api
    command: …
    publicUrl: https://…
    tunnel: edgepass-mcp
    secretEnv: [{ key: MY_API_GATE_SECRET, path: personal/mi-api, field: GATE_SECRET }]
    autostart: true

runners: []                 # what is ghaRunners[] today
```

- `schema/edgepass.schema.json` is published: VS Code autocompletion and
  validation in `edgepass validate` (same code, one place).
- `configVersion` stays: idempotent migrations and an `edgepass migrate`.
- Unknown keys are **kept** (never overwritten) and reported, so a downgrade
  does not delete a new provider's config.

## 5. CLI (what turns "clone" into "use")

```
edgepass                       # the window (what the .exe does today)
edgepass init [--dir .]        # writes a minimal config.yaml + picks the vault
edgepass doctor                # today's verify-mcp.ps1, generalised and in Rust
edgepass validate [config]     # schema + ids + providers
edgepass provider add|list|remove|verify
edgepass run <toggle-id>       # the same as {"op":"toggle"}, for scripts/CI
edgepass migrate
```

`doctor` is the piece that makes "you only need the config" true: it walks the
chain (vault binary, service, seal status, v1/v2 mount, token, MCP server port,
process, tunnel, 401 endpoint) and says **which link is missing and which
command fixes it**, without opening the UI.

## 6. Distribution

- Self-contained `.exe` (already) + **signed** MSI/NSIS; `winget` and `scoop`
  manifests in the repo; Releases with `SHA256SUMS`.
- The app **does not self-update** in v1: a widget with vault access does not
  swap binaries without a signature. `edgepass update --check` reports.
- `SECURITY.md` with the threat model (loopback only, no values over the pipe,
  hashed providers) and how to report.
- License: MIT (or Apache-2.0) for the binary; the Q-Ring pack is proprietary and
  installed separately — the README says so on the first line so nobody is
  confused.

## 7. What is never opened

The trust core stays in Rust, with tests, and providers **ask for it** but do not
implement it: vault open/close (`net::assert_local` + init-file keys), the
committee's Allow/`request`, secret reads, process supervision (own pid first,
image only when distinctive) and `secretEnv` injection. A provider may *decide*
when an LED is on; it cannot read a secret or speak over the pipe.

## 8. Phases

| Phase | What | Why in this order |
|---|---|---|
| **F0** (~1 day) | LICENSE, SECURITY, CONTRIBUTING; move `DEFAULT_*` to config/env; tidy `docs/`; README for "install OpenBao, run this" | without it the repo is not publishable, and it is cheap |
| **F1** | `config.yaml` + `schema/edgepass.schema.json` + `edgepass init/validate/doctor` + migration from `config.json` | it is the literal premise: "all I need is the config" |
| **F2** | provider contract v1 + loader + `provider add/verify` + **Q-Ring as the first provider** | the proof: if QR lands without touching Rust, the design holds |
| **F3** | port `gha`, `skills`, `apps` to providers; `vault` and `docker` stay built in (they are primitives, not plugins) | closes the "two ways to make an LED" debt |
| **F4** | binary signing, winget/scoop, `edgepass update --check`, "your first LED in 10 minutes" docs | only once F1–F3 are stable |

## 9. Risks

- **A manifest is executable code.** Mitigated by hash pinning + an explicit act
  + no network + secrets by reference. Even so, `provider add` must show *which
  commands* it will run before accepting.
- **Two LED paths (built-in and provider) during F2–F3** can drift. Avoid it by
  porting the built-ins to internal manifests in F3, not later.
- **The vault as a prerequisite** kills first impressions if it is mandatory
  (see the open decision).
- **Windows-first**: the manifest is portable, but `scheduled task`, keychain and
  pipes are per platform; each needs its adapter before "any developer" includes
  macOS.
