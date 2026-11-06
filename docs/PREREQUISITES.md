# Prerequisites

edgePass is a console. The vault is the product. You must have a local OpenBao
before the widget is useful.

## 1. OpenBao

Install OpenBao 2.x and run it as a local service. It must listen on loopback.

- Windows binary: `bao.exe` on your `PATH`, or a full path in the service config.
- Default address: `http://127.0.0.1:8200`.
- Storage: raft, on a local disk.
- UI and API: loopback only. Never publish port 8200 to the internet.

Two settings matter for edgePass:

1. **Address.** edgePass refuses anything that is not loopback. A remote vault is
   out of scope by design.
2. **Seal state.** The vault starts sealed. edgePass can unseal it on demand, but
   it needs the unseal keys, which live in your init file.

## 2. Initialize the vault

Run the `bao operator init` step once. Store the init file outside every
repository, with restrictive permissions. It contains the unseal keys and the root
token. Without it the vault cannot be opened, and the data is lost.

After a reboot the service starts **sealed**. Unseal it before the app can read.

## 3. A token for edgePass

edgePass keeps a token in the operating system keychain. Two options:

- The root token, for a personal machine. Simple, broad, and easy to misuse.
- A scoped token with a policy that allows only the paths you want. Recommended.

Create a policy that grants `read` on your KV mount and `list` on the metadata
path, then create a token with that policy. Do not grant `root` to a client that
only needs to read.

## 4. Mounts and paths

edgePass detects the engine version per namespace. A KV **v1** mount is read and
written as `kv/<path>`. A KV **v2** mount uses `kv/data/<path>` and
`kv/metadata/<path>`. The app handles the difference. Your paths do not change.

Prefer namespaces per project: namespace `<project>`, path `kv/<scope>`.

## 5. Docker (optional)

Docker is needed only for the CI runner LEDs. The engine this project was developed
against is Kiri. Any Docker-compatible engine works if it exposes the same CLI.

The vault does **not** need Docker. Keep it native unless you have a reason.

## 6. Checklist

1. `bao status` returns unsealed.
2. `curl http://127.0.0.1:8200/v1/sys/health` returns 200.
3. The token can list your KV mount.
4. edgePass **Config** shows the same address.
5. A test secret round-trips: write it, search it, copy it from the widget.

If step 5 fails, the problem is the token or the policy. Look in **Logs**.
