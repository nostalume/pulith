# pulith

Pulith is a single-root Rust crate ecosystem for composing typed external-resource management
behaviors without a hidden package-manager or implementation-specific policy layer.

> **Status:** pre-release. Local package verification does not publish a registry artifact, and no
> release is implied by the `0.1.0` manifest version.

```text
Materialize -> Acquired -> Applied
                        -> Verified -> Applied
                        -> Prepared -> Applied
                        -> Verified -> Prepared -> Applied
Forget -----------------------------> Applied
LocalTarget -> Inspected -> Reconciled
RemoteUrl  --> Inspected
```

Each behavior passes policy `Need` as a trait type parameter where required and declares associated
`Error` and `Output` contracts. Canonical outputs are open adapter-attested records: callers choose
the adapter, decide whether to trust its evidence, and compose concrete effects.

## Scope

| Module | Behavior |
| --- | --- |
| `application` | Typed `Materialize` and `Forget` requests |
| `behavior` | Transition traits and state nodes |
| `local` | Local acquire, staged apply, inspect, and pure reconcile |
| `hash` | Typed descriptor verification plus opt-in exact local artifact inspection/reconciliation |
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

A behavior law declares its input, policy `Need` where required, output-carried `Evidence`, `Error`,
`Output`, effect boundary, and failure law. Resource semantics define what those terms mean for a filesystem,
HTTP representation, archive, digest, trust system, durable store, or another external resource. An
adapter implements one demonstrated behavior/resource intersection. Callers retain application
identity, desired state, trust/admission policy, durable aggregates, orchestration, and rollback or
retention policy.

The behavior vocabulary is not a universal lifecycle. Current and candidate families are:

| Family | Behaviors | Boundary |
| --- | --- | --- |
| Materialization | acquire, optional verify, optional prepare | transfer, factual proof, and transformation stay separate |
| Mutation | apply, forget | explicit target effect; `Forget` does not claim ownership |
| Observation | inspect | read-only resource facts; filesystem absence and HTTP status retain resource-specific meaning |
| Convergence | reconcile | caller expectation plus observation; never repairs or adopts |
| Future gated families | admit, recover, activate, prune | require a demonstrated authority and adapter before public vocabulary |

Concrete behavior contracts currently admitted to the public surface are:

| Behavior | Need / authority | Evidence and output | Effect and failure law |
| --- | --- | --- | --- |
| local acquire | `Materialize<_, LocalPath, _>`; filesystem adapter | source path and `LocalMaterial` classification | a missing source fails acquisition; unsupported entry types are rejected before publication by downstream concrete behavior |
| HTTP acquire | `RemoteSource` URL/policy plus adapter-owned admission and pacing resources | per-attempt transfer/resume/wait evidence and RAII-owned staged material | each attempt is admitted separately; acquire never publishes `Materialize.target`, and dropping the state removes its stage |
| HTTP inspect | `RemoteUrl`; adapter-owned HEAD policy | status, declared content length, requested/final URL, and per-attempt evidence | HEAD only; every received final status is an observation; no body copy, destination, or GET fallback |
| digest/descriptor verify | caller-supplied digest or exact descriptor | typed expected/observed digest and size facts | factual mismatch fails without applying a target |
| archive prepare | `ArchivePolicy` and exclusive workspace | prepared tree and observed extraction evidence | final destination remains untouched |
| local apply | `MaterializeMode` and exact target | typed `Applied` result plus files/directories/bytes/placement evidence | staged publication with an explicit single-target commit boundary |
| local forget | exact caller-authorized target | removed/no-op apply evidence | direct idempotent removal; no ownership or uninstall claim |
| local inspect | `LocalTarget`; local adapter owns observed facts | no-follow entry observation and evidence | read-only; `NotFound` is `Missing`, other I/O failures remain errors |
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

Canonical states are public records so external adapters can enter and continue the same typed
chain as built-in adapters. Their evidence is the selected adapter's attestation, not provenance,
authorization, or an unforgeable capability. Invariant-bearing resource outputs may still restrict
construction independently. Evidence is not automatically a DDD domain event. A software artifact
repository is likewise not a DDD `Repository` unless it rehydrates a demonstrated
aggregate. Pulith defines no universal `Installation`, global repository, registry, or transaction
manager.

Openness is boundary-specific rather than universal. `ArchiveTree` is crate-constructed and exposes
its prepared root by shared reference: callers cannot fabricate one from an arbitrary path or write
its private root directly. It remains a replaceable value inside open canonical records, not a
permanent input/evidence binding, immutable workspace, hostile-filesystem capability, or
unforgeable proof. Requests, policies, canonical records, and adapter evidence remain open where
caller authority and external composition require it.

This pre-release cutover intentionally replaces canonical-state constructors and read-only
accessors with ordinary record literals and field access. Callers should use `.input`, `.material`,
`.prepared`, `.observation`, `.reconciliation`, and `.evidence` directly; no compatibility methods
or aliases are retained.

