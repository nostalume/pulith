# Pulith net prefix removal and byte bandwidth pacing plan

## Status

Planning/report only. No production code was changed in this slice.

User correction accepted:

```text
Inside `net` module, public types do not need the `Net` prefix.
```

The current naming is module-redundant:

```rust
pulith::net::NetAcquireError
pulith::net::NetAcquirePolicy
pulith::net::NetAttemptEvidence
pulith::net::NetAdmissionMode
```

Preferred module-scoped vocabulary:

```rust
pulith::net::AcquireError
pulith::net::AcquirePolicy
pulith::net::AttemptEvidence
pulith::net::AdmissionMode
```

Root re-export policy needs an explicit decision during implementation. My recommendation is:

```text
Inside `crate::net`: remove `Net` prefix.
At crate root: re-export the short names only if root API remains small enough; otherwise ask callers to import from `pulith::net`.
No compatibility aliases unless explicitly requested.
```

## Current context

Current changed area:

```text
crates/pulith/src/net.rs
crates/pulith/src/lib.rs
crates/pulith/src/error.rs
```

Current public `net.rs` vocabulary includes:

```text
NetAcquireError
NetTransportPhase
NetProtocolError
NetUnsafeDestination
NetAdmissionError
NetAcquirePolicy
NetAdmissionMode
NetAdmissionPermit
NetSyncAdmission
NetAsyncAdmission
NetRetryPolicy
NetResumePolicy
NetResumeMode
NetValidator
RemoteUrl
RemoteSource
NetAcquireEvidence
NetResumeEvidence
NetResumeOutcome
NetAttemptEvidence
NetAttemptOutcome
UreqResource
UreqAcquire
ReqwestResource
ReqwestAcquire
```

Current non-prefixed names that already read well inside `net`:

```text
RemoteUrl
RemoteSource
UreqResource
UreqAcquire
ReqwestResource
ReqwestAcquire
```

These should probably remain unchanged unless a later broader backend naming pass decides `Ureq`/`Reqwest` should become ZST-style backend markers.

## External research summary

### governor

Command used:

```text
cargo search --registry crates-io governor --limit 5
cargo info --registry crates-io governor
```

Findings:

```text
governor = 0.10.4
Description: rate-limiting implementation in Rust
Tags: rate-limiting, rate-limit, no_std, gcra
Default features: std, dashmap, jitter, quanta
Docs: https://docs.rs/governor
```

Assessment:

```text
Good mature candidate for request-start rate limiting or keyed quotas.
Too policy-heavy for first byte bandwidth pacing slice.
It owns GCRA/request-rate semantics, not necessarily stream body-copy pacing.
Would add dependency surface and a conceptual dependency on rate-limiter terminology.
```

Decision:

```text
Do not add governor for first byte pacing slice.
Reconsider for future request-rate admission implementation.
```

### leaky-bucket

Command used:

```text
cargo search --registry crates-io leaky-bucket --limit 5
cargo info --registry crates-io leaky-bucket
```

Findings:

```text
leaky-bucket = 1.1.2
Description: token-based rate limiter based on leaky bucket algorithm
License: MIT OR Apache-2.0
Docs: https://docs.rs/leaky-bucket
Depends on tokio, parking_lot, pin-project-lite
```

Assessment:

```text
Closer to token bucket / byte-token semantics.
But it is async/Tokio-shaped and therefore not directly shared by sync ureq without either blocking adapters or separate sync path.
Good candidate only if the reqwest path needs a concrete async limiter soon.
```

Decision:

```text
Do not add leaky-bucket in first unified design.
Use Pulith-owned trait boundary first; concrete async token bucket can be implemented later if real callers need it.
```

### async-rate-limiter

Command used:

```text
cargo search --registry crates-io async-rate-limiter --limit 5
cargo info --registry crates-io async-rate-limiter
```

Findings:

```text
async-rate-limiter = 1.1.0
Description: pure Rust token bucket for API access frequency
Default feature: rt-tokio
Also has async-std feature
License unknown in cargo info
```

Assessment:

```text
Targets API access frequency, not stream chunk pacing.
Runtime-feature surface is broader than Pulith's current first need.
Unknown license in cargo metadata is a blocker for adopting casually.
```

Decision:

```text
Reject for now.
```

### tokio-util

Command used:

```text
cargo search --registry crates-io tokio-util --limit 5
cargo info --registry crates-io tokio-util
```

Findings:

```text
tokio-util = 0.7.18
Feature families: io, io-util, codec, time, net, rt
ReaderStream and stream/io adapters exist behind io/io-util features
```

Assessment:

```text
Useful if Pulith later switches reqwest from `chunk().await` into stream adapters.
Not needed for current reqwest body primitive; existing `Response::chunk().await` is explicit and range-aware enough for the current design.
```

Decision:

```text
Do not add tokio-util for first byte pacing slice.
If future design needs Stream/AsyncRead adapters, enable a narrow tokio-util feature then.
```

### tower / reqwest-leaky-bucket

Command used:

```text
cargo info --registry crates-io tower
cargo info --registry crates-io reqwest-leaky-bucket
```

Findings:

```text
tower = 0.5.3, modular client/server middleware, includes `limit` feature.
reqwest-leaky-bucket = 0.5.0, reqwest-specific leaky-bucket rate-limit middleware.
```

Assessment:

```text
Tower middleware belongs around request services, not body-copy bytes.
reqwest-leaky-bucket is reqwest-specific and would violate ureq/reqwest parity for the first Pulith-owned behavior.
```

Decision:

```text
Reject for first byte pacing and for module naming cleanup.
```

## Naming reduction plan

### Principle

Within `net.rs`, names should use the module as the prefix:

```text
net::AcquireError, not net::NetAcquireError
net::AdmissionMode, not net::NetAdmissionMode
```

This is consistent with the user's correction and with Rust API style where module paths provide semantic context.

### Proposed rename map

| Current | Proposed inside `crate::net` | Notes |
|---|---|---|
| `NetAcquireError` | `AcquireError` | `PulithError::NetAcquire` may stay as root error variant unless root error design is also being renamed. |
| `NetTransportPhase` | `TransportPhase` | Error sub-ADT. |
| `NetProtocolError` | `ProtocolError` | Error sub-ADT. |
| `NetUnsafeDestination` | `UnsafeDestination` | Error sub-ADT. |
| `NetAdmissionError` | `AdmissionError` | Admission sub-ADT. |
| `NetAcquirePolicy` | `AcquirePolicy` | Main policy. |
| `NetAdmissionMode` | `AdmissionMode` | Request admission policy field. |
| `NetAdmissionPermit` | `AdmissionPermit` | Admission trait return. |
| `NetSyncAdmission` | `SyncAdmission` | Sync trait; module path disambiguates. |
| `NetAsyncAdmission` | `AsyncAdmission` | Async trait. |
| `NetRetryPolicy` | `RetryPolicy` | Attempt-loop policy. |
| `NetResumePolicy` | `ResumePolicy` | Resume behavior policy. |
| `NetResumeMode` | `ResumeMode` | Resume mode ADT. |
| `NetValidator` | `Validator` | HTTP validator ADT; module path supplies net/HTTP context. |
| `NetAcquireEvidence` | `AcquireEvidence` | Acquire output proof. |
| `NetResumeEvidence` | `ResumeEvidence` | Resume proof. |
| `NetResumeOutcome` | `ResumeOutcome` | Resume proof outcome. |
| `NetAttemptEvidence` | `AttemptEvidence` | Attempt record. |
| `NetAttemptOutcome` | `AttemptOutcome` | Attempt record outcome. |
| `NetBodyCopyError` | `BodyCopyError` | Private ureq body-copy intermediate. |
| `NetStageWriteError` | `StageWriteError` | Private reqwest body-copy intermediate. |

Names to keep:

```text
RemoteUrl
RemoteSource
UreqResource
UreqAcquire
ReqwestResource
ReqwestAcquire
SyncDelay
AsyncDelay
AsyncDelayFuture
```

Rationale:

```text
RemoteUrl/RemoteSource describe selected source kind, not merely module membership.
Ureq*/Reqwest* encode backend family, which is still needed under the `net` module.
```

### Root re-export strategy

Current root re-exports would become ambiguous if all short names are dumped into `pulith::*`:

```rust
pub use net::{AcquireError, AcquireEvidence, AcquirePolicy, ...};
```

This is readable now, but may conflict later with archive/local/hash concepts such as `Policy`, `Evidence`, `Validator`, `Outcome`.

Recommended root policy:

