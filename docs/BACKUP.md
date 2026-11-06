# Backup

**A snapshot contains every secret in clear text.** Encrypt it, and keep it away
from the machine that holds the vault.

## Where the data lives

| What | Where | Notes |
|---|---|---|
| Raft data | the OpenBao `storage.path` | One copy. Never share it between two live instances. |
| Unseal keys, root token | your init file | Highest sensitivity. Keep separate from snapshots. |
| edgePass config | `%APPDATA%\edgePass\` | Metadata only. No values. |
| edgePass index | `%APPDATA%\edgePass\` SQLite | Paths and names. No values. |
| Snapshots | your backup folder | Encrypted. Never committed. |

edgePass stores no secret values on disk. If you find a value in that folder, it
is a bug and an incident.

## Snapshot

```powershell
bao operator raft snapshot save C:\vault-backups\openbao-2026-01-31.snap
```

Then encrypt it immediately. `age` and `gpg` both work. Keep the encryption key
elsewhere. A snapshot next to its own key is not a backup.

## Retention

- Seven daily snapshots.
- Four weekly snapshots.
- One copy off the machine, encrypted.

Scheduled daily, not manual. A backup that depends on memory does not exist.

## Restore

Restoring is a procedure, not a command. Test it on a throwaway container.

1. Start a fresh OpenBao with an empty data directory.
2. Initialize it, using a throwaway init file.
3. Restore the snapshot:

```powershell
bao operator raft snapshot restore -force C:\vault-backups\openbao-2026-01-31.snap
```

4. Unseal with the throwaway keys.
5. Read one secret and compare it with the source. Count the paths.
6. Destroy the throwaway container.

If you have never run step 6, you do not have a backup. You have a file.

## Two sources of truth

Do not run two vaults against the same data. Two live instances on the same raft
directory corrupt the store. Do not keep an editable copy on the side and call it a
sync. One active vault, plus point-in-time snapshots.

## What breaks the backup

- A snapshot stored on the same disk as the raft data.
- An unencrypted snapshot in a synced folder.
- Unseal keys kept next to the snapshots.
- A backup job nobody watches.
- A restore never tested.
