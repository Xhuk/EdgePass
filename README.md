# edgePass

A desktop widget that administers a local **OpenBao** vault and hands secrets to
processes without ever printing them.

edgePass is a **console**, not a server. It does not serve secrets to your containers.
OpenBao does that. edgePass lets a human or an agent read, write, copy and inject
secrets from a vault that runs on the same machine.

> **Read [DISCLAIMER.md](DISCLAIMER.md) first.** This software manages credentials.
> A mistake can expose or destroy them.

## What it does

- Lists, searches and copies secrets from a local OpenBao (KV v1 and v2).
- Injects a secret into a child process **by environment**, never by argument.
- Opens the vault when needed and seals it on demand.
- Finds secrets by name with complementary tokens (`infisi cb2` → Infisical + path cb2).
- Ships a named-pipe contract so local agents can ask for a secret without seeing it.
- Watches the local GitHub Actions runner and repairs it when it stops.

## What it does not do

- It does not replace OpenBao. OpenBao is a **prerequisite**.
- It does not run a public endpoint and does not keep a copy of the vault.
- It does not talk to a remote vault. Every request must go to loopback.
- It does not print values, log values, or write values to disk.

If you need runtime secrets for containers, use OpenBao directly, with least-privilege
policies and short-lived credentials. See [docs/PREREQUISITES.md](docs/PREREQUISITES.md).

## Requirements

| Component | Minimum | Notes |
|---|---|---|
| Windows | 10 1809+ | The widget is Tauri 2. macOS support is planned. |
| WebView2 | Evergreen | Ships with Windows 11. Windows 10 may need the runtime. |
| OpenBao | 2.x | Local, listening on `127.0.0.1:8200`. |
| Docker | Optional | Needed only for the CI runner LEDs. |

OpenBao is not optional. edgePass is useless without a vault to talk to.

## No vault yet?

If you install edgePass before OpenBao, it still starts: the pill shows one amber
`KV` LED and, on the first run, the panel opens once with a card that says what is
missing. From there:

- **Install OpenBao** opens the official install guide.
- **Retry** re-checks the vault after you start it.
- **Connect a vault** opens Settings → Vault for the address and the token.

edgePass does not install OpenBao, does not start a service that is not there, and
does not invent a token. If the Windows service `OpenBao` exists but is stopped, it
offers to start it. An agent sees the same answer in `{"op":"status"}` → `setup`.

## Install

1. Install and start OpenBao. Follow [docs/PREREQUISITES.md](docs/PREREQUISITES.md).
2. Initialize and unseal the vault. Keep the init file **off** every repository.
3. Run the `edgePass_x.y.z_x64-setup.exe` release asset.
4. Open edgePass. The pill docks to a screen edge.
5. Open **Config**, set the vault address, and save.
6. Load the vault token. edgePass stores it in the OS keychain.

The installer is per-user. It does not need administrator rights.

## Give the skill to your agent

The installer ships the app, not the agent instructions. Copy them into your
agent's skills folder, or point the agent at `skills/edgepass/SKILL.md`:

```powershell
powershell -File scripts\install-skills.ps1
# another folder: -Dest "$HOME\.claude\skills" -Force
```

Then say **“usa edge pass”**. The skill carries the rule that matters: the agent
never sees a secret value. It asks the widget (`meter`), searches names (`search`)
or injects the value into a child process (`run`). The backend enforces the same
rule: no pipe operation returns a value.

## First run

With no vault configured, the panel opens once with the setup card described
above; after that it starts collapsed. A click on the shield opens the panel.

Agents drive the app through a named pipe, one JSON object per line. The full
contract lives in [skills/edgepass/SKILL.md](skills/edgepass/SKILL.md). That file is
plain Markdown on purpose: any agent, with any harness, can read it.

The Spanish operations manual is [docs/OPERATIONS.es.md](docs/OPERATIONS.es.md).

## Backup

The vault data and the snapshots are yours. Read [docs/BACKUP.md](docs/BACKUP.md)
before you store a single real secret. A snapshot contains every secret in clear.

## Stack

This project was developed against **Kiri** as the local container engine and
**OpenBao** as the vault. Why those two, and what to watch out for, is in
[docs/STACK.md](docs/STACK.md).

## Status

`0.5.1`. Early. Not audited. Interfaces can change between minor versions.

## License

MIT. See [LICENSE](LICENSE).
