# pulith

Pulith is a single Rust crate for composing typed artifact behaviors without a hidden package-manager policy layer.

Its core state transition is:

```text
WithSource -> Chosen -> Acquired -> Verified -> Prepared -> Applied -> Remembered
```

Each transition owns an associated `Need`, `Evidence`, `Error`, and `Output`. Callers choose policy and compose concrete effects.

## Current scope

The active implementation is `crates/pulith`.

| Module | Behavior |
| --- | --- |
| `application` | Typed intent, item, target, and operation vocabulary |
| `behavior` | Transition traits and state nodes |
| `evidence` | Evidence-chain types carried across transitions |
| `local` | Local acquire, prepare, staged file/directory apply, and in-memory remember |
| `hash` | Typed BLAKE3 and SHA-2 verification |
| `archive` | ZIP/TAR preparation with traversal, symlink, entry-count, and byte limits |
| `net` | HTTP acquire with retry, resume, admission, body pacing, staging, and attempt evidence |

Removed side crates and examples are historical. Their names are not current APIs.

## Features

```text
default = local + sync

execution:
  sync
  async
  runtime-tokio -> async + tokio

filesystem:
  local

network:
  net -> local
  ureq -> net + sync
  reqwest -> net + runtime-tokio

hashing:
  hash
  blake3 -> hash
  sha2 -> hash

archives:
  zip -> local
  tar -> local
  gzip -> tar
  xz -> tar
  zstd -> tar
```

There is deliberately no empty `archive`, `object`, `compress`, `fs-extra`, or `json` capability feature.

## Use

```toml
[dependencies]
pulith = { version = "0.1", features = ["ureq", "blake3", "zip"] }
```

The API is intentionally typed rather than configured through a global context. Build an `Intent`, attach a typed source, select it, then pass the resulting node through the behavior implementations required by the caller.

See [`docs/architecture.md`](docs/architecture.md) for the full contract and feature boundaries.

## Guarantees and limits

- destination writes are staged before publication where the concrete behavior supports it;
- archive paths and symlink entries are rejected by default;
- retry, resume, admission, and body-pacing decisions produce attempt evidence;
- `max_bytes` is checked before body pacing and persistence;
- request admission and decoded-body pacing are separate shared resources;
- network pacing controls decoded-body materialization, not kernel/TLS/HTTP flow-control timing;
- `ExistingExtractRoot` is an exclusive destructive capability: preparation clears it recursively;
- local path safety assumes trusted parent directories; it is not a hostile concurrent-filesystem sandbox;
- dependency solving, global rollback, lifecycle stores, installation policy, and package-manager orchestration are out of scope.

## Development

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
```

Feature-gated behavior must also compile with `--no-default-features` and its smallest public feature combination.

Engineering constraints live in [`docs/AGENT.md`](docs/AGENT.md). Design and execution history lives in `docs/report/` and is non-authoritative unless referenced by the current architecture.

## License

Apache-2.0. See [`LICENSE`](LICENSE).