1. Keep behavior-node types root-visible only when they are used in the top-level typed tree.
2. Prefer users importing detailed net types from `pulith::net::{...}`.
3. Avoid compatibility aliases like `pub type NetAcquireError = AcquireError` unless the user explicitly wants transitional API support.

Because the user explicitly suggested removing prefixes, my implementation plan assumes no compatibility aliases.

## Step-by-step implementation plan: naming reduction

### Task 1 — RED test for module-short names

File:

```text
crates/pulith/src/net.rs
```

Add a small compile/test usage proving the desired names exist inside the module:

```rust
#[test]
fn module_short_names_replace_net_prefix() {
    let policy = AcquirePolicy::default()
        .retry(RetryPolicy::disabled())
        .resume(ResumePolicy::restart_only())
        .shared_admission();
    assert_eq!(policy.admission, AdmissionMode::Shared);

    let attempt = AttemptEvidence::new(0, AttemptOutcome::AdmissionRejected);
    assert_eq!(attempt.status, None);
}
```

Run and expect failure before rename:

```text
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::module_short_names_replace_net_prefix
```

Expected failure:

```text
cannot find type/value `AcquirePolicy`, `RetryPolicy`, ...
```

### Task 2 — Rename type definitions in `net.rs`

File:

```text
crates/pulith/src/net.rs
```

Perform mechanical renames using AST-aware or careful search/replace.

Priority order:

1. Error types: `NetAcquireError` -> `AcquireError`, etc.
2. Policy/evidence types: `NetAcquirePolicy` -> `AcquirePolicy`, etc.
3. Admission traits: `NetSyncAdmission` -> `SyncAdmission`, `NetAsyncAdmission` -> `AsyncAdmission`.
4. Private body-copy errors.
5. Tests.

Run after this task:

```text
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::module_short_names_replace_net_prefix
```

### Task 3 — Update `error.rs`

File:

```text
crates/pulith/src/error.rs
```

Change import:

```rust
use crate::net::AcquireError;
```

Change wrapper variant source type:

```rust
PulithError::NetAcquire(AcquireError)
```

Keep variant name `NetAcquire` for now because it is a root-level error category, not a type inside `net`.

Run:

```text
cargo check -p pulith --features "sync local net ureq hash blake3"
```

### Task 4 — Update `lib.rs` re-exports

File:

```text
crates/pulith/src/lib.rs
```

Preferred clean export shape:

```rust
#[cfg(feature = "net")]
pub mod net;
```

and either remove detailed root re-exports or re-export only behavior nodes.

Because the crate already re-exports many module details from `local`, a conservative implementation may re-export the short net names temporarily:

```rust
pub use net::{AcquireError, AcquireEvidence, AcquirePolicy, ...};
```

But this should be a deliberate choice. My recommendation for long-term cleanliness:

```text
root: behavior spine + common nodes
module: detailed backend/error/policy/evidence types
```

If removing root re-exports is too broad for this slice, keep short root re-exports and avoid old `Net*` aliases.

### Task 5 — Search and remove stale `Net*` names

Commands:

```text
search_files("Net[A-Z][A-Za-z]+", path="crates/pulith/src")
search_files("Net[A-Z][A-Za-z]+", path="docs/report")
```

Expected production result:

```text
No `Net*` type names in active Rust code except root variant `PulithError::NetAcquire` if kept.
```

Do not rewrite historical reports unless they are active authority.

### Task 6 — Fresh verification

Use required Pulith ad-hoc verification:

```text
F:\Stratum\TEMP\hermes-verify-*.py
```

Commands:

```text
cargo fmt --all --check
cargo check -p pulith --no-default-features
cargo check -p pulith --features "sync local net ureq hash blake3"
cargo check -p pulith --features "async net reqwest hash blake3"
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::
cargo test -p pulith --features "async net reqwest hash blake3" net::tests::reqwest
cargo test --workspace --all-features
git diff --check -- crates/pulith/src/net.rs crates/pulith/src/lib.rs crates/pulith/src/error.rs
```

## Next design: byte bandwidth pacing

### Goal

Add optional per-body-copy byte pacing without changing request admission, retry, resume, validator, or net-owned error semantics.

### Non-goals

Do not implement in this slice:

