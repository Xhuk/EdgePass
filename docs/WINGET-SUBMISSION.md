# winget manifest: what Microsoft asked for

Record of the first submission, PR
[microsoft/winget-pkgs#472420](https://github.com/microsoft/winget-pkgs/pull/472420).
Date: 6 November 2026.

## What Microsoft answered

### 1. The Contributor License Agreement (CLA) is missing

Check status: `license/cla` queued. The bot says:

> This PR cannot be merged until you sign the Contributor License Agreement (CLA).

The fix is a comment from the account that owns the PR:

```
@microsoft-github-policy-service agree
```

Only the account owner can do it. It is a legal agreement, not a technical step.
If the contribution is made by a company, the form is `agree company="Name"`.

### 2. The singleton manifest does not work

Check `02. Manifest Validation`: failed in 16 seconds.

> `Manifest type not supported. ManifestType: singleton`.

The community catalog requires a set of three files, each with its own
`ManifestType`:

| File | `ManifestType` |
|---|---|
| `Xhuk.EdgePass.yaml` | `version` |
| `Xhuk.EdgePass.locale.en-US.yaml` | `defaultLocale` |
| `Xhuk.EdgePass.installer.yaml` | `installer` |

That diagnosis came from an assistant marked experimental. It was verified against
the catalog's `docs/Authoring.md`: the three-file set is the expected format.

## A trap worth recording

`winget validate` **accepts** a singleton and answers "Manifest validation
succeeded". The catalog does not accept it. Validating locally does not prove the
PR will pass. The only real test is the PR check.

## Status

| Step | Status |
|---|---|
| Three manifests written | done, in `packaging/winget/` |
| `winget validate` on the full set | done: "Manifest validation succeeded" |
| License agreement signed | **pending, done by the account owner** |
| PR with 0.5.0 | pending |

PR 472420 targets 0.3.0 and is obsolete. Once the CLA is signed, 0.5.0 is sent
with the three-file set.
