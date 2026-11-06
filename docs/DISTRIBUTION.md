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
