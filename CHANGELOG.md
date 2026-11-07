# Changelog

All notable changes to edgePass. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions are
`MAJOR.MINOR.PATCH` and a release is a `vX.Y.Z` tag in this repo.

## 0.5.0 — 2026-11-06

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
