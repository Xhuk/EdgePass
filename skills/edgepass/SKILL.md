---
name: edgepass
description: Use edgePass, the local OpenBao secret widget, from an agent. Ask the human for a credential in the widget, search secrets by name, run a command with secrets in the child environment, request permission with a popup, or read vault health. Trigger on "usa edge pass", "dame la contraseña de", "corré esto con el secreto", "está abierto el vault".
---

# edgePass — how an agent uses it

edgePass is a desktop widget that talks to a local OpenBao. The agent talks to the
**widget**, not to OpenBao. The widget does the work and sees the values. The agent
never does.

## Rule zero

1. **Never print a secret value.** Not to check the length, not to "verify". If a
   value reaches the output, report it as an incident and rotate it.
2. **Never ask the human to paste a secret into the chat.** The chat is the worst
   place. Use the widget.
3. **When the human says "usa edge pass",** they authorized you. Do not ask again.
4. **`export-env` does not exist.** It wrote values to disk and was removed. To use
   a secret, run it in a child process.
5. **Do not invent paths.** Search first. The index knows the paths.

## The contract

A named pipe, one JSON object per line, one JSON line back.

- Windows: `\\.\pipe\edgepass`
- Operations: `ping`, `status`, `search`, `sync_index`, `meter`, `ingest-form`,
  `run`, `run-status`, `run-wait`, `run-cancel`, `request`, `request-wait`,
  `unseal`, `seal`, `hold`, `toggles`, `toggle`, `notes`, `note`.

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
