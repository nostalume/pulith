# Publish Readiness Matrix

## Active package

| Package | Version | Source state | Quality state | Release state | Blockers |
| --- | --- | --- | --- | --- | --- |
| `pulith` | `0.1.0` | single active workspace package | local cutover gates passed; clean-commit CI pending | **NO-GO** | API/SemVer review, archive observed-byte hardening decision, clean crates.io dry run, cross-platform CI |

## Feature readiness

| Feature family | Minimum combination | Current source status |
| --- | --- | --- |
| execution | `sync`, `async`, `runtime-tokio` | compiles |
| local | `local` | compiles; staged apply tests present |
| network | `net`, `ureq`, `reqwest` | compiles; sync/async integration tests present |
| hash | `hash`, `blake3`, `sha2` | compiles; concrete algorithm tests present |
| archive | `zip`, `tar`, `gzip`, `xz`, `zstd` | compiles; safety/limit tests present; observed-byte limit review remains |

## Historical packages

The deleted side crates and examples are historical releases only. Their published versions do not imply that the consolidated crate is published or release-ready:

```text
pulith-archive
pulith-fetch
pulith-fs
pulith-install
pulith-lock
pulith-platform
pulith-resource
pulith-serde-backend
pulith-shim
pulith-source
pulith-state
pulith-store
pulith-verify
pulith-version
```

## Decision rule

Change `NO-GO` only after every unchecked item in [`checklist.md`](checklist.md) that applies to the target release has evidence from the release commit.
