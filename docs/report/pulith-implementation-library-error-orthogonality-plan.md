# Pulith Implementation Library, Error, Feature, and Orthogonality Plan

## Status

Migration analysis only. No Rust implementation changes are authorized by this document.

This document amends:

```text
docs/report/pulith-behavior-semantic-migration-plan.md
docs/report/pulith-behavior-semantic-migration-execution-report.md
```

New rule from review:

```text
Before migrating any concrete implementation, search mature Cargo crates.
If mature crates already implement the mechanism, mark old Pulith code as delete/adapter only.
Every concrete implementation must map accurately to one behavior semantic.
Errors must be classified orthogonally; do not create one large PulithError enum.
Cargo features control implementation families selectively.
Implementation semantics must be orthogonal.
```

## Crates.io survey evidence

The search was run with:

```text
cargo search --registry crates-io --limit 5 <crate/query>
cargo info --registry crates-io <crate>
```

The local Cargo config replaces crates.io with `ustc`, so explicit `--registry crates-io` was required for search.

### Acquire / download candidates

| Crate | Evidence from search/info | Semantic fit | Pulith disposition |
|---|---|---|---|
| `reqwest` | `reqwest = "0.13.4"`, higher level HTTP client, MIT/Apache-2.0, rust-version 1.85, docs/repo present, features include `blocking`, `gzip`, `brotli`, `deflate`, `http2`, `http3`, TLS families | `Acquire` mechanism for HTTP(S) chosen source | Use as optional `net-reqwest` implementation. Delete custom HTTP client/retry/stream code unless Pulith semantics require extra evidence. |
| `ureq` | `ureq = "3.3.0"`, simple safe HTTP client, MIT/Apache-2.0, rust-version 1.85, features `rustls`, `gzip`, `brotli`, `json` | sync/small `Acquire` mechanism | Candidate for lightweight `net-ureq` implementation. Do not build a custom blocking HTTP layer. |
| `object_store` | `object_store = "0.14.0"`, generic object store interface for AWS/GCS/Azure/local, MIT/Apache-2.0, rust-version 1.85 | remote/local object acquisition abstraction | Use only if Pulith needs cloud/object stores as source offers. Otherwise avoid broad dependency. |

Delete/mark old code:

```text
HttpClient unless it encodes Pulith evidence semantics
ReqwestClient wrapper if only forwarding to reqwest
BatchFetcher / SegmentedFetcher / ResumableFetcher / ConditionalFetcher unless behavior law requires them
TokenBucket / retry/backoff custom code if reqwest/ureq/tower ecosystem covers it and Pulith does not expose semantic evidence
```

Keep only Pulith-owned facts:

```text
Chosen source
Acquired material handle
transfer evidence required by Receipt
failure category
```

### Verify / digest candidates

| Crate | Evidence from search/info | Semantic fit | Pulith disposition |
|---|---|---|---|
| `blake3` | `blake3 = "1.8.5"`, official BLAKE3 implementation, docs/repo present, features `std`, `mmap`, `rayon`, `serde`, `zeroize` | `Verify` digest implementation | Use; delete custom BLAKE3 hashing. |
| `sha2` | `sha2 = "0.11.0"`, RustCrypto SHA-2 implementation, MIT/Apache-2.0, rust-version 1.85 | `Verify` SHA-256/SHA-512 implementation | Use; delete custom SHA-2 hashing. |

Delete/mark old code:

```text
pulith-verify reader/hasher wrappers
custom checksum pipelines that just wrap sha2/blake3
any FetchOptions checksum implementation that duplicates Verify semantics
```

Keep only Pulith-owned facts:

```text
VerifyNeed
expected digest
observed digest
algorithm label
verification evidence
```

### Prepare / archive and decompression candidates

