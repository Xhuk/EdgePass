# Stack: Kiri and OpenBao

This project was built against two young, opinionated pieces. Both are good choices
for a single machine. Both have limits you should know before you trust your
credentials to them.

## Kiri — the local container engine

Kiri is a lightweight container engine. It gives you Docker-compatible workflows
without the full Docker Desktop footprint. This project used it for the local CI
runner and for the demo stack.

**Strengths**

- Light on disk and memory. It starts fast on a laptop.
- Docker-compatible CLI, so existing commands and compose files usually work.
- The project is young and moves quickly. Features you ask for arrive.

**What to watch**

- Young means fewer users. A rare bug has fewer people who already hit it.
- Pin the version you tested. Do not auto-update an engine that runs your deploys.
- Community answers are scarce. Read the changelog before you upgrade.
- It is not a Docker replacement for every edge case. Multi-node, Swarm and some
  networking features may behave differently.

**Recommendation.** Use Kiri for local development and for the CI runner. Keep the
vault native, not in a container, unless you have a reason.

## OpenBao — the vault

OpenBao is an open-source fork of HashiCorp Vault, under the MPL-2.0 license. It
keeps the API, the CLI and the mental model, without the license change that moved
Vault to BUSL.

**Strengths**

- **Open license.** MPL-2.0. You can use it, fork it and ship it without a
  commercial agreement.
- **The Vault API.** Existing clients, SDKs and documentation mostly apply.
- **KV v1 and v2.** Simple key-value, plus versioned secrets with metadata.
- **Namespaces.** One vault, many isolated tenants, each with its own mounts and
  policies.
- **Policies and AppRole.** Least privilege is a first-class idea, not an add-on.
- **Raft storage.** A single binary, a local data directory, no external database.
- **Audit devices.** Every request can be recorded, which is what an incident needs.

**What to watch**

- It starts **sealed** after a restart. Somebody must unseal it. Plan for that.
- A snapshot is the whole vault in clear text. Encrypt it, always.
- Losing the unseal keys loses the data. There is no recovery.
- The API is Vault's, so most guides apply — but read OpenBao's own release notes
  before you rely on a specific behavior.

**Recommendation.** For a single machine, run OpenBao natively with raft storage and
loopback-only listeners. Add namespaces per project. Give every consumer its own
token or AppRole. Never share a root token with an app.

## Why not Infisical

Infisical is a good product and it plays well with apps at runtime. This project is
deliberately narrower: one machine, one vault, one console. If you need a hosted
secret platform with a web UI for a team, look at Infisical. If you need a local
vault you fully own, keep OpenBao.

## Version pinning

Record the versions you tested in your own notes. This project does not pin your
engine or your vault. It only refuses to talk to a vault that is not on loopback.
