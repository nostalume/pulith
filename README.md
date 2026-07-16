# pulith

Pulith is a single Rust crate for composing typed artifact behaviors without a hidden package-manager policy layer.

```text
Intent -> WithSource -> Chosen -> Acquired -> Verified -> Prepared -> Applied -> Remembered
```

Each transition owns an associated `Need`, `Evidence`, `Error`, and `Output`. Callers choose policy and compose concrete effects.

## Scope

| Module | Behavior |
| --- | --- |
| `application` | Typed intent, item, target, and operation vocabulary |
| `behavior` | Transition traits and state nodes |
| `evidence` | Evidence chains carried across transitions |
| `local` | Local acquire, prepare, staged apply, and in-memory remember |
| `hash` | Typed BLAKE3 and SHA-2 verification |
| `archive` | ZIP/TAR preparation with path, entry-type, and resource guards |
| `net` | HTTP acquire with retry, resume, admission, body pacing, staging, and attempt evidence |

Pulith uses mature libraries for HTTP, hashing, archive parsing, and compression codecs. It owns the typed behavior, policy, evidence, staging, and publication contracts around those mechanisms.

## Features

```text
default = local + sync

execution:
  sync
  async
  runtime-tokio -> async + tokio

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
  gzip/xz/zstd -> tar
```

There is deliberately no empty `archive`, `object`, `compress`, `fs-extra`, or `json` capability feature.

## Use

```toml
[dependencies]
pulith = { version = "0.1", features = ["ureq", "blake3", "zip"] }
```

Build an `Intent`, attach a typed source, select it, then pass the node through the concrete behavior implementations required by the caller. There is no global context, plugin registry, or hidden workflow policy.

## Guarantees and limits

- final destination writes are staged before publication where the concrete behavior supports it;
- archive traversal, symlink, hardlink, and unsupported entry types are rejected by default;
- archive extraction uses an exclusive destructive scratch root before final `LocalApply` publication;
- retry, resume, admission, and body-pacing decisions produce attempt evidence;
- network `max_bytes` is checked before body pacing and persistence;
- request admission and decoded-body pacing are separate shared resources;
- decoded-body pacing does not control kernel, TLS, or HTTP flow-control timing;
- local path safety assumes trusted parent directories, not a hostile concurrent filesystem;
- dependency solving, lifecycle stores, installation policy, and package-manager orchestration are out of scope.

## Development

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps
```

Feature-gated behavior must also compile with `--no-default-features` and its smallest supported feature combination. Engineering rules and the current tech stack are in [`AGENTS.md`](AGENTS.md).

## License

Apache-2.0. See [`LICENSE`](LICENSE).