| Crate | Evidence from search/info | Semantic fit | Pulith disposition |
|---|---|---|---|
| `zip` | search shows `zip = "9.0.0-pre2"`; current workspace already uses `zip = "8.4.0"`; docs/repo present; broad compression features | `Prepare` zip read/extract mechanism | Use library API; Pulith owns Zip Slip/path-safety evidence, not ZIP parser. Avoid custom zip parsing. |
| `safe_unzip` | search result: secure zip extraction, prevents Zip Slip and Zip Bombs | possible `Prepare` safe zip extraction mechanism | Investigate before preserving custom secure unzip. If mature enough, delete custom zip safety extraction except evidence mapping. |
| `tar` | `tar = "0.4.46"`, Rust TAR reader/writer, MIT/Apache-2.0, rust-version 1.63, streaming design | `Prepare` tar mechanism | Use; Pulith owns policy/evidence, not TAR parser. |
| `async-compression` | `async-compression = "0.4.42"`, adaptors for modern async IO, features `gzip`, `xz`, `zstd`, `tokio`, `futures-io` | async decompression mechanism for Acquire/Prepare | Use only if async prepare/acquire is needed. Avoid custom stream transform layers. |

Delete/mark old code:

```text
Decoder wrappers if only map extension to crate decoder
EntrySource/PendingEntry if only duplicate archive crate iteration
custom gzip/deflate/brotli decoders in fetch unless they carry behavior evidence
custom archive parsing
```

Keep only Pulith-owned facts:

```text
prepared root/handle
archive kind observed
entries materialized, if needed by evidence
sanitized path decisions
permission decisions, if semantically required
```

### Apply / filesystem candidates

| Crate | Evidence from search/info | Semantic fit | Pulith disposition |
|---|---|---|---|
| `fs_extra` | `fs_extra = "1.3.0"`, recursive copy folders with progress/info, MIT, docs/repo present | directory copy mechanism for `Apply` | Candidate replacement for custom `copy_dir_all`; keep only if semantics need custom atomicity/evidence. |
| `tempfile` | `tempfile = "3.27.0"`, temp files/dirs, MIT/Apache-2.0, rust-version 1.63 | staging/temp mechanism for `Apply`/`Prepare`/tests | Use; avoid custom temp path generation for non-evidence internals. |
| std `fs` | already mature for basic copy/remove/create | simple local apply mechanism | Prefer std for minimal local behavior; do not build fs abstraction unless semantics need it. |

Delete/mark old code:

```text
copy_dir_all helper if fs_extra is accepted
custom temp directory naming in tests/implementation where tempfile suffices
Workspace/Transaction wrappers if they only repackage std/tempfile/fs_extra without Pulith behavior evidence
```

Keep only Pulith-owned facts:

```text
created/replaced/removed target
rollback snapshot evidence, when implemented
mutation failure category
```

### Remember / persistence candidates

This was not fully searched in this slice, but rule is fixed:

```text
Before migrating store/state persistence, search crates for embedded persistence, JSON/schema, directory layout, locking, and file locks.
```

Likely candidate families to search before implementation:

```text
serde_json / toml / postcard for serialization
fs4 / fd-lock / fslock for locking
jiff/time for timestamps if needed
camino for UTF-8 paths if public path display becomes semantic
```

Until that search is complete, do not port `pulith-store`/`pulith-state` mechanics.

## Implementation adoption depth

Candidate crate selection is not enough. Each implementation migration must decide how deeply Pulith adopts the external crate.

Adoption levels:

| Level | Meaning | Public API exposure | When allowed |
|---|---|---|---|
| L0 delete | Mature crate fully replaces old Pulith mechanism | none | Old code had no Pulith-owned behavior/evidence. |
| L1 internal use | Pulith calls crate internally | no crate types in public API | Default for most mechanisms. |
| L2 configured adapter | Pulith exposes semantic config and maps it to crate config | only Pulith config types | Caller needs behavior-level policy. |
| L3 escape hatch | Pulith accepts a prebuilt client/handle | crate type appears behind feature module only | Caller truly needs transport tuning not expressible semantically. |
| L4 re-export | Pulith re-exports crate types | public API coupled to crate | Avoid unless Pulith is intentionally a facade for that crate. |

Default rule:

```text
Use L1 or L2.
Avoid L3.
Reject L4.
```

### Reqwest adoption depth

`reqwest` should not become the Pulith public API.

Allowed first implementation:

```text
L1 internal use for HTTP(S) Acquire.
```

Feature:

```toml
net-reqwest = ["dep:reqwest"]
```

Behavior mapping:

```text
Chosen { source: Source::Url } -> Acquired<HttpMaterial>
```

Pulith-owned evidence:

```text
chosen URL
final URL after redirects, if redirects are allowed and observable
HTTP status
content length when known
downloaded local/material handle
transport failure category
```

Do not expose by default:

```text
reqwest::Client
reqwest::RequestBuilder
reqwest::Response
reqwest middleware stack
headers as arbitrary reqwest HeaderMap
TLS backend selection as raw reqwest config
```

Expose only semantic acquire policy:

```text
AcquireNeed / NetNeed:
  offline: bool
  redirect: RedirectPolicy
  timeout: Option<Duration>
  max_bytes: Option<u64>
  user_agent: Option<String>, only if product semantics require it
  auth: none for now; credentials must be separate later
```

Do not expose low-level download configuration until there is a behavior reason:

```text
connection pool tuning
HTTP/2 toggles
proxy configuration
TLS root store selection
cookie store
retry backoff internals
chunk size
```

If these are needed later, they belong to an implementation-local config behind `net-reqwest`, not to core behavior semantics. That is L3 and must not leak into `App`, `Need`, or `Source`.

Recommended split:

```text
NetNeed          # semantic policy, public if needed by behavior
ReqwestAcquire   # implementation type behind net-reqwest
ReqwestConfig    # feature-local implementation config, not core semantic state
```

`ReqwestConfig` may contain reqwest-shaped knobs, but it must not alter behavior laws. It only chooses how to perform `Acquire`.

### Ureq adoption depth

`ureq` is a candidate for sync HTTP Acquire.

Allowed adoption:

```text
L1 internal use, possibly L2 semantic config mapping.
```

Do not support both `reqwest` and `ureq` in the same first implementation slice. They are alternative Acquire implementations. Adding both without a selection need creates implementation duplication.

Decision rule:

```text
If Pulith's behavior API is sync-first, use ureq first.
If async Acquire is required by surrounding runtime, use reqwest first.
If both are needed, expose two feature-gated implementation types, not one behavior with hidden runtime switching.
```

### Async behavior handling

Async is an implementation modality, not a different domain behavior.

The semantic morphism remains:

```text
Chosen source -> Acquired material
```

Do not mix sync and async in one trait by default.

Use separate behavior traits when async is required:

```text
Acquire       # sync implementation trait
AsyncAcquire  # async implementation trait, feature-gated
```

Rules:

```text
Do not force async_trait on the core behavior model unless async implementations are active.
Do not make local/path/hash/archive sync implementations depend on tokio.
Do not expose tokio runtime handles as domain semantics.
Async feature enables async implementation types, not different behavior states.
```

Possible feature shape:

```toml
async = []
net-reqwest = ["async", "dep:reqwest", "dep:tokio"]
```

If using Rust stable async traits is insufficient for object-safe dynamic dispatch, prefer static generics first. Introduce boxed futures or `async-trait` only after runtime polymorphism is proven necessary.

Semantic evidence must be the same between sync and async implementations:

```text
sync ureq Acquire and async reqwest Acquire both produce Acquired material + Acquire evidence.
They differ only in mechanism and error source.
```

### Archive adoption depth

`zip`, `tar`, and decompression crates should be L1 internal mechanisms.

Pulith should expose preparation policy/evidence, not archive crate types.

Public/semantic:

```text
PrepareNeed::Identity / File / Directory / Archive, if Archive is introduced
ArchiveSafetyNeed, only if safety policy is caller-visible
Preparation evidence: format, prepared root, sanitized entries, rejected entries
```

Private/feature-local:

```text
zip::ZipArchive
tar::Archive
async_compression decoders
```

