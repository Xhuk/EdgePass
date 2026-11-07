# Distribution: scoop and winget

Two package managers, for one reason: an install done by a package manager does
not carry the "downloaded from the internet" mark, so SmartScreen does not
interrupt the user. Neither of them replaces a trusted signature. They reduce the
friction of one.

## Scoop first

Scoop takes the portable `edgepass.exe`, not the NSIS installer. The installer
writes to `%LOCALAPPDATA%`, creates a logon task and registers an uninstaller, and
none of that survives a Scoop upgrade cleanly. The portable binary is the right
shape for a scoop bucket.

### One-time: the bucket

Create a public repository named `scoop-bucket`. Inside it, a `bucket/` folder with
`edgepass.json`, copied from `packaging/scoop/edgepass.json`.

```
scoop-bucket/
  bucket/
    edgepass.json
```

Push it. That is the whole bucket.

### For the user

```powershell
scoop bucket add xhuk https://github.com/Xhuk/scoop-bucket
scoop install edgepass
```

To upgrade later:

```powershell
scoop update
scoop update edgepass
```

### On every release

Update two fields in `bucket/edgepass.json`: `version` and the `hash` of the new
`edgepass.exe`. `checkver` and `autoupdate` handle the URL. Do not type the hash by
hand: copy it from `Get-FileHash`.

The manifest also carries the thin parts of the lifecycle Scoop does not infer:

- `notes` says to quit the widget before `scoop update` (Windows cannot replace a
  running `.exe`), to install the WebView2 Runtime, and that config/index/logs
  live in `%APPDATA%\edgePass` and survive an uninstall.
- `post_uninstall` unregisters the `edgePass` logon task. Scoop never runs the
  NSIS uninstaller, so without this the task outlives the binary.

`packaging/scoop/edgepass.json` in this repo and `bucket/edgepass.json` in the
bucket must stay byte-identical: the bucket is the published copy, the repo copy
is what gets reviewed in a PR.

## winget

winget uses the NSIS installer, and it goes into the central catalog
`microsoft/winget-pkgs`. The manifest is **three files**, not one: the catalog
rejects a singleton with `Manifest type not supported`. They live in
`packaging/winget/` and travel together:

| File | `ManifestType` |
|---|---|
| `Xhuk.EdgePass.yaml` | `version` |
| `Xhuk.EdgePass.locale.en-US.yaml` | `defaultLocale` |
| `Xhuk.EdgePass.installer.yaml` | `installer` |

### Validate

```powershell
winget validate --manifest packaging\winget
```

A pass here is **not** proof that the PR will pass: `winget validate` accepts a
singleton and answers "succeeded", while the catalog does not. The only real
proof is the check on the pull request. See `docs/WINGET-SUBMISSION.md`.

### Submit

Install the tool once:

```powershell
winget install Microsoft.WingetCreate
```

Then let it open the pull request:

```powershell
wingetcreate submit packaging\winget\Xhuk.EdgePass.yaml
```

The first submission is reviewed by a person. Later versions go through the same
path, faster.

### On every release

Update `PackageVersion`, `InstallerUrl`, `InstallerSha256`, `ReleaseDate` and
`ReleaseNotesUrl`. The SHA256 is the one of the **signed** setup file.

## Release checklist (0.5.0 onward)

> **Which repo.** The bucket downloads from `Xhuk/EdgePass` (public). The
> workflows below only run where they are committed, so the release must be cut
> in the public repo. A tag pushed to the private `edgepass-dev` remote produces
> an asset the bucket cannot download.

1. Bump `version` in `package.json`, `src-tauri/Cargo.toml` and
   `src-tauri/tauri.conf.json` (and `src-tauri/Cargo.lock`, which `cargo`
   rewrites). Put the same version at the top of `CHANGELOG.md`.
2. Commit, tag `vX.Y.Z`, push the tag. `.github/workflows/release.yml` builds the
   portable exe **and** the NSIS setup, writes `SHA256SUMS.txt` with both, signs
   them when `WINDOWS_CERT_PFX_BASE64` and `WINDOWS_CERT_PASSWORD` exist, and
   attaches all three files to the GitHub release. Without a certificate the
   binaries are unsigned, which is what the manifest notes say.
3. **Wait for the asset to answer** at
   `https://github.com/Xhuk/EdgePass/releases/download/vX.Y.Z/edgepass.exe`.
   Do not touch the bucket before that: a manifest pointing at a 404 breaks
   `scoop update` for everyone.
4. Hash the *published* files, not the local build:
   `Get-FileHash edgepass.exe -Algorithm SHA256` for Scoop, and the same for
   `edgePass_X.Y.Z_x64-setup.exe` for winget.
5. Update `version`, `url` and `hash` in the bucket, then copy the same file into
   `packaging/scoop/edgepass.json` (they must stay identical).
6. On a second machine: `scoop update; scoop update edgepass`, launch it, and
   check `{"op":"ping"}` returns the new version.
7. winget: update `PackageVersion`, `InstallerUrl`, `InstallerSha256` (the
   **setup**, not the portable exe) and `ReleaseDate` in `packaging/winget/`,
   then `wingetcreate submit`.

## Signing

The 0.5.0 binary is Authenticode-signed with the project certificate
`CN=Firma Instalador` (self-signed, code-signing EKU, valid until 2028-11-05),
which lives in the Windows certificate store. Locally:

```powershell
& "C:\Program Files (x86)\Windows Kits\10\bin\10.0.22621.0\x64\signtool.exe" sign `
  /fd SHA256 /td SHA256 /tr http://timestamp.digicert.com `
  /sha1 DFB044D2258C3A8361CBF68DC3B86A644898BCB3 /d "edgePass" `
  src-tauri\target\release\edgepass.exe
```

For CI, the private key was exported from that store entry and the two secrets
`WINDOWS_CERT_PFX_BASE64` and `WINDOWS_CERT_PASSWORD` are set in
`Xhuk/EdgePass`; `release.yml` signs only when they exist.

**Limitation, stated plainly:** a self-signed certificate is not trusted by
anyone else's machine, so SmartScreen still warns until the root is installed.
Removing that warning for every user requires an OV/EV code-signing certificate
from a public CA. The signature still proves the binary was not modified after
the build, and the timestamp keeps it valid past the certificate's expiry.

## What each one gives you

| | Scope | Install | Upgrade | Works offline |
|---|---|---|---|---|
| Scoop | user, portable | `scoop install edgepass` | `scoop update edgepass` | yes, after first download |
| winget | user, installer | `winget install Xhuk.EdgePass` | `winget upgrade` | no |

Scoop is faster to publish: the bucket is yours, no review. winget reaches more
people but validates every submission.

## What neither one fixes

- The signature. A package manager avoids the SmartScreen path; it does not make
  the binary trusted. For a CA-backed signature see the SignPath Foundation program
  in the README discussion, or a paid certificate.
- The prerequisite. OpenBao must exist and be unsealed before the app is useful.
  The installer cannot do that for the user.

## Checklist for a new version

1. Bump the version in `tauri.conf.json`, `Cargo.toml` and `package.json`.
2. Build: `npm run tauri build -- --bundles nsis --target x86_64-pc-windows-msvc`.
3. Sign the setup and the portable exe.
4. Upload both to the release.
5. Copy the two SHA256 values.
6. Update `packaging/scoop/edgepass.json` (version, hash) and the three files in
   `packaging/winget/` (version, url, sha, date).
7. Push the bucket, submit the winget manifests.

The winget pull request needs a signed Contributor License Agreement, once, from
the account that opens it. The bot asks and the account answers
`@microsoft-github-policy-service agree`. Without that the PR cannot merge.
