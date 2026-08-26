# Pulith

Pulith is a Rust library for safely composing work with external resources: files, archives,
downloads, child processes, versioned trees, and links. It supplies small typed operations and
leaves application policy—what should exist, whom to trust, when to retry, and how to recover—to
your code.

Use Pulith when you need the reliable parts of an installer, updater, tool manager, or deployment
utility without adopting a framework or global workflow engine.

## Quick start

Pulith supports Rust 1.88 or newer. The default feature set prepares common archive formats.

```text
cargo add pulith
```

Acquire and inspect a local artifact:

```rust
use pulith::local::{LocalExpectation, LocalSource, LocalTarget};
use pulith::{Acquire, Inspect, Reconcile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let material = LocalSource::new("artifact.bin")?.acquire()?;
    println!("acquired {}", material.path().display());

    let (observed, _) = LocalTarget::new("artifact.bin")?.inspect(())?;
    let (difference, _) = observed.reconcile(LocalExpectation::File)?;
    println!("state: {difference:?}");
    Ok(())
}
```

For a smaller dependency surface, disable defaults and select only what the program uses:

```text
cargo add pulith --no-default-features --features local,sha2
```

## What Pulith provides

Pulith separates operations that larger systems commonly entangle:

| Need | Pulith operation |
| --- | --- |
| Obtain material without publishing it | `Acquire` / `AsyncAcquire` |
| Prove a digest or another explicit fact | `Verify` |
| Observe current state without changing it | `Inspect` / `AsyncInspect` |
| Compare observation with desired state | `Reconcile` |
| Assemble and atomically publish a tree | `StagedTree` methods |
| Expose or withdraw a published tree | `Link` / `Unlink` |
| Remove a selected target | `Remove` |
| Run a bounded or managed child process | process resource methods |

These operations are independent. A caller can inspect without reconciling, verify without
publishing, or acquire with one adapter and prepare with another. Typed evidence reports what each
operation observed or changed.

Pulith deliberately does not own package identity, desired-state storage, trust policy, retry
across a whole workflow, retention, rollback decisions, or service-manager policy.

## Features

| Capability | Feature |
| --- | --- |
| Local acquisition, staging, publication, links, records, inspection | `local` |
| Bounded and managed child processes | `process` |
| Tokio process execution with matching semantics | `process-tokio` |
| URL, retry, admission, pacing, and network evidence types | `net` |
| Synchronous HTTP over rustls | `http-ureq` |
| Asynchronous HTTP over rustls and Tokio | `http-reqwest` |
| Digest vocabulary | `hash` |
| BLAKE3 or SHA-256 verification | `blake3` or `sha2` |
| ZIP or plain TAR preparation | `zip` or `tar` |
| gzip-, xz-, or zstd-compressed TAR preparation | `gzip`, `xz`, or `zstd` |
| Serializable public values | `serde` |

The default `archive` feature enables `zip + gzip + xz + zstd`; it does not select a digest.
`--all-features` is an integration-test profile, not the recommended consumer configuration.

## Safety model

- Material remains in private staging custody until an explicit publication boundary.
- Archive preparation rejects path traversal, unsafe links, devices, collisions, and configured
  resource-limit violations.
- HTTP acquisition stages bytes but cannot publish a caller's final destination.
- Inspection and reconciliation never repair state as a hidden side effect.
- Process APIs apply explicit execution and diagnostic bounds where their configuration promises
  them.
- Linux and Windows implement the same declared meaning through platform-specific mechanisms.

Pulith assumes trusted parent directories. It is not a capability sandbox against an attacker who
can concurrently replace ancestor paths.

## Examples

- [`vtool`](examples/vtool/README.md) manages versioned artifacts. It demonstrates local and HTTP
  acquisition, exact digest verification, archive preparation, atomic publication, active views,
  reconciliation, repair, and durable records.
- [`toolhost`](examples/toolhost/README.md) builds and harvests a tool, verifies its runtime,
  publishes a versioned layout, dispatches through a compiled shim, creates an exact child
  environment, and adapts one service declaration to systemd or Windows SCM.

The examples exercise architecture and expose missing library semantics; they are not supported
end-user package managers or service frameworks.

## Learn more

- [Architecture and contracts](docs/architecture.md)
- [Contributing](CONTRIBUTING.md)
- [Project goal, principles, and stack](docs/AGENTS.md)
- [API documentation](https://docs.rs/pulith)

Releases are produced from version tags after the tag, package version, and `main` revision agree.
crates.io uses GitHub trusted publishing; a GitHub Release with generated notes follows successful
registry publication.

## License

Licensed under the [Apache License 2.0](LICENSE).