Do not expose parser knobs unless they are semantic safety policy.

### Hash adoption depth

`blake3` and `sha2` should be L1 internal mechanisms for Verify.

Public/semantic:

```text
VerifyNeed::Digest { algorithm, value }
Verify evidence: expected, observed, algorithm, match/mismatch
```

Private/feature-local:

```text
blake3::Hasher
sha2::{Sha256, Sha512, Digest}
hex decoder details
```

No custom hasher abstraction unless two conditions hold:

```text
multiple digest implementations are enabled at the same time
runtime digest choice is required by declared need
```

### Filesystem adoption depth

`std::fs`, `fs_extra`, and `tempfile` are mechanisms for Apply/Prepare staging.

Public/semantic:

```text
target path
operation mode
created/replaced/removed facts
rollback/backup need when supported
```

Private/feature-local:

```text
recursive copy implementation
temporary directory creation
atomic rename/copy mechanics
```

Expose progress only if there is a semantic observation behavior for progress. Otherwise progress callbacks are mechanism noise.

### Implementation decision record template

Every implementation migration must add a short record before code:

```text
Behavior: Acquire / Verify / Prepare / Apply / Remember / Inspect / Repair / Forget
External crate: name/version/features searched
Adoption depth: L0/L1/L2/L3/L4
Public semantic config: yes/no, fields
Private implementation config: yes/no, fields
Async: no / trait split / feature-gated runtime
Evidence produced: fields
Error category: behavior-specific error
Old Pulith code disposition: delete / adapter / migrate evidence only
Orthogonality proof: forbidden behaviors not performed
```

## Feature control rules

Cargo features select implementation families, not workflow concepts.

Feature design must be orthogonal. A feature may enable exactly one axis:

```text
execution modality: sync / async
behavior family implementation: net / hash / archive / persist / fs
mechanism backend: reqwest / ureq / blake3 / sha2 / zip / tar / json
```

It must not bundle unrelated axes:

```text
bad: fetch = source selection + HTTP + retry + checksum + cache + receipt
bad: full = every implementation and every runtime
bad: archive = zip + tar + xz + zstd + async runtime + filesystem staging
```

### Recommended feature surface

Feature names should be short and readable. Prefer one noun for the behavior family and one noun for the backend.

```toml
[features]
default = ["local", "sync"]

# execution modality
sync = []
async = []

# local/source implementation
local = []

# Acquire implementations
net = []
reqwest = ["async", "dep:reqwest", "dep:tokio"]
ureq = ["sync", "dep:ureq"]
object = ["async", "dep:object_store"]

# Verify implementations
hash = []
blake3 = ["hash", "dep:blake3"]
sha2 = ["hash", "dep:sha2"]

# Prepare implementations
archive = []
zip = ["archive", "dep:zip"]
tar = ["archive", "dep:tar"]
compress = ["dep:async-compression"]

# Apply / Remember mechanisms
fs-extra = ["dep:fs_extra"]
json = ["dep:serde", "dep:serde_json"]
```

Naming notes:

```text
Use `reqwest`, not `net-reqwest`, if the feature only means the reqwest backend.
Use `net` only as a dependency-free grouping/marker if docs need it.
Use `async` only for async execution support; do not make `async` imply reqwest.
Use `archive`, `hash`, and `net` as dependency-free family markers only.
Do not use old crate names: pulith-fetch, pulith-archive, pulith-store.
```

Rules:

```text
No feature may change the meaning of a behavior.
A feature only makes a behavior implementation available.
Behavior traits and semantic states compile without optional implementation features.
Feature names must not mirror old crates unless the name describes a real implementation family.
Features should be additive.
Default features should stay minimal.
Feature combinations must not silently select one of multiple backends.
If two backends are enabled, caller composes/selects implementation explicitly.
```

Orthogonal examples:

