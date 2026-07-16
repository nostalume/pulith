# pulith

Pulith is a single-root Rust crate ecosystem for composing typed external-resource management
behaviors without a hidden package-manager or implementation-specific policy layer.

> **Status:** pre-release. Local package verification does not publish a registry artifact, and no
> release is implied by the `0.1.0` manifest version.

```text
materialize: Intent -> WithSource -> Chosen -> Acquired -> Verified -> Prepared -> Applied -> Remembered
forget:      Intent<Forget> -------------------------------------------------> Applied -> Remembered
observe:     LocalTarget -> Inspected -> Reconciled
             RemoteUrl  -> Inspected
```

Each behavior explicitly declares the associated contracts it uses: policy `Need` where required,
plus its `Evidence`, `Error`, and `Output`. Callers choose policy and compose concrete effects.

## Scope

| Module | Behavior |
| --- | --- |
| `application` | Typed intent, item, target, and operation vocabulary |
| `behavior` | Transition traits and state nodes |
| `evidence` | Evidence chains carried across transitions |
| `local` | Local acquire, prepare, staged apply, inspect, pure reconcile, and in-memory remember |
| `hash` | Typed digest and exact digest-plus-size descriptor verification |
| `archive` | ZIP/TAR preparation with path, entry-type, and resource guards |
| `net` | HTTP HEAD inspection plus acquire with retry, resume, admission, body pacing, staging, and attempt evidence |

Pulith uses mature libraries for HTTP, hashing, archive parsing, and compression codecs. It owns the typed behavior, policy, evidence, staging, and publication contracts around those mechanisms.

## Architecture

Pulith keeps three axes orthogonal:

```text
behavior law
    x resource-specific semantics
        x concrete adapter
```

A behavior law declares its input, policy `Need` where required, `Evidence`, `Error`, `Output`,
effect boundary, and failure law. Resource semantics define what those terms mean for a filesystem,
HTTP representation, archive, digest, trust system, durable store, or another external resource. An
adapter implements one demonstrated behavior/resource intersection. Callers retain application
identity, desired state, trust/admission policy, durable aggregates, orchestration, and rollback or
retention policy.

The behavior vocabulary is not a universal lifecycle. Current and candidate families are:

| Family | Behaviors | Boundary |
| --- | --- | --- |
| Choice | attach, select; offer remains caller vocabulary | no acquisition or authorization claim |
| Materialization | acquire, verify, prepare | transfer, factual proof, and transformation stay separate |
| Mutation | apply, forget | explicit target effect; `Forget` does not claim ownership |
| Memory | remember | durability only when a concrete adapter proves it |
| Observation | inspect | read-only resource facts; filesystem absence and HTTP status retain resource-specific meaning |
| Convergence | reconcile | caller expectation plus observation; never repairs or adopts |
| Future gated families | admit, recover, activate, prune | require a demonstrated authority and adapter before public vocabulary |

Concrete behavior contracts currently admitted to the public surface are:

| Behavior | Need / authority | Evidence and output | Effect and failure law |
| --- | --- | --- | --- |
| explicit select | caller-attached typed source | `Chosen` source | no provider discovery, trust, or I/O claim |
| local acquire | chosen `LocalPath`; filesystem adapter | observed source metadata and `LocalMaterial` | missing/unsupported source fails before later transitions |
| HTTP acquire | request, retry, admission, resume, and pacing policy | per-attempt transfer/resume/wait evidence and staged material | each attempt admitted separately; complete validated stage precedes persistence |
| HTTP inspect | `RemoteUrl`; adapter-owned HEAD policy | status, declared content length, requested/final URL, method, and per-attempt evidence | HEAD only; every received final status is an observation; no body copy, destination, or GET fallback |
| identity/descriptor verify | caller expectation or explicit identity pass-through | typed expected/observed digest and size facts | factual mismatch fails without applying a target |
| identity/archive prepare | preparation Need and exclusive workspace where required | prepared material/tree and observed preparation evidence | final destination remains untouched |
| local apply | typed create/replace intent and target | receipt plus observed files/directories/bytes/placement | staged publication with an explicit single-target commit boundary |
| local forget | exact caller-authorized target | removed/no-op apply evidence | direct idempotent removal; no ownership or uninstall claim |
| memory remember | applied result | receipt and evidence carried in `Remembered` | process-local only; no durability claim |
| local inspect | `LocalTarget`; local adapter owns observed facts | entry observation plus no-follow method evidence | read-only; `NotFound` is `Missing`, other I/O failures remain errors |
| local reconcile | caller-owned `LocalExpectation` | preserved inspect evidence, expected/observed evidence, and typed classification | pure comparison; no repair, adoption, deletion, or persistence |