## Current maturity

The state graph is a composition vocabulary, not a claim that Pulith is already a complete package
manager. The concrete path currently supplied by this crate is:

| Transition | Concrete behavior today |
| --- | --- |
| `Materialize -> Acquired` | local material or staged HTTP download |
| `Acquired -> Verified` | typed digest or exact digest-plus-size descriptor verification |
| `Acquired/Verified -> Prepared` | guarded ZIP/TAR extraction when the caller needs transformation |
| `Acquired/Verified/Prepared -> Applied` | staged local file/tree publication according to `MaterializeMode` |
| `Forget -> Applied` | direct idempotent local target removal; no artificial source acquisition |
| `LocalTarget -> Inspected` | cheap metadata observation, or opt-in hash-backed full-read artifact observation |
| `RemoteUrl -> Inspected` | sync `SyncHttpInspect` or async `AsyncHttpInspect` HEAD observation with redirect and attempt evidence |
| `Inspected -> Reconciled` | pure metadata or exact-descriptor comparison against caller-owned expected state |

Async execution is concrete for HTTP acquisition through `AsyncAcquire`/`AsyncHttpAcquire` and
HTTP inspection through `AsyncInspect`/`AsyncHttpInspect`; the other transitions currently expose synchronous behavior laws only. Pulith does
not provide source discovery, dependency solving, a durable installation database, multi-target
transactions, automatic repair, or system package-manager integration. Those require demonstrated
callers and explicit storage/rollback laws; they are not hidden behind the existing state names.

`ArtifactDescriptor<A>` identifies one exact raw representation by digest and byte size, independent
of whether it came from a local path, synchronous HTTP, or asynchronous HTTP. Descriptor equality proves only that the
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
  http-sync -> net + local
  http-async -> net + local + tokio

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

### Feature selections

Features are additive capabilities, not mutually exclusive modes. Select only the concrete behavior
or shared vocabulary a consumer needs; Pulith never chooses a global transport, runtime, digest, or
archive adapter.

| Consumer need | Selection |
| --- | --- |
| local materialization and observation | default features, or `default-features = false, features = ["local"]` |
| network URL/policy/attempt vocabulary only | `default-features = false, features = ["net"]` |
| synchronous HTTP HEAD/acquire | `default-features = false, features = ["http-sync"]` |
| Tokio asynchronous HTTP HEAD/acquire | `default-features = false, features = ["http-async"]` |
| typed digest/descriptor vocabulary only | `default-features = false, features = ["hash"]` |
| exact local BLAKE3 or SHA-256 artifact observation | `default-features = false, features = ["local", "blake3"]` or `features = ["local", "sha2"]` |
| ZIP or plain TAR preparation | `default-features = false, features = ["zip"]` or `features = ["tar"]` |
| gzip, xz, or zstd TAR preparation | `default-features = false, features = ["gzip"]`, `features = ["xz"]`, or `features = ["zstd"]` |

`--all-features` is an integration-validation profile, not a consumer-facing `full` feature. A
consumer may combine an HTTP adapter, digest algorithm, and archive adapter in one dependency
declaration; it must select each concrete adapter in its own composition code.

## Use

```toml
[dependencies]
pulith = { path = "../pulith", features = ["http-sync", "blake3", "zip"] }
```

Construct a `Materialize` request after the caller has selected a source, then compose only the concrete acquire, optional verify/prepare, and apply behaviors the request needs. Use `Forget` for a direct target-only removal. There is no global context, plugin registry, or hidden workflow policy.

HTTP sources are constructed as `RemoteSource::new(url)`. Acquisition returns
`LocalMaterial::StagedFile`; it does not accept a second destination. `LocalMaterial::File` and
`Directory` remain caller-owned, while dropping `StagedFile` removes the adapter-owned stage. A
caller that wants a durable cache composes `LocalApply` with that cache path as the one target.

## Guarantees and limits

- final destination writes are staged before publication where the concrete behavior supports it;
- for local regular files, `MaterializeMode::CreateNew` means the expected predecessor is missing;
  the final no-clobber persist is authoritative, and an early or late existing target returns
  `ApplyWouldOverwrite` without changing that target;
- that conditional-file law does not cover directory publication, `ReplaceOrCreate`,
  `Forget`, or digest-based compare-and-swap;
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
- network `max_bytes` is checked before body pacing and staged writes;
- HTTP acquire never creates or publishes the final target or its parent; only apply has that authority;
- caller-owned local sources and resume partial files are never removed implicitly;
- request admission and decoded-body pacing are separate shared resources;
- decoded-body pacing does not control kernel, TLS, or HTTP flow-control timing;
- local path safety assumes trusted parent directories and does not claim a hostile concurrent-filesystem sandbox;
- required runtime evidence covers Windows and Linux; macOS is currently best-effort and unverified,
  and does not gate phase admission;
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