```text
Good: blake3 enables BLAKE3 Verify implementation.
Good: sha2 enables SHA-2 Verify implementation.
Good: zip enables ZIP Prepare implementation.
Good: reqwest enables async HTTP Acquire implementation.
Good: ureq enables sync HTTP Acquire implementation.
Bad: verify enables a generic old pulith-verify module.
Bad: fetch bundles source selection, HTTP, cache, retry, and receipts.
Bad: archive-zip implies target staging or persistence.
```

### Sync/async feature rules

Sync and async are execution modalities, not behavior semantics.

```text
Acquire and AsyncAcquire have the same behavior law.
Prepare and AsyncPrepare have the same behavior law.
Remember and AsyncRemember have the same behavior law.
```

Use separate traits when async is active:

```text
sync feature  -> Acquire / Verify / Prepare / Apply / Remember
async feature -> AsyncAcquire / AsyncVerify / AsyncPrepare / AsyncApply / AsyncRemember only where needed
```

Do not create async variants just because a feature exists. Add async trait only when an implementation truly needs async IO.

Implementation mapping:

```text
ureq       -> sync Acquire
reqwest    -> async Acquire by default
blake3     -> sync Verify; async wrapper unnecessary unless reading async stream
sha2       -> sync Verify; async wrapper unnecessary unless reading async stream
zip/tar    -> sync Prepare unless async IO is required
object     -> async Acquire/Remember candidate
json       -> sync Remember by default
```

Runtime rule:

```text
No core behavior type may require tokio.
Tokio appears only in async implementation modules/features.
No function creates a hidden global runtime.
Callers that choose async own runtime provisioning.
```

## Resource control rules

Concrete implementations will own scarce resources. Resource control must be explicit, orthogonal, and composable.

Pulith must distinguish:

```text
shared resource control: one budget/limiter/client reused across many operations
exclusive resource control: one operation owns a temporary path, lock, file handle, or mutation slot
```

### Shared controls

Use shared controls for resources where parallel operations compete for global capacity:

| Resource | Shared control | Applies to | Rule |
|---|---|---|---|
| network concurrency | shared semaphore / client pool | `Acquire` | One limiter per engine/session, not one per download. |
| bandwidth | shared rate limiter | `Acquire` | Do not create per-resource token buckets that multiply total bandwidth. |
| HTTP client | shared reqwest client | `Acquire` | Reuse client; do not build a new client per resource. |
| CPU hashing | shared CPU/thread budget | `Verify` | Avoid per-resource rayon pools. |
| decompression CPU | shared CPU/thread budget | `Prepare` | Avoid archive implementation spawning uncontrolled workers. |
| temp root quota | shared temp/staging allocator | `Prepare`/`Apply` | Avoid every implementation inventing its own temp policy. |
| persistence lock table | shared lock manager | `Remember`/`Inspect`/`Forget` | Avoid separate lock domains for store/state/receipt. |

Shared controls are implementation context, not semantic state.

Public semantics may express a need:

```text
offline
max bytes
retain evidence
rollback required
```

But shared resource controllers remain behind implementation types:

```text
ReqwestAcquire { client, limits }
HashVerify { cpu }
ArchivePrepare { temp, cpu }
JsonRemember { locks }
```

### Exclusive controls

Use exclusive controls for resources that must be owned by one operation:

| Resource | Exclusive owner | Behavior boundary |
|---|---|---|
| target mutation path | `Apply` operation | `Apply` only |
| activation name/link | `Apply` or `Forget` operation | mutation boundary |
| staging directory | one `Prepare`/`Apply` operation | released/remembered after receipt |
| lock file for a remembered fact | one `Remember`/`Forget` operation | persistence boundary |
| rollback snapshot | one `Apply` operation | receipt/repair evidence |

Exclusive controls must not be hidden inside earlier behaviors:

```text
Acquire cannot lock target path.
Verify cannot reserve staging path.
Prepare cannot mutate final target.
Remember cannot acquire network bandwidth.
Inspect cannot take mutation lock unless it is explicitly a repair/forget precondition.
```

### Resource-control adoption depth

Resource control has its own adoption-depth rule:

| Level | Meaning | Allowed |
|---|---|---|
| R0 none | implementation uses no scarce shared control | local small operations/tests |
| R1 internal shared control | implementation owns/reuses one limiter/client internally | default for reqwest client, hash CPU budget |
| R2 injected shared control | caller/engine provides shared limiter/client | when many operations must coordinate globally |
| R3 global singleton | process-global limiter/client | avoid; only if explicitly required and testable |

Default:

```text
Use R1 for single-engine use.
Use R2 when orchestration runs multiple resources concurrently.
Reject R3.
```

### Resource control in decision records

Every implementation decision record must now include:

```text
Resource controls: none / shared / exclusive
Shared controls: client, semaphore, rate limiter, CPU budget, temp quota, lock manager
Exclusive controls: target lock, staging dir, receipt lock, rollback snapshot
Contention rule: how multiple resources avoid multiplying resource use
Ownership lifetime: who creates, shares, and releases the control
```

### Resource-control tests

Each implementation with resource control needs at least one behavior test or structural test:

```text
shared client/limiter is reused across multiple resources
per-resource operation does not create independent unlimited limiter
exclusive target/staging lock prevents concurrent mutation conflict
resource control does not leak into semantic state/evidence except as observable facts
```

## Error taxonomy rules

Do not use one large enum for all Pulith behavior failures.

Error categories must be orthogonal to behavior semantics:

```text
DeclareError    invalid intent, empty item, invalid target expression
OfferError      no source offers, invalid offer expression, policy excludes all offers
SelectError     no chosen candidate, selection policy failure
AcquireError    unsupported source mechanism, transfer failure, missing local source, cache read failure
VerifyError     missing required verifier, digest mismatch, signature/trust failure
PrepareError    unsupported material kind, unsafe path, unsupported archive format, shape mismatch
ApplyError      create would overwrite, replace missing target, fs mutation failure, activation failure
RememberError   persistence unavailable, serialization failure, retention failure
InspectError    remembered fact missing, live observation failure, inconsistent evidence
RepairError     cannot plan repair, insufficient evidence, repair mutation failed if repair applies
ForgetError     retention policy blocks deletion, remove target failure, tombstone failure
```

Mechanism errors are sources, not top-level semantic categories:

```text
std::io::Error
reqwest::Error
ureq::Error
zip::result::ZipError
tar extraction error
serde_json::Error
```

They should be wrapped at the behavior boundary:

```text
AcquireError::Transfer { source: ... }
PrepareError::Archive { source: ... }
ApplyError::Fs { source: std::io::Error }
RememberError::Serialize { source: serde_json::Error }
```

Top-level orchestration may have a small sum type only for composition:

```text
RunError::Declare(DeclareError)
RunError::Offer(OfferError)
RunError::Select(SelectError)
RunError::Acquire(AcquireError)
RunError::Verify(VerifyError)
RunError::Prepare(PrepareError)
RunError::Apply(ApplyError)
RunError::Remember(RememberError)
```

But this is not a large domain error enum. It is a transparent behavior-stage wrapper.

Current active `PulithError` is therefore transitional. Before next implementation migration, split it by behavior.

## Orthogonality rules

Every implementation must own exactly one primary behavior semantic.

| Implementation type | Allowed primary semantic | Forbidden semantic coupling |
|---|---|---|
| HTTP downloader | `Acquire` | cannot select source, verify digest, unpack archive, remember lifecycle |
| local path reader | `Acquire` | cannot prepare archive or mutate target |
| digest checker | `Verify` | cannot fetch, select, prepare, apply |
| zip extractor | `Prepare` | cannot choose source, download, apply target, persist lifecycle |
| tar extractor | `Prepare` | cannot verify unless Verify explicitly composes before/after |
| copy/link applier | `Apply` | cannot acquire source or remember lifecycle truth |
| receipt writer | `Remember` | cannot mutate target or create facts not produced by prior behavior |
| state inspector | `Inspect` | cannot repair/mutate/delete |
| repair planner | `Repair` | cannot hide Apply or Forget side effects |
| remover | `Forget` | cannot erase required evidence without a receipt/tombstone |

