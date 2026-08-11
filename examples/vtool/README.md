# vtool example

`vtool` is a small versioned-artifact manager built by composing Pulith behaviors. It is intended
to show library boundaries, not to be installed as a supported package manager.

## What it does

For the current operating system, `vtool` selects one atomic source-and-digest declaration from a
TOML manifest. It can:

1. plan the resolved source and destination;
2. acquire a local file/directory or an HTTP artifact;
3. verify the declared BLAKE3 or SHA-256 digest;
4. prepare supported archives or copy unarchived material into private staging;
5. publish `<root>/artifacts/<name>/<version>`;
6. link an optional exposed subdirectory into an absolute consumer view;
7. inspect and reconcile the result during bounded repair attempts;
8. atomically update a bounded state snapshot under `<root>/.vtool/state`.

Install and activate are deliberately separate. Deactivation removes the view, not the published
version.

## Build

From the Pulith repository:

```text
cargo build --no-default-features --features local,http-ureq,zip,tar,gzip,blake3,sha2,serde --example vtool
```

The executable is `target/debug/vtool` on Unix and `target\debug\vtool.exe` on Windows.

## Manifest

The manifest must declare a complete source/hash pair for the host platform. Names and versions are
single path components. `link_at`, when present, must be absolute.

```toml
name = "demo-tool"
version = "1.2.0"
expose = "bin"
link_at = "/opt/demo-tool"

[linux.source]
kind = "url"
url = "https://example.invalid/demo-tool-1.2.0.tar.gz"

[linux.hash]
kind = "sha2"
hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

[windows.source]
kind = "local"
path = "vendor/demo-tool-1.2.0.zip"

[windows.hash]
kind = "blake3"
hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
```

Replace the URLs, paths, digests, and `link_at` value with real values. The example does not weaken
digest verification for placeholder data.

## Use

Commands have the same meaning on Linux and Windows; only executable and absolute-path syntax
differs.

```text
vtool plan       --root <absolute-root> <manifest>
vtool install    --root <absolute-root> <manifest>
vtool activate   --root <absolute-root> <manifest>
vtool repair     --root <absolute-root> <manifest> --attempts 3
vtool deactivate --root <absolute-root> <manifest>
```

`plan` is read-only. `install` publishes a version but does not change the active view. `activate`
links the declared view, and `deactivate` removes it. `repair` repeatedly inspects, reconciles, and
performs only the required install or activation behavior up to the declared attempt bound.

## Pulith features and APIs

| Feature | APIs exercised | Purpose |
| --- | --- | --- |
| `local` | `LocalSource`, `LocalTarget`, `StagedTree`, `RecordStore`, `Link`, `Unlink`, `Inspect`, `Reconcile` | local custody, publication, views, observations, and state |
| `http-ureq` | `RemoteUrl`, `RemoteSource`, `Acquire` | synchronous staged HTTP acquisition |
| `zip`, `tar`, `gzip` | `ArchiveKind`, `ArchivePolicy` | archive detection and preparation |
| `blake3`, `sha2` | `DigestValue`, `Verify` | explicit artifact identity |
| `serde` | digest and manifest deserialization | strict TOML declarations |

## Guarantees and non-goals

- Source and digest are selected together for one platform.
- Publication uses private staging; a failed preparation is not exposed as the version directory.
- State replacement is bounded and atomic, and crash tests cover the snapshot boundary.
- Repair is bounded and explicit; it is not a daemon.
- Parent directories are trusted. The example is not a hostile-filesystem sandbox.
- Retention, dependency solving, signatures, privilege elevation, and multi-package transactions are
  caller concerns and intentionally absent.

See the crate [architecture](../../docs/architecture.md) and the `local`, `net`, `archive`, and
`hash` modules in rustdoc for the underlying contracts.