```text
parallel segmented download
multi-source mirror racing
object_store pacing
global singleton limiter
Tower middleware
reqwest-only middleware
progress callbacks
sidecar metadata
bytes_stream() migration
```

### Behavior tree insertion point

Target placement:

```text
Attempt[n]
  ResumePlan
  Admission              # request/resource admission, already implemented
  RequestBuild
  SendRequest
  ResponseClassify
  BodyCopy
    BytePacing           # future insertion here only
    MaxBytesCheck
    WriteChunk
  Persist
```

Pacing actor sees:

```text
chunk length
already copied bytes
optional rate config / shared limiter
```

Pacing actor must not see or decide:

```text
HTTP status retryability
Range/If-Range validity
Content-Range validity
admission mode
destination persist semantics
```

### Proposed short-name API after prefix removal

Inside `net` module:

```rust
pub enum BytePacingMode {
    Unbounded,
    Shared,
}

pub struct BytePacingPermit {
    waited: Duration,
}

#[cfg(feature = "ureq")]
pub trait SyncBytePacer: Send + Sync {
    fn before_chunk(&self, bytes: u64) -> Result<BytePacingPermit, PacingError>;
}

#[cfg(feature = "reqwest")]
pub trait AsyncBytePacer: Send + Sync {
    fn before_chunk(
        &self,
        bytes: u64,
    ) -> Pin<Box<dyn Future<Output = Result<BytePacingPermit, PacingError>> + Send + '_>>;
}

pub enum PacingError {
    Unavailable,
    Closed,
    Rejected,
}
```

Policy extension:

```rust
pub struct AcquirePolicy {
    pub timeout: Option<Duration>,
    pub max_bytes: Option<u64>,
    pub headers: Vec<(String, String)>,
    pub retry: RetryPolicy,
    pub resume: ResumePolicy,
    pub admission: AdmissionMode,
    pub byte_pacing: BytePacingMode,
}
```

Resource extension:

```rust
pub struct UreqResource {
    agent: ureq::Agent,
    delay: SyncDelay,
    admission: Option<Arc<dyn SyncAdmission>>,
    byte_pacer: Option<Arc<dyn SyncBytePacer>>,
}

pub struct ReqwestResource {
    client: reqwest::Client,
    delay: AsyncDelay,
    admission: Option<Arc<dyn AsyncAdmission>>,
    byte_pacer: Option<Arc<dyn AsyncBytePacer>>,
}
```

Evidence extension should be minimal:

```rust
pub struct AttemptEvidence {
    ...
    pub pacing_wait: Option<Duration>,
}
```

But `Option<Duration>` may under-report multi-chunk waits. Better first design:

```rust
pub pacing_wait: Duration
```

with zero default. This avoids frequent `None` and matches body-copy accumulation semantics.

Recommendation:

```text
Use `Duration` for accumulated pacing wait, not `Option<Duration>`.
```

### Error model

If pacing can reject/close, add:

```rust
AcquireError::Pacing {
    url: url::Url,
    kind: PacingError,
    attempts: Vec<AttemptEvidence>,
    resume: Option<ResumeEvidence>,
}
```

But if first implementation is only no-op/default with test pacers, no production error path is needed until shared pacing mode exists.

Recommendation for first executable slice:

```text
Implement trait boundary + evidence accumulation + test pacer.
Do not add concrete token bucket dependency.
```

### Concrete implementation options

#### Option A — Pulith-owned no-dependency trait first

Use only:

```text
std::thread::sleep for sync test/concrete simple pacer if needed
tokio::time::sleep for async test/concrete simple pacer if needed
```

Pros:

```text
No new dependencies.
Sync/async parity.
Behavior semantics stay Pulith-owned.
Easy to test deterministically with fake pacers returning waited durations without sleeping.
```

Cons:

```text
No production-grade shared token bucket yet.
```

Recommendation:

```text
Choose Option A first.
```

#### Option B — leaky-bucket for async only

Pros:

```text
Good token-bucket semantics.
Mature enough.
```

Cons:

```text
Tokio-shaped; no sync ureq parity.
Would force adapter design before behavior is proven.
```

Decision:

```text
Defer.
```

#### Option C — governor

Pros:

```text
Mature GCRA rate limiter.
Good for keyed request-rate admission.
```

Cons:

```text
Better fit for request admission/rate than body-copy byte pacing.
More dependency surface.
```

Decision:

```text
Defer to request-rate admission if needed.
```

## Byte pacing implementation task plan

### Task 1 — RED sync test: byte pacer called per ureq chunk

Test intent:

```text
Given Shared byte pacing and a fake sync pacer,
when ureq downloads a body spanning multiple read chunks,
then pacer is entered for each non-empty body chunk before writing,
and evidence accumulates pacing wait.
```

Implementation note:

```text
The current ureq buffer is 16 KiB, so test body must exceed 16 KiB if chunk count matters.
```

### Task 2 — RED async test: byte pacer called per reqwest chunk

Test same behavior for reqwest path.

Avoid asserting exact network chunk count unless the test server writes controlled chunks. Prefer asserting:

```text
enters >= 1
pacing_wait == sum(fake waits observed)
```

### Task 3 — Add short-name pacing API under `net` module

Add:

```text
BytePacingMode
BytePacingPermit
PacingError
SyncBytePacer
AsyncBytePacer
```

Only after prefix-removal migration, so no `NetBytePacingMode`.

### Task 4 — Add resource handles and policy field

Add:

```text
AcquirePolicy::byte_pacing(BytePacingMode)
AcquirePolicy::shared_byte_pacing()
UreqResource::with_byte_pacer(...)
ReqwestResource::with_byte_pacer(...)
```

Default:

```text
BytePacingMode::Unbounded
```

### Task 5 — Thread pacing through BodyCopy only

ureq:

```text
copy_response_body(reader, writer, max_bytes, initial_bytes, pacer) -> BodyCopyResult
```

reqwest:

```text
stage.write_chunk(&chunk, max_bytes, pacer).await
```

Do not pass pacer into request build, response classification, retry, or resume.

### Task 6 — Evidence accumulation

Avoid `Option<Duration>` for accumulated pacing. Use:

```text
pacing_wait: Duration
```

Constructor default:

```text
Duration::ZERO
```

Each body-copy attempt updates the current attempt evidence on terminal push.

### Task 7 — Verification

Ad-hoc script under `F:\Stratum\TEMP\hermes-verify-*.py`:

```text
cargo fmt --all --check
cargo check -p pulith --no-default-features
cargo check -p pulith --features "sync local net ureq hash blake3"
cargo check -p pulith --features "async net reqwest hash blake3"
cargo test -p pulith --features "sync local net ureq hash blake3" pacing
cargo test -p pulith --features "async net reqwest hash blake3" pacing
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::
cargo test -p pulith --features "async net reqwest hash blake3" net::tests::reqwest
cargo test --workspace --all-features
git diff --check -- crates/pulith/src/net.rs crates/pulith/src/lib.rs crates/pulith/src/error.rs
```

## Recommended execution order

1. Naming prefix removal first.
2. Fresh verify.
3. Byte pacing trait boundary + fake-pacer tests.
4. Fresh verify.
5. Only after those pass, decide whether concrete limiter is needed.

Rationale:

```text
If prefix removal happens after pacing, every new pacing type will need to be renamed too.
```

## Risks and mitigations

### Risk: root re-export ambiguity

Short names like `Evidence`, `Policy`, `Validator` are too generic at crate root.

Mitigation:

```text
Use `pulith::net::{AcquirePolicy, AttemptEvidence, ...}` as the recommended import path.
```

### Risk: rename touches many tests and docs

Mitigation:

```text
TDD compile test first, then mechanical rename, then full feature matrix checks.
Do not rewrite historical reports except active references.
```

### Risk: pacing accidentally becomes admission

Mitigation:

```text
Pacing traits only receive chunk byte count.
No URL/status/retry/resume inputs.
Wire only inside BodyCopy functions.
```

### Risk: exact chunk-count tests become flaky

Mitigation:

```text
Use fake controlled readers/writers for unit tests where possible.
For loopback HTTP tests, assert at least one pacing call and total wait evidence rather than exact chunk count unless server chunking is deterministic.
```

## Final recommendation

Proceed with two separate implementation slices:

```text
Slice A: remove redundant `Net` prefixes inside `net` module.
Slice B: add byte bandwidth pacing as BodyCopy-only trait boundary with no external dependency.
```

Do not add governor/leaky-bucket/tokio-util/Tower yet. The first pacing slice should prove Pulith's behavior boundary and evidence shape before selecting any concrete limiter crate.
