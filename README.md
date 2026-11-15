# Pulith

Pulith is a Rust library for safely composing work with external resources: files, archives,
downloads, child processes, versioned trees, and links. It supplies small typed operations and
leaves application policy—what should exist, whom to trust, when to retry, and how to recover—to
your code.

Use Pulith when you need the reliable parts of an installer, updater, tool manager, or deployment
utility without adopting a framework or global workflow engine.

## Quick start

Pulith supports Rust 1.88 or newer. The default feature set is `local`: local artifacts, staging, publication, views, and inspection. Nothing else is pulled in.

```text
cargo add pulith
```

Inspect a local artifact:

```rust
use pulith::local::{LocalExpectation, LocalPath};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target = LocalPath::new("artifact.bin")?;
    let entry = target.inspect_entry()?;
    let difference = entry.compare(&LocalExpectation::File);
    println!("state: {difference:?}");
    Ok(())
}
```

Additional capabilities are opt-in. Add archive preparation or a digest explicitly:

```text
cargo add pulith --features archive
cargo add pulith --features local,sha2
```

To start from an empty feature set, use `--no-default-features`.

## What Pulith provides

Pulith separates operations that larger systems commonly entangle:

| Need | Pulith operation |
| --- | --- |
| Inspect an entry or read its data | `LocalPath::inspect_entry` / `open_file` |
| Capture bytes for later use | `OpenedFile::snapshot_verified` / `UreqResources::download` |
| Describe a live file or verify captured bytes | `OpenedFile::digest` / `FileSnapshot::verify_digest` |
| Verify and publish a local file | `LocalPath::publish_verified_file` |
| Compare observed facts with desired state | `LocalObservation::compare` / `ArtifactDescriptor::compare` |
| Read a bounded record | `RecordStore::read` |
| Stage and publish output | `LocalPath::stage` / `StagedTree::publish` |
| Inspect, activate, or withdraw a view | `LocalPath::inspect_view` / `link_view` / `unlink_view` |
| Run a bounded or managed child process | `PreparedProcess::execute` / `ManagedProcess::start` |

Captured files implement `Read` and `Seek`. Observations describe what was read. Publication
receipts describe the filesystem effect that completed.

Pulith deliberately does not own package identity, desired-state storage, trust policy, retry
across a whole workflow, retention, rollback decisions, or service-manager policy.

## Features

| Kind | Capability | Feature |
| --- | --- | --- |
| Core (default) | Local entry access, staging, publication, links, records, inspection | `local` |
| Adapters | Bounded and managed child processes | `process` |
| | URL, retry, admission, pacing, and network evidence types | `net` |
| | HTTP over rustls | `http-ureq` |
| Digests | Digest vocabulary | `hash` |
| | BLAKE3 or SHA-256 verification of local material | `blake3` / `sha2` |
| Formats | ZIP (`stored` / `deflate` entries) or TAR entry-stream preparation | `zip` / `tar` |
| | Both | `archive` |
| Shape | `Deserialize` for public values | `serde` |

Feature edges are explicit in `Cargo.toml`: `archive` expands to `zip + tar`; digest features
require `local`, because verification acts on local material. The format features do not require
`local` and do not imply each other. The HTTP adapter requires `net` and `local`. `serde` only
switches `Deserialize` implementations on; it adds no dependency to `net`. `--all-features` is an
integration-test profile, not the recommended consumer configuration.

Archive extraction takes a reader, not a path: `extract_zip` wants a `Read + Seek` stream and
`extract_tar` wants a `Read` stream. Decoding is the caller's job, so `flate2`, `xz2`, or `zstd`
stay out of this crate — pass `GzDecoder::new(file)` and get TAR preparation, or pass the file
directly for plain TAR. What this crate owns is the custody contract: path and collision policy,
symlink and device rejection, four independent limits, evidence, and no residue on failure.

The `zip` feature compiles only the entry methods it promises: `stored` and `deflate`. Zip entries
using bzip2, lzma, ppmd, zstd, or AES encryption fail with an unsupported-method error instead of
pulling those codecs in. Header consistency (duplicate names, local/central mismatches, overlapping
entries) is validated by `zip` itself before anything is written.

## Safety model

- Material remains in private staging custody until an explicit publication boundary.
- Staging is not a zero-write operation: creating private staging custody may create the target's
  ancestor directories. A caller that requires a plan with no filesystem writes must not call
  `stage()`.
- Archive preparation rejects path traversal, unsafe links, devices, collisions, and configured
  resource-limit violations.
- An HTTP download captures bytes but cannot publish a caller's final destination.
- HTTP adapters use direct transport by default and never read ambient or system proxy
  settings implicitly. To use a proxy for inspection or download, supply an agent through
  `UreqResources::from_agent`.
- Inspection and reconciliation never repair state as a hidden side effect.
- Process APIs apply explicit execution and diagnostic bounds where their configuration promises
  them. Managed processes are headless: standard streams are null, and a caller supplies the
  reaping deadline to `stop_within`.
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
