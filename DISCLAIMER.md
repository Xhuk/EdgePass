# DISCLAIMER

**Read this before you install edgePass.**

edgePass manages credentials. It connects to a vault, reads secrets, and can inject
them into other processes. A mistake here does not cost you an afternoon. It can
expose customer data, take a service down, or destroy the only copy of a key.

## No warranty

The software is provided **as is**. There is no warranty of any kind, express or
implied, including merchantability, fitness for a particular purpose and
non-infringement. You use it at your own risk.

## You own the vault

edgePass ships **no** vault and **no** secrets. You install OpenBao yourself. You
initialize it yourself. You keep the unseal keys and the root token. The author of
this software never sees them and has no way to recover them.

**If you lose the unseal keys, the data is gone.** There is no recovery path and no
support path. Write the keys down, keep them apart, and test the restore.

## Not audited

There has been no independent security audit. The code is small enough to read.
Read it before you trust it with production credentials. Do not deploy it to a
regulated environment without your own review.

## Local only

edgePass refuses to talk to an address that is not loopback. That is a deliberate
limit, not a bug. It keeps the vault token on your machine. Do not patch it out
unless you have thought through what happens when the token leaves the host.

## The value never leaves the process

The design rule is that a secret value must not appear in a chat, a log, a file, or
a command line. Agents get secrets through a child process environment. Humans copy
them from the widget. If you find a value in a log or a transcript, treat it as
compromised: rotate it.

## Snapshots contain everything

A raft snapshot is a full copy of the vault with the secrets in clear text. Treat a
snapshot exactly as you treat the vault itself. Encrypt it. Keep it off the machine
that holds the vault. Never commit it.

## Third-party components

OpenBao, Tauri, Kiri and every dependency carry their own licenses and their own
behavior. This disclaimer does not extend to them, and their issues are not the
responsibility of this project.

## No support commitment

There is no service level, no guaranteed response time and no promise that a
breaking change gets a migration path. Pin a version if you need stability.

## Your responsibility

You decide what goes into the vault, who reads it, and how long it lives. Use
least-privilege policies. Rotate what you can. Assume that any long-lived credential
is already on its way to being leaked.

By installing or using edgePass you accept these terms.
