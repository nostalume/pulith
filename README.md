# Pulith

Pulith is a Rust library for composing typed operations over external resources. It provides
small behaviors—acquire, verify, stage, publish, inspect, reconcile, link, remove, and run—without
owning your application model or hiding those effects behind a global workflow.

```rust
use pulith::{Acquire, Inspect, Reconcile};
use pulith::local::{LocalExpectation, LocalSource, LocalTarget};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let material = LocalSource::new("artifact.bin")?.acquire()?;
println!("acquired {}", material.path().display());

let (observed, _) = LocalTarget::new("artifact.bin")?.inspect(())?;
let (difference, _) = observed.reconcile(LocalExpectation::File)?;
println!("state: {difference:?}");
# Ok(())
# }
```

## Why Pulith?

Resource-management libraries often combine application policy, transport, filesystem mutation,
and global orchestration. Pulith keeps those responsibilities separate:

- behavior traits express one operation and its typed result;
- restricted resource values validate the input law for that operation;
- concrete adapters own the relevant I/O;
- evidence describes what the operation observed or changed;
- callers retain desired state, trust policy, retries across workflows, and rollback decisions.

There is no plugin registry, global context, or universal package-manager model.

## Install

Pulith requires Rust 1.88 or newer.

```text
cargo add pulith
```

Default features enable common archive preparation. For a smaller dependency surface, disable
defaults and select only the behavior you use:

```text
cargo add pulith --no-default-features --features local,sha2
```

## Behavior model

The public traits are intentionally independent:

| Behavior | Meaning |
| --- | --- |
| `Acquire` / `AsyncAcquire` | obtain material from an admitted source without publishing it |
| `Verify` | prove one fact about material against an explicit expectation |
| `Inspect` / `AsyncInspect` | observe a resource without changing it |
| `Reconcile` | compare an observation with caller-owned desired state |
| `Link` / `Unlink` | expose or withdraw a published tree through a selected view |
| `Remove` | remove the selected target |

Concrete types add operations that do not fit a universal trait, such as creating a
`StagedTree`, preparing an archive, or executing a bounded process.

## Features

| Need | Features |
| --- | --- |
| local acquisition, staging, publication, links, records, and inspection | `local` |
| managed and bounded child processes | `process` |
| Tokio process execution | `process-tokio` |
| URL, retry, admission, and pacing vocabulary | `net` |
| synchronous HTTP with ureq/rustls | `http-ureq` |
| asynchronous HTTP with reqwest/rustls and Tokio | `http-reqwest` |
| digest vocabulary | `hash` |
| exact BLAKE3 or SHA-256 verification | `blake3` or `sha2` |
| ZIP or plain TAR preparation | `zip` or `tar` |
| gzip-, xz-, or zstd-compressed TAR preparation | `gzip`, `xz`, or `zstd` |
| serializable manifest values | `serde` |

The `archive` default is `zip + gzip + xz + zstd`. It does not select a digest algorithm.
`--all-features` is an integration profile, not a consumer-facing “full” feature.

## Examples

- [`vtool`](examples/vtool/README.md) is a small versioned-artifact manager. It demonstrates local
  and HTTP acquisition, digest verification, archive preparation, atomic publication, active
  views, reconciliation, repair, and durable state records.
- [`toolhost`](examples/toolhost/README.md) builds and harvests a tool, verifies its runtime,
  publishes a versioned layout, dispatches through a compiled shim, constructs an exact child
  environment, and integrates the same service declaration with systemd or Windows SCM.

These are executable architecture examples, not supported end-user package managers or service
frameworks.

## Guarantees and boundaries

- Final destinations are published from private staging custody.
- Archive preparation rejects traversal, unsafe links, devices, collisions, and configured limit
  violations.
- HTTP acquisition returns staged material and never publishes a final destination.
- Inspection is read-only; reconciliation does not repair.
- Exact hash inspection is opt-in and reads regular-file handles.
- Process APIs bound execution time and diagnostic capture where their configuration requires it.
- Local path safety assumes trusted parent directories; Pulith is not a hostile concurrent-filesystem
  sandbox.
- Service-manager integration belongs to the `toolhost` example, not the library API.

Linux and Windows are required test platforms. macOS is best-effort until equivalent runtime
evidence is available.

## Documentation

- [Architecture](docs/architecture.md)
- [Contributor guide](CONTRIBUTING.md)
- [Project goal, principles, and stack](docs/AGENTS.md)

API details and feature-gated examples are available on
[`docs.rs/pulith`](https://docs.rs/pulith).

## License

Licensed under the [Apache License 2.0](LICENSE).
