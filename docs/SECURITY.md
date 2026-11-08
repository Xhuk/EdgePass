# edgePass security

Status: review done on 6 November 2026, re-checked for 0.5.0. This document says
what the app protects, what was fixed, and what is still open.

## Trust model

edgePass does not listen on the network. There is no port, no server, no remote API.

There are two surfaces:

1. **The local pipe** (`\\.\pipe\edgepass`). Any process running as your user can
   open it and send JSON.
2. **The vault**. The app only talks to OpenBao on loopback (`127.0.0.1`, `::1`,
   `localhost`).

The security boundary is therefore **"any process of the same user"**, not a
remote attacker. A process running under your account can already read the config
file and your files. The app's goal is smaller and more useful: keep the vault
token on the machine, and make the pipe no shorter a path than the operating
system itself.

## Fixed in 0.3.1

### High: escaping the declared path

`net::join_url` concatenated the relative path without looking at what it
contained. The URL parser normalises `..`, so the path escaped the one the client
declared.

The test that showed the flaw, before the fix:

```
{"op":"run","entries":[{"namespace":"demo","path":"surfaces/demo-runtime/../saas-runtime"}],"keys":["ADMIN_KEY"]}
→ {"code":0,"env":["ADMIN_KEY"],"ok":true}
```

The client asked for one path and read another. `meter` with `../sys/health`
answered `ok:true`.

`join_url` now validates every segment before building it. It rejects `.`, `..`,
`/`, `\`, `?`, `#` and control characters. The check runs on the decoded form, so
`%2e%2e` and `%2f` fail like `..` and `/`. A normal key name passes unchanged.

The same path is used for writing and deleting, so the fix covers all three.

### Medium: the token could follow a redirect

The HTTP client followed redirects. `X-Vault-Token` is a custom header, and a
custom header is not dropped when the host changes. Something answering on
`127.0.0.1:8200` could redirect the token to an external site. The client now uses
`redirect(Policy::none())`.

## Fixed in 0.5.0

- **CSP.** `tauri.conf.json` used `"csp": null`; the webview that handles secrets
  had no policy. It now sets `default-src 'self'`, `script-src 'self'`, and only
  the Tauri IPC origin in `connect-src` (with a separate `devCsp` for Vite).
- **Scoop lifecycle.** `post_uninstall` in the manifest unregisters the logon task
  and deletes the keychain entry, so an uninstall no longer leaves a task pointing
  at a deleted binary or a vault token behind.
- **Authenticode.** The portable exe is signed with the project certificate
  (`CN=Firma Instalador`, self-signed) and timestamped. That detects tampering but
  does not remove SmartScreen warnings on machines that do not trust the root; a
  public CA certificate is the remaining step for that.

## Fixed after the 0.5.1 review

| Severity | Finding | Where | How |
|---|---|---|---|
| Low | Errors forwarded the vault's JSON envelope, including `request_id` and `warnings` | `openbao.rs` | Only `errors[]` is forwarded; any other JSON body becomes `unexpected error body`. The status code still says what happened. |
| Low | `localhost` was accepted by name, so a poisoned `hosts` file could redirect the token | `net.rs` | `canonical_loopback` rewrites `localhost` to `127.0.0.1` before any URL is built; the name never reaches the resolver. A non-loopback host is refused. |

## What is still open (accepted)

| Severity | Finding | Where |
|---|---|---|
| Medium | `{"op":"seal"}` asks no human for confirmation | `ipc.rs` |
| Info | `cmd /C` over `credential_command`; no pipe op writes configuration | `mcp.rs` |

The `seal` finding is availability, not confidentiality. A process of the same
user can seal the vault and cut off every consumer. There is no privileged account
to prevent it, and a confirmation dialog would not stop that process — it can kill
the `.exe`. It is accepted as a design decision: **what the dialog would protect
against is a mistake, not an attacker**, so the rule lives where mistakes happen —
the `edgepass` skill says sealing is a human action, and the KV LED is the button.

`credential_command` runs through `cmd /C` because the owner may need a compound
command. Config is executable either way, and **no pipe operation writes
configuration**, so an agent cannot inject into it; it stays informational.


## What is right, and was measured

- `run` passes the command through argv, with no shell. JSON cannot inject commands.
- The pipe survives broken JSON, an unknown op, and a 20 KB query.
- `npm audit` reports zero vulnerabilities.
- The frontend does not use `dangerouslySetInnerHTML`.
- Loopback is enforced before the client is built, and the proxy is disabled.
- The token lives in the OS keychain, not on disk.
- `run` replies with variable names, never values.
- A service probe's `cmd` runs only when a `svc.<id>` LED exists, and its stdout
  is capped and dropped when it looks like a credential.
- Passphrases: every word, digit, symbol and random-block character comes from
  the OS CSPRNG, one fresh draw each. The per-install salt is public and only
  rotates the mapping between machines; it adds no entropy and is not a pepper.

## If you find an exposed secret

Tell the repository owner. The value is not copied into the chat, the issue or the
report. It is rotated, and the rotation is the containment.

## Reporting a problem

- **A bug or a question** → [open an issue](https://github.com/Xhuk/EdgePass/issues/new/choose).
  The forms ask for the version and the *Copy report* output, which redacts the token.
- **A vulnerability** → do **not** open a public issue. Use
  [private vulnerability reporting](https://github.com/Xhuk/EdgePass/security/advisories/new).
  If it is unavailable, contact the repository owner directly.
- Never paste a secret value, a vault token or an init file into an issue. A public
  issue is permanent; a rotation is not.

