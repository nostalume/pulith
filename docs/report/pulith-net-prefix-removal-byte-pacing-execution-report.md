# Pulith net prefix removal and byte pacing execution report

## Status

Implemented.

This slice executed the planned two-step cleanup:

1. Remove redundant `Net` prefixes from types inside `crates/pulith/src/net.rs`.
2. Add body-copy byte pacing as a BodyCopy-only behavior boundary, separate from request admission/retry/resume/validator logic.

## Files changed

```text
crates/pulith/src/net.rs
crates/pulith/src/lib.rs
crates/pulith/src/error.rs
docs/report/pulith-net-prefix-removal-byte-pacing-execution-report.md
```

## Prefix removal

The `net` module now relies on the module path as the semantic prefix.

Examples:

```rust
pulith::net::AcquirePolicy
pulith::net::AcquireError
pulith::net::AttemptEvidence
pulith::net::AdmissionMode
```

instead of:

```rust
pulith::net::NetAcquirePolicy
pulith::net::NetAcquireError
pulith::net::NetAttemptEvidence
pulith::net::NetAdmissionMode
```

### Rename map implemented

| Old | New |
|---|---|
| `NetAcquireError` | `AcquireError` |
| `NetTransportPhase` | `TransportPhase` |
| `NetProtocolError` | `ProtocolError` |
| `NetUnsafeDestination` | `UnsafeDestination` |
| `NetAdmissionError` | `AdmissionError` |
| `NetAcquirePolicy` | `AcquirePolicy` |
| `NetAdmissionMode` | `AdmissionMode` |
| `NetAdmissionPermit` | `AdmissionPermit` |
| `NetSyncAdmission` | `SyncAdmission` |
| `NetAsyncAdmission` | `AsyncAdmission` |
| `NetRetryPolicy` | `RetryPolicy` |
| `NetResumePolicy` | `ResumePolicy` |
| `NetResumeMode` | `ResumeMode` |
| `NetValidator` | `Validator` |
| `NetAcquireEvidence` | `AcquireEvidence` |
| `NetResumeEvidence` | `ResumeEvidence` |
| `NetResumeOutcome` | `ResumeOutcome` |
| `NetAttemptEvidence` | `AttemptEvidence` |
| `NetAttemptOutcome` | `AttemptOutcome` |
| `NetBodyCopyError` | `BodyCopyError` |
| `NetStageWriteError` | `StageWriteError` |

Kept as-is:

```text
RemoteUrl
RemoteSource
UreqResource
UreqAcquire
ReqwestResource
ReqwestAcquire
```

These names encode remote-source/backend identity rather than repeating the module name.

### Root error wrapper

`PulithError::NetAcquire(AcquireError)` remains intentionally prefixed because it lives at crate-root error scope. It is a root category, not a type inside `net`.

### Root re-exports

`lib.rs` now re-exports short net names. One exception: `net::AcquireEvidence` is not re-exported at crate root because `crate::evidence::AcquireEvidence` already occupies that root name. The detailed net evidence remains accessible through:

```rust
pulith::net::AcquireEvidence
```

## Body-copy byte pacing

This is intentionally not advertised as raw socket bandwidth control. The supported behavior boundary is:

```text
response body chunk observed -> max_bytes guard -> byte pacing -> staged write
```

Pulith paces bytes before they enter the local staging artifact. It does not promise to pace kernel socket reads, TLS records, Hyper/ureq internal buffers, or HTTP/2 flow-control windows.

### New API

```rust
pub enum BytePacingMode {
    Unbounded,
    Shared,
}

pub struct BytePacingPermit {
    waited: Duration,
}

pub enum PacingError {
    Unavailable,
    Closed,
    Rejected,
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
```

### Policy and resources

`AcquirePolicy` now includes:

```rust
pub byte_pacing: BytePacingMode
```

with builders:

```rust
AcquirePolicy::byte_pacing(...)
AcquirePolicy::shared_byte_pacing()
```

`UreqResource` now owns optional:

```rust
Arc<dyn SyncBytePacer>
```

`ReqwestResource` now owns optional:

```rust
Arc<dyn AsyncBytePacer>
```

with methods:

```rust
with_byte_pacer(...)
byte_pacer()
```

### Evidence

`AttemptEvidence` now records accumulated body-copy pacing wait:

```rust
pub pacing_wait: Duration
```

The default constructor sets:

```rust
Duration::ZERO
```

This avoids adding another frequent `Option<Duration>` field. Pacing wait is naturally accumulated over body chunks, so zero is the absence value.

### Error model

`AcquireError` now includes:

```rust
Pacing {
    url: url::Url,
    kind: PacingError,
    attempts: Vec<AttemptEvidence>,
    resume: Option<ResumeEvidence>,
}
```

This keeps byte-pacing rejection separate from request-admission rejection.

Pacing rejection is also represented in attempt evidence as:

```rust
AttemptOutcome::PacingRejected
```

This avoids conflating pacing failure with `AttemptOutcome::LimitExceeded`, which is reserved for `max_bytes` guard failure.

## Orthogonality preserved

Byte pacing is wired only into body copy:

```text
ureq:    copy_response_body(..., pacer)
reqwest: StagedDownload<Open>::write_chunk(..., pacer).await
```

It is not passed to:

```text
admit_sync_attempt / admit_async_attempt
planned_resume
request construction
HTTP status classification
retry delay calculation
validator selection
persist/final placement
```

Current behavior tree:

