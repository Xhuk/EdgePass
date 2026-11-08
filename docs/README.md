# edgePass docs

Everything here is versioned with the code, so a change to behaviour and its
documentation land in the same review. The GitHub **wiki**, when enabled, is a
generated mirror of these files — edit here, never there.

## Start here

| Document | Read it when |
|---|---|
| [PREREQUISITES.md](PREREQUISITES.md) | Before the first install: OpenBao, WebView2, what the vault needs. |
| [OPERATIONS.es.md](OPERATIONS.es.md) | Day-to-day use, in Spanish: secrets, LEDs, the agent pipe. |
| [SECURITY.md](SECURITY.md) | You want the threat model and how to report a problem. |
| [BACKUP.md](BACKUP.md) | Before storing the first real secret: snapshots, init file, restore drill. |

## How it is built

| Document | What it records |
|---|---|
| [STACK.md](STACK.md) | Why Tauri 2, why Kiri, why OpenBao — and what to watch out for. |
| [TOGGLE-CONTRACT.md](TOGGLE-CONTRACT.md) | The header LEDs as data: probes, actions, the JSON the backend enforces. |
| [PUBLIC-PLAN.md](PUBLIC-PLAN.md) | The design behind the public release: provider manifests, config, CLI. |

## Ships and packaging

| Document | What it covers |
|---|---|
| [DISTRIBUTION.md](DISTRIBUTION.md) | Releases, Scoop and winget, the checklist for cutting a version. |
| [WINGET-SUBMISSION.md](WINGET-SUBMISSION.md) | The first winget submission, step by step. |

## Skills

The agent instructions live in [`../skills/`](../skills): `edgepass` (use the
widget), `edgepass-led` (register a LED), `openbao-bootstrap` (from zero to a
vault). Install them into your agent's hub with:

```powershell
powershell -File scripts\install-skills.ps1
```

## The landing page

[`index.html`](index.html) is the site served at
<https://xhuk.github.io/EdgePass/>. Its brand assets are in
[`assets/brand/`](assets/brand).
