# Changelog

All notable changes to edgePass. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions are
`MAJOR.MINOR.PATCH` and a release is a `vX.Y.Z` tag in this repo.

## 0.5.2 — 2026-11-08

### Changed

- **English is the identity language.** The interface and the backend copy are
  written in English and Spanish is a translation (`src/i18n.ts`), so a string
  without one renders in English instead of disappearing. Text emitted by Rust —
  LED details, status lines, errors — is translated at display, including the
  interpolated ones (`1 of 1 running · tunnel X up` → `1 de 1 corriendo · túnel X
  arriba`).
- **Brand.** New candado icons for the app and the installer, and the site uses the
  banner lockup with per-theme favicons and a 1200×630 social card.
- Docs: `OPERATIONS.md` in English (Spanish kept as `OPERATIONS.es.md`), a
  `docs/README.md` index, issue forms, and a one-way mirror to the GitHub wiki.

### Fixed

- **Secrets list:** one number per row (times used) and notes moved to the metadata
  line. The badges no longer misalign depending on which one a row had.
- **Dock:** the size verification waits for the resize to land instead of measuring
  mid-animation. Collapsing from the panel is one settle, not three corrections
  (581 → 98 → 71) with three springs.

### Security

- `localhost` is rewritten to `127.0.0.1` before any URL is built, so a poisoned
  `hosts` file cannot redirect the token.
- Vault errors forward only `errors[]`; `request_id` and `warnings` no longer reach
  the UI, the log or an agent.

## 0.5.1 — 2026-11-07

### Added

- **First-run onboarding.** On a machine without a usable vault (unreachable,
  uninitialized, no token, missing init file or sealed), the panel opens once with
  what is missing and four actions: **Install OpenBao** (official guide),
  **Retry**, **Start service** when the Windows service exists but is stopped, and
  **Connect a vault** (Settings → Vault).
- **`setup` in the status.** `get_status` and `{"op":"status"}` now carry
  `needed`, a stable `code`, an English `message`, the ordered `steps`, and the
  official `installUrl` / `releasesUrl` / `readmeUrl`. An agent can tell the human
  exactly what to do instead of guessing.
- `open_setup_link` command: opens one of the fixed setup URLs (id, not a URL).

### Changed

- README has a "No vault yet?" section that describes the same path.
- An agent that downloads this build has the contract in
  `skills/edgepass/SKILL.md`: read `setup`, tell the human, do not install or
  invent a token, one retry after they say it is running.

## 0.5.0 — 2026-11-07

### Added

- **Owner-defined services** (`services[]`): a `process` or `cmd` probe, optional
  `on`/`off` commands, and the derived `svc.<id>` probe plus `svc.<id>.on|off`
  actions. A new header LED is now data, not a Rust change.
- **Per-server MCP LEDs** (`mcp.<id>`), derived from `mcpServers[]`, alongside the
  aggregate `mcp.servers`.
- `origin` (`managed` / `external` / `down`) on the LED view: who owns the
  process behind it.
- `{"op":"services"}` to read or replace the service list over the pipe, and
  `-Services` / `-SetServices` in `scripts/edgepass-toggles.ps1`.
- Settings → **Services**: the JSON editor for that list, validated on save.
- `schema/edgepass.schema.json`: editor autocompletion and validation.
- **Passphrases get a fresh random block** (`randomChars`, 0–12): 8 alphanumeric
  characters per phrase, drawn from the OS CSPRNG, worth ~47 bits. A fresh
  install ships with 8; a stored 0.4.x style keeps its old shape.
- A per-install **salt** (`installSalt`, 128-bit hex) rotates the vocabulary
  mapping so two machines cloned from the same snapshot do not draw the same
  words. It is public and adds no entropy; it is not a pepper.
- `.github/workflows/ci.yml` and `release.yml`: test gate, tagged release, SHA-256
  checksum, and an optional Authenticode step when the certificate secrets exist.

### Changed

- The header holds up to **8** LEDs (was 6). The collapsed pill grows with the
  LEDs it actually has and shrinks back; the top dock grows the same way.
- A fresh install ships only `KV` (OpenBao is the prerequisite). An upgrade keeps
  the three hardcoded LEDs.
- `edgepass validate` prints "solo mirar" for a LED with no action.
- Documentation is in English, with English file names. The Spanish operations
  manual stays as `docs/OPERATIONS.es.md` by design.
- The index, logs and placement moved to `%LOCALAPPDATA%\edgePass` (config and
  the keychain entry stay in `%APPDATA%`), with a one-time migration.
- Start-at-logon is a native scheduled task (8 s delay), with no PowerShell
  wrapper and no `-ExecutionPolicy Bypass`.
- The webview runs under a CSP (`default-src 'self'`, IPC only in `connect-src`).

### Fixed

- 32 lines of `lib.rs` had double-encoded UTF-8 (mojibake) in user-facing
  messages, including two clippy errors for invisible characters.
- A scan of the Windows process list no longer blocks the whole status snapshot:
  `svc` probes run in parallel with a 2 s budget, `get_status` runs off the async
  workers, and the UI dedupes an in-flight poll instead of stacking another.
- Scoop: `post_uninstall` unregisters the logon task and removes the keychain
  token; `notes` cover WebView2, quitting before `scoop update`, and where user
  data lives.

## 0.4.2 — 2026-10

- MCP servers as data with their own panel, autostart and tunnel handling.
- Header LEDs as data (`toggles[]`) with a published catalog.
- Local GHA runner LED, skills sync/watcher LEDs.
- Portable release + Scoop/winget manifests.
