# pulith

Pulith is a single Rust crate for composing typed artifact behaviors without a hidden package-manager policy layer.

> **Status:** pre-release. Local package verification does not publish a registry artifact, and no
> release is implied by the `0.1.0` manifest version.

```text
materialize: Intent -> WithSource -> Chosen -> Acquired -> Verified -> Prepared -> Applied -> Remembered
forget:      Intent<Forget> -------------------------------------------------> Applied -> Remembered
```

Each behavior explicitly declares the associated contracts it uses: policy `Need` where required,
plus its `Evidence`, `Error`, and `Output`. Callers choose policy and compose concrete effects.

## Scope

| Module | Behavior |
| --- | --- |
| `application` | Typed intent, item, target, and operation vocabulary |
| `behavior` | Transition traits and state nodes |
| `evidence` | Evidence chains carried across transitions |
| `local` | Local acquire, prepare, staged apply, and in-memory remember |
| `hash` | Typed digest and exact digest-plus-size descriptor verification |
| `archive` | ZIP/TAR preparation with path, entry-type, and resource guards |
| `net` | HTTP acquire with retry, resume, admission, body pacing, staging, and attempt evidence |

Pulith uses mature libraries for HTTP, hashing, archive parsing, and compression codecs. It owns the typed behavior, policy, evidence, staging, and publication contracts around those mechanisms.

## Current maturity

The state graph is a composition vocabulary, not a claim that Pulith is already a complete package
manager. The concrete path currently supplied by this crate is:

| Transition | Concrete behavior today |
| --- | --- |
| `Intent -> WithSource -> Chosen` | caller-provided typed source selected by `SelectFirst` |
| `Chosen -> Acquired` | local material or staged HTTP download |
| `Acquired -> Verified` | explicit identity pass-through, typed digest, or exact digest-plus-size descriptor |
| `Verified -> Prepared` | identity preparation or guarded ZIP/TAR extraction |
| `Prepared -> Applied` | staged local file/tree publication for create and replace operations |
| `Intent<Forget> -> Applied` | direct idempotent local target removal; no artificial source acquisition |
| `Applied -> Remembered` | `MemoryRemember`, which carries the receipt and evidence in memory only |

The asynchronous transition traits other than acquisition and custom remember behaviors are
extension vocabulary. Pulith does not currently provide source discovery, dependency solving, a
durable installation database, multi-target transactions, reconciliation, or system package-manager
integration. Those require demonstrated callers and explicit storage/rollback laws; they are not
hidden behind the existing state names.

`ArtifactDescriptor<A>` identifies one exact raw representation by digest and byte size, independent
of whether it came from a local path, `ureq`, or `reqwest`. Descriptor equality proves only that the
material matches the supplied expectation. It does not authenticate who supplied that expectation,
authorize a publisher, or establish provenance; those remain separate caller-owned policy/trust
behaviors until a concrete adapter justifies them.

## Features

```text
default = local

network:
  net -> local
  ureq -> net
  reqwest -> net + tokio

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
pulith = { path = "../pulith", features = ["ureq", "blake3", "zip"] }
```

Build an `Intent`, attach a typed source, select it, then pass the node through the concrete behavior implementations required by the caller. There is no global context, plugin registry, or hidden workflow policy.

## Guarantees and limits

- final destination writes are staged before publication where the concrete behavior supports it;
- archive traversal, symlink, hardlink, and unsupported entry types are rejected by default;
- archive path collisions are rejected with portable case-folded identity on every platform;
- archive extraction uses an exclusive destructive `ExtractWorkspace` before final `LocalApply` publication;
- per-entry and total archive limits are enforced against observed materialized bytes, while decoded-container limits also bound TAR metadata, padding, and stripped entries;
- declared-size mismatches are rejected, and extraction errors report any subsequent workspace-cleanup failure;
- retry, validator-bound resume, admission, and body-pacing decisions produce attempt evidence;
- HTTP partial bytes are recombined only with a strong validator, a terminal `Content-Range`, and an
  observed body length matching that range; weak or conflicting validators are rejected;
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