```text
Attempt[n]
  ResumePlan
  Admission              # request/resource admission
  RequestBuild
  SendRequest
  ResponseClassify
  BodyCopy
    ReceiveChunk         # backend-specific: ureq Read, reqwest async chunk stream
    MaxBytesGuard        # reject before waiting on a chunk that will never be staged
    BytePacing           # body-copy boundary; chunk bytes only
    StageWrite
  Persist
```

The sync/async difference is mechanical rather than semantic:

```text
ureq    = blocking Read/Write over a NamedTempFile local to the acquire loop
reqwest = async chunk().await plus StagedDownload<Open> to own flush/drop-before-persist
```

Both backends now follow the same staging law:

```text
destination is untouched until body copy completes, the stage is finalized, and persist succeeds
```

## TDD evidence

### Prefix RED

Added and first ran:

```text
net::tests::module_short_names_replace_net_prefix
```

It failed before rename with missing short names such as `AcquirePolicy`, `RetryPolicy`, `ResumePolicy`, `AdmissionMode`, `AttemptEvidence`, and `AttemptOutcome`.

### Pacing RED

Added and first ran:

```text
cargo test -p pulith --features "sync local net ureq hash blake3" pacing
```

It failed before implementation because `SyncBytePacer`, `BytePacingPermit`, and `PacingError` did not exist.

Then implemented the minimum API and wiring to pass sync and async pacing tests.

### Semantic alignment RED

Added unavailable-pacer tests for both backends:

```text
ureq_shared_byte_pacing_unavailable_records_rejection_without_persist
reqwest_shared_byte_pacing_unavailable_records_rejection_without_persist
```

They assert:

```text
AcquireError::Pacing { kind: PacingError::Unavailable, ... }
AttemptOutcome::PacingRejected
pacing_wait == Duration::ZERO
bytes == 0
destination does not exist
```

## Contract hardening follow-up

Added sync/async parity tests proving:

- `max_bytes` rejection runs before byte pacing, so a rejected chunk does not acquire a pacing permit or pay pacing wait.
- A pacer returning `PacingError::Rejected` produces `AttemptOutcome::PacingRejected`, not `LimitExceeded`.
- Failed max guards and pacing rejection leave the destination untouched.
- Sync and async backends share the same behavior contract even though ureq owns staging as a local `NamedTempFile` and reqwest owns it through `StagedDownload<Open>`.

Public rustdoc now states that `BytePacingMode` controls body-copy materialization rather than raw socket bandwidth.

Focused tests:

```text
ureq_max_bytes_rejects_before_byte_pacing
reqwest_max_bytes_rejects_before_byte_pacing
ureq_byte_pacing_rejection_records_pacing_rejected_without_persist
reqwest_byte_pacing_rejection_records_pacing_rejected_without_persist
```

## Concrete shared byte-rate pacer

`ByteRatePacer` supplies a production body-copy limiter backed by one governor direct GCRA state. `ByteRate` names the sustained bytes-per-second rate and maximum burst bytes with `NonZeroU32` fields.

Both backend traits execute the same decision loop:

```text
bound batch by burst_bytes
-> check_n(batch)
-> wait when nonconforming
-> retry
-> account accepted batch
```

Only the wait effect differs:

```text
SyncBytePacer  -> std::thread::sleep
AsyncBytePacer -> tokio::time::sleep(...).await
```

Chunks larger than `burst_bytes` are split into bounded accounting batches instead of being rejected as insufficient capacity. Zero-byte calls are immediate. Sharing the pacer through `Arc` shares one atomic budget across resources and calls.

This remains decoded body-copy pacing. It does not control socket reads, TLS buffering, or HTTP transport flow control. Cancellation after accepted sub-batches can conservatively consume budget without staging the chunk, but cannot permit an overrun.

Focused concrete tests cover typed rate access, zero-byte behavior, burst splitting, shared-state waits, and real ureq/reqwest downloads through the existing `with_byte_pacer` resource boundary.

## Fresh ad-hoc verification

Final script:

```text
F:\Stratum\TEMP\hermes-verify-1o82izph.py
```

Cleanup:

```text
AD_HOC_SCRIPT_CLEANED=F:\Stratum\TEMP\hermes-verify-1o82izph.py
```

Marker:

```text
AD_HOC_VERIFY_PASS pulith concrete shared body-copy byte-rate pacer
```

Commands:

```text
cargo fmt --all --check
cargo check -p pulith --no-default-features
cargo check -p pulith --features net
cargo check -p pulith --features "sync local net ureq hash blake3"
cargo check -p pulith --features "async net reqwest hash blake3"
cargo test -p pulith --features net byte_rate_preserves_rate_and_burst
cargo test -p pulith --features "sync local net ureq hash blake3" byte_rate_pacer
cargo test -p pulith --features "async net reqwest hash blake3" byte_rate_pacer
cargo test -p pulith --features "sync local net ureq hash blake3" byte_pacing
cargo test -p pulith --features "async net reqwest hash blake3" byte_pacing
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::
cargo test -p pulith --features "async net reqwest hash blake3" net::tests::
cargo test --workspace --all-features
git diff --check -- Cargo.toml Cargo.lock crates/pulith/Cargo.toml crates/pulith/src/net.rs crates/pulith/src/lib.rs docs/report/pulith-net-prefix-removal-byte-pacing-execution-report.md
```

Observed results:

```text
net ByteRate API test: 1 passed
sync byte_rate_pacer tests: 4 passed
async byte_rate_pacer tests: 4 passed
sync byte_pacing tests: 4 passed
async byte_pacing tests: 4 passed
sync ureq net tests: 36 passed
async reqwest net tests: 33 passed
workspace all-features tests: 85 passed
fmt/check/diff-check: passed
```
