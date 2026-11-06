---
name: openbao-bootstrap
description: Use when a human needs to install, initialize, unseal, secure or back up a local OpenBao vault before edgePass can work. Covers the prerequisites, the init keys, a least-privilege token, the backup folder and the restore drill. Trigger on "instalar openbao", "no arranca edgepass", "el vault está sellado", "configurar el vault", "respaldar los secretos".
---

# OpenBao bootstrap — from zero to a working vault

edgePass is a console. Without a local OpenBao it has nothing to show. This skill
walks a human from an empty machine to a vault that edgePass can use, and leaves a
backup story behind.

## Before you start: ask three questions

1. Is OpenBao already installed and does it listen on loopback?
2. Is the vault initialized, and does the human have the init file?
3. Is the data directory on a local disk that gets backed up?

If the answer to 2 is no and the vault is already initialized, **stop**. Without the
unseal keys the data is unrecoverable. Do not re-initialize. Ask where the keys are.

## Hard rules

1. **Never print a key, a token or a value.** Not even partially. Not to verify.
2. **Never ask for a key in the chat.** The human keeps it; you get a path or a
   confirmation that it exists.
3. **Never commit** an init file, a snapshot, a `.env` or a token file.
4. **Never store the init file next to the snapshots.** Together they are the whole
   vault.
5. If a value reaches the output, treat it as compromised. Rotate, then report.

## Step 1 — verify the install

```powershell
bao version
bao status
```

Expected: a version, and `Sealed: false`. `Sealed: true` is normal after a reboot.
The vault starts sealed on purpose.

Check the listener:

```powershell
curl.exe -sS -o NUL -w "%{http_code}\n" http://127.0.0.1:8200/v1/sys/health
```

`200` unsealed, `503` sealed, `501` not initialized. Any other address than loopback
is out of scope for edgePass.

## Step 2 — initialize once

```powershell
bao operator init -key-shares=5 -key-threshold=3
```

The output contains the unseal keys and the root token. **Do not let them scroll.**
Direct the human to write them down or save them to a protected path they choose.
Then unseal:

```powershell
bao operator unseal    # three times, one key each time
```

Recommend they keep the init file outside every repository, in a folder with
restricted permissions, and a second copy somewhere else.

## Step 3 — a token for edgePass

Do not hand the root token to the app. Create a policy and a scoped token.

```hcl
path "kv/*" {
  capabilities = ["create", "read", "update", "delete", "list"]
}
```

Adjust the mount and the paths to what the human actually needs. Then:

```powershell
bao policy write edgepass-readwrite edgepass-policy.hcl
bao token create -policy=edgepass-readwrite -ttl=720h
```

Give the token to edgePass through its Config screen. edgePass stores it in the OS
keychain. Do not write it to a file.

## Step 4 — backup, before the first real secret

A raft snapshot is the entire vault in clear text. It is not a file to leave in a
synced folder.

1. Create a backup folder on a local disk, outside the data directory.
2. Take the first snapshot: `bao operator raft snapshot save <folder>\openbao-<date>.snap`
3. Encrypt it immediately. `age` or `gpg`. Keep the encryption key elsewhere.
4. Copy it off the machine.
5. Schedule the snapshot daily. A backup that depends on memory does not exist.
6. Run a restore drill on a throwaway instance. If the drill never ran, there is no
   backup.

## Step 5 — verify that edgePass is ready

1. `bao status` reports unsealed.
2. edgePass Config points to the same address.
3. A test secret round-trips: write it, search it, copy it from the widget.
4. The child-process path works: run a command that reads the variable and prints a
   marker, never the value.

If step 3 fails, the token or the policy is wrong. If step 4 fails but step 3 works,
the child environment is wrong.

## Failure modes

| Symptom | Cause | Fix |
|---|---|---|
| `Sealed: true` after reboot | by design | unseal with three keys |
| Port 8200 refuses | service not running | start the service, check the config |
| Token rejected | expired or wrong policy | create a new scoped token |
| Paths missing in the widget | KV v1 vs v2, or wrong namespace | check `sys/mounts` and the namespace |
| Values appear in a log | a consumer prints them | rotate the value, fix the consumer |

## What not to do

- Do not run two vaults on the same raft directory.
- Do not keep an editable copy of the vault and call it a sync.
- Do not publish port 8200.
- Do not give an app the root token.
- Do not disable the loopback rule to "make it work".