Resource semantics remain bounded and cross through typed anti-corruption mappings:

| Resource context | Owns | Must not be interpreted as |
| --- | --- | --- |
| Filesystem | entry kind, no-follow observation, staging, publication | package ownership or durable state |
| HTTP | representation, validator, ranges, attempts, pacing | artifact identity or publisher trust |
| Artifact identity | digest algorithm, raw digest, exact byte size | authorization or provenance |
| Archive | entries, decoded/materialized limits, guarded scratch | final publication |
| Trust/provenance | external signature, delegation, attestation facts | generic core identity or automatic admission |
| Durable state | adapter-specific revision, commit, and recovery laws | a mandatory Pulith aggregate |

`Evidence` proves one behavior result; it is not automatically a DDD domain event. A software
artifact repository is likewise not a DDD `Repository` unless it rehydrates a demonstrated
aggregate. Pulith defines no universal `Installation`, global repository, registry, or transaction
manager.

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
| `LocalTarget -> Inspected` | read-only, no-follow local entry observation with method evidence |
| `RemoteUrl -> Inspected` | sync `UreqInspect` or async `ReqwestInspect` HEAD observation with redirect and attempt evidence |
| `Inspected -> Reconciled` | pure comparison against caller-owned local expected state |

Async execution is concrete for HTTP acquisition through `AsyncAcquireNode`/`ReqwestAcquire` and
HTTP inspection through `AsyncInspectNode`/`ReqwestInspect`; the other transitions currently expose synchronous behavior laws only. Pulith does
not provide source discovery, dependency solving, a durable installation database, multi-target
transactions, automatic repair, or system package-manager integration. Those require demonstrated
callers and explicit storage/rollback laws; they are not hidden behind the existing state names.

`ArtifactDescriptor<A>` identifies one exact raw representation by digest and byte size, independent
of whether it came from a local path, `ureq`, or `reqwest`. Descriptor equality proves only that the
material matches the supplied expectation. It does not authenticate who supplied that expectation,
authorize a publisher, or establish provenance; those remain separate caller-owned policy/trust
behaviors until a concrete adapter justifies them.

`RemoteUrl::parse` reports resource-level `RemoteUrlError`; acquisition explicitly wraps that error
as `AcquireError::RemoteUrl` rather than making URL validation an acquisition behavior.

## Features

```text
default = local

network:
  net
  ureq -> net + local
  reqwest -> net + local + tokio

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
- HTTP inspection issues HEAD only, admits every retry separately, records requested/final URLs,
  treats every received final status as an observation, and never falls back to GET or copies a body;
- an HTTP inspection `declared_content_length` is response metadata, not observed bytes, an artifact
  descriptor, validator continuity, provenance, or trust evidence;
- HTTP partial bytes are recombined only with a strong validator, a terminal `Content-Range`, and an
  observed body length matching that range; weak or conflicting validators are rejected;
- network `max_bytes` is checked before body pacing and persistence;
- request admission and decoded-body pacing are separate shared resources;
- decoded-body pacing does not control kernel, TLS, or HTTP flow-control timing;
- local path safety assumes trusted parent directories, not a hostile concurrent filesystem;
- local inspection uses `symlink_metadata`, treats `NotFound` as `Missing`, reports dangling
  symlinks without following them, and performs no mutation;
- local reconciliation compares caller-owned expectation with an observation and returns only a
  classification plus evidence; it never repairs, adopts, deletes, or persists state;
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