If a candidate implementation crosses behavior boundaries, split it.

Examples:

```text
MultiSourceFetcher = Select + Acquire mixed -> split.
FetchReceipt passed into Store = Acquire evidence + Remember glue -> split.
ArchiveReport plus root passed into Install = Prepare evidence + Apply glue -> split.
InstallFlow<S> = Apply choreography + Remember/Repair evidence -> internalize/split.
StoreReady used by Apply = Remember mechanism invading Apply -> split.
```

## Revised concrete migration order

Do not pause migration for a complete error-taxonomy rewrite. Error categories should be split gradually as each behavior implementation lands, while preserving high cohesion and low coupling.

Current order:

```text
sync/async trait split
orthogonal feature surface
resource-control design as an implementation principle, not a glue layer
hash Verify implementation
gradual behavior-specific error extraction as implementation grows
```

Do not create a resource-control middleware layer before a concrete implementation needs shared or exclusive controls. Resource control is a design invariant for implementations, not a new public orchestration object.

### Gate A — Library survey per behavior

Before each behavior implementation:

```text
cargo search/cargo info mature crates
classify old Pulith code as delete/adapter/semantic evidence
record feature names and optional deps
```

### Gate B — Error taxonomy compatibility

Before adding net/archive/persist implementations, split errors where the behavior boundary needs it. For the hash Verify slice, the transitional `PulithError` may remain if the new errors are cohesive and local to Verify.

Rules:

```text
Do not block implementation on a full error rewrite.
Do not grow unrelated error variants in random modules.
Keep each new error attached to the behavior being implemented.
Extract behavior-specific error enums when multiple mechanisms share the same behavior boundary.
```

### Gate C — Orthogonality test design

For each implementation:

```text
prove it only changes the semantic state owned by its behavior
prove forbidden composition is not hidden in implementation
prove evidence does not smuggle caller glue
```

### First concrete slice after this amendment

Preferred next slice:

```text
sync/async trait split + feature surface + hash Verify implementation
```

Why:

```text
Verify has clear behavior semantics.
Mature crates exist: blake3 and sha2.
Old pulith-verify is already marked wheel repetition.
Feature control is simple: blake3/sha2 under hash.
Orthogonality is easy to test: Verify cannot acquire/prepare/apply.
No async implementation is needed for file hashing yet; sync Verify remains enough.
```

Do not migrate HTTP/download or archive extraction before this trait/feature foundation is in place.

### Hash Verify decision record

Implementation slice:

```text
Behavior: Verify
External crate: blake3 1.8, sha2 0.10, hex 0.4
Version management: Cargo resolves and updates versions through Cargo.toml/Cargo.lock; run cargo update after dependency changes.
Adoption depth: L1 internal use
Public semantic config: VerifyNeed::Digest { algorithm, value }
Private implementation config: none
Async: no async trait implementation for file hashing in this slice
Evidence produced: EvidenceDetail::Digest { algorithm, expected, observed }
Error compatibility: transitional PulithError keeps digest-specific variants until broader behavior error extraction
Old Pulith code disposition: delete old pulith-verify/custom checksum wrappers; do not restore them
Orthogonality proof: HashVerify performs Verify only; it does not acquire, prepare, apply, remember, inspect, repair, or forget
Resource controls: R0 for current single-file implementation; future batch hashing may introduce shared CPU budget as R1/R2
```

Feature surface used by this slice:

```text
sync local hash blake3
sync local hash sha2
```

Feature compatibility checks required:

```text
cargo check -p pulith --no-default-features
cargo check -p pulith --features 'sync local hash blake3'
cargo check -p pulith --features 'sync local hash sha2'
cargo check -p pulith --features async
cargo check -p pulith --all-features
cargo test -p pulith --all-features
```

## Required checks for this analysis

```text
git diff --check -- docs/report/pulith-implementation-library-error-orthogonality-plan.md
marker check: crates.io survey, feature rules, error taxonomy, orthogonality, revised migration order
```
