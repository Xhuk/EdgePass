---
name: edgepass
description: Use edgePass, the local OpenBao secret widget, from an agent. Trigger on any request to use edgePass in any language or spelling — "usa edge pass", "use edge pass", "use edgepass", "utilise edge pass", "verwende edge pass", "edge pass を使って", "使用 edge pass" — and on "dame la contraseña de", "do we have credentials for", "corré esto con el secreto", "run this with the secret", "está abierto el vault", "is the vault open". Ask the human for a credential in the widget, search secrets by name, run a command with secrets in the child environment, request permission with a popup, or read vault health.
---

# edgePass — how an agent uses it

edgePass is a desktop widget that talks to a local OpenBao. The agent talks to the
**widget**, not to OpenBao. The widget does the work and sees the values. The agent
never does.

## When this skill applies

The phrase means **"go to the widget"**. There is no fixed spelling: any equivalent of
"use edgePass" in any language counts, and so does a bare phrase with no object.

| The human says | Do this |
|---|---|
| "usa edge pass" / "use edge pass", no object | `{"op":"status"}`; if ready, `meter` for the secret they last named, or ask which one |
| "usa edge pass" + a name, site or path | `{"op":"meter","query":"<name>"}` and tell them where to look |
| "use edge pass and run X" | `run` with the secret by environment, never on the command line |
| "usa edge pass" and the pipe does not answer | tell them to open the widget; retry once, do not launch it yourself |

Translations, typos and variants are all the same authorization: `usaedgepass`,
`edge pass por favor`, `usá edgepass`, `usar el widget`, `use the pass`. Do not ask a
second time for the exact wording.

## Rule zero

1. **Never print a secret value.** Not to check the length, not to "verify". If a
   value reaches the output, report it as an incident and rotate it.
2. **Never ask the human to paste a secret into the chat.** The chat is the worst
   place. Use the widget.
3. **When the human says "usa edge pass", "use edge pass" or the same in any
   language, they authorized you.** Do not ask again, and do not require the exact
   phrase.
4. **`export-env` does not exist.** It wrote values to disk and was removed. To use
   a secret, run it in a child process.
5. **Do not invent paths.** Search first. The index knows the paths.

## The contract

A named pipe, one JSON object per line, one JSON line back.

- Windows: `\\.\pipe\edgepass`
- Operations: `ping`, `status`, `search`, `sync_index`, `meter`, `ingest-form`,
  `run`, `run-status`, `run-wait`, `run-cancel`, `request`, `request-wait`,
  `unseal`, `seal`, `hold`, `toggles`, `toggle`, `services`, `notes`, `note`.

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

Invoke-EdgePass '{"op":"ping"}'      # {"ok":true,"pong":true}
Invoke-EdgePass '{"op":"status"}'    # sealed, reachable, runs, grants
```

## Which operation for which request

| The human asks | Operation | Who sees the value |
|---|---|---|
| "give me the password for X" | `meter` | the human, in the widget |
| "do we have credentials for X?" | `search` | nobody, names only |
| "run this with the secret" | `run` | the child process, by environment |
| "ask permission and run it" | `request` + `request-wait` | the child, after approval |
| "load these variables" | `ingest-form` | the human pastes them |
| "is the vault open?" | `status` / `unseal` / `seal` | nobody |

## `meter` — the path for "usa edge pass"

```
{"op":"meter","query":"betty"}
{"op":"meter","namespace":"project","path":"projects/app/users","title":"Users"}
```

The reply carries `{ok, id, namespace, path}`: **names only**. The widget opens a
form, reads the path with its own token, and fills the rows. Values mask after a few
seconds. Tell the human where to look, never what the value is.

If the reply says `ambiguo`, the query matched several paths. Show the candidates and
retry with an explicit `path`.

## `run` — use a secret without seeing it

```
{"op":"run",
 "entries":[{"namespace":"project","path":"projects/app/env"}],
 "keys":["DATABASE_URL"],
 "cmd":["npm","run","migrate"],
 "cwd":"C:\\work\\app",
 "timeoutSecs":600}
```

- The reply is `{ok, code, env[], stdout, stderr}`. `env` lists **names**.
- `entries` accepts several paths. `keys` narrows what is injected. `envMap`
  renames a variable. `timeoutSecs` defaults to 300, maximum 3600.
- Arguments never carry the value. If a tool needs a value in a command line, that
  tool is wrong for this job.
- For long runs use `detach:true` (maximum four) and follow with `run-status`.

## The header LEDs are data

`{"op":"toggles"}` reads the LEDs and the vocabulary (`catalog`); `{"op":"toggle"}`
fires one. A service the binary does not know about is `services[]` data:
`{"op":"services","set":[…]}` publishes `svc.<id>`, and a `toggles` row can watch
it. The full interview and the JSON shapes are in `skills/edgepass-led`.

## First run without OpenBao

`{"op":"status"}` carries `setup`. Read it before trying anything else:

```json
{"ok":true, "...": "...",
 "setup":{
   "needed":true,
   "code":"vault_unreachable",
   "message":"OpenBao is not reachable at http://127.0.0.1:8200.",
   "steps":["Install OpenBao 2.x ...", "..."],
   "installUrl":"https://openbao.org/docs/install/",
   "releasesUrl":"https://github.com/openbao/openbao/releases",
   "readmeUrl":"https://github.com/Xhuk/EdgePass#readme"
 }}
```

Codes: `vault_unreachable`, `vault_uninitialized`, `init_file_missing`,
`no_token`, `vault_sealed`, `ready`.

When `needed` is true:

1. Tell the human what is missing in one sentence (`message`), not a wall of text.
2. Point at `installUrl` to install OpenBao and at **Settings → Vault** for the
   address and token. The widget opens the same card by itself on first run.
3. Do **not** install OpenBao, run `bao operator init`, start a service that is
   not there, or invent a token. Do not retry in a loop: one retry after the human
   says it is running.
4. Never ask for the token in the chat. It goes in the widget, or `initPath` points
   at the init file.

`needed:false` means the vault is reachable and has a token; continue normally.

## If the widget does not answer

`Connect` failing means it is not running. Do not launch it with `Start-Process`:
the shell job kills it. Use a scheduled task or a process created outside the job
object, then verify in a later call.

## Failure modes

| Symptom | Meaning |
|---|---|
| `{"ok":false,"error":"json invalido"}` | more than one line, or broken quotes |
| `meter` → `ambiguo` | several paths match; retry with an explicit path |
| `meter` → `sin coincidencias` | stale index: run `sync_index`, then retry |
| `run` → `{"ok":false,"sealed":true}` | the vault is sealed; unseal, or ask the human |
| `run` → `run slots full` | four detached runs are alive; cancel one |
| `request` stays `pending` | the human did not see the popup; do not invent a result |

## Boundaries

- The vault must be loopback. A remote vault is by design out of scope.
- The pipe is reachable from any process of the same user. That is the trust
  boundary, and the reason the human is always in the loop for values.
- Agents get names, child environments and approval prompts. Not values.
