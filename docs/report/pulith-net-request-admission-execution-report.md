# Pulith Net Request Admission Execution Report

## Status

Completed.

This slice implements the reduced request-admission plan from:

```text
docs/report/pulith-net-behavior-tree-api-reduction.md
```

Production code changed:

```text
crates/pulith/src/net.rs
crates/pulith/src/lib.rs
```

The implementation deliberately uses the reduced naming and API:

```text
Admission, not generic budget/rate.
```

No byte bandwidth pacing, governor/leaky-bucket/Tower integration, object_store work, progress callback, or sidecar metadata was added.

## Implemented behavior

### Behavior tree position

Admission now sits inside each outbound attempt:

```text
Attempt[n]
  ResumePlan
  Admission
  RequestBuild
  SendRequest
  ResponseClassify
  BodyCopy
  Persist
```

The implementation enters admission after resume planning and before HTTP request construction/send.

This preserves the intended semantics:

```text
Retry creates attempts.
Each attempt enters admission once.
Resume remains request/response recovery.
Admission is pre-request resource behavior.
```

### Public policy API

Added:

```rust
pub enum NetAdmissionMode {
    Unbounded,
    Shared,
}
```

`NetAcquirePolicy` now includes:

```rust
pub admission: NetAdmissionMode,
```

with methods:

```rust
pub fn admission(mut self, admission: NetAdmissionMode) -> Self;
pub fn shared_admission(self) -> Self;
```

Default remains unbounded:

```text
NetAcquirePolicy::default().admission == NetAdmissionMode::Unbounded
```

### Public admission traits

Added sync admission:

```rust
#[cfg(feature = "ureq")]
pub trait NetSyncAdmission: Send + Sync {
    fn enter(&self) -> Result<NetAdmissionPermit, NetAdmissionError>;
}
```

Added async admission:

```rust
#[cfg(feature = "reqwest")]
pub trait NetAsyncAdmission: Send + Sync {
    fn enter(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<NetAdmissionPermit, NetAdmissionError>> + Send + '_>>;
}
```

Added permit:

```rust
pub struct NetAdmissionPermit { ... }
```

Public methods:

```rust
NetAdmissionPermit::immediate()
NetAdmissionPermit::waited(waited)
waited_for()
```

The permit is kept because a future semaphore/concurrency implementation may need to hold a private slot until attempt completion. Public evidence still exposes only the wait duration.

### Resource integration

`UreqResource` now owns optional sync admission:

```rust
admission: Option<Arc<dyn NetSyncAdmission>>
```

Methods:

```rust
with_admission(...)
admission()
```

`ReqwestResource` now owns optional async admission:

```rust
admission: Option<Arc<dyn NetAsyncAdmission>>
```

Methods:

```rust
with_admission(...)
admission()
```

No public no-op admission structs were added. The default resource has `None`, and default policy is `Unbounded`, so old behavior remains unchanged.

### Evidence

`NetAttemptEvidence` now records admission wait directly:

```rust
pub admission_wait: Option<Duration>,
```

This follows the reduced design:

```text
No one-field NetBudgetEvidence struct.
Admission wait belongs to the attempt record.
```

All existing attempt construction sites now set `admission_wait`.

### Error model

Added admission error kind:

```rust
pub enum NetAdmissionError {
    Unavailable,
    Closed,
    Rejected,
}
```

Added net-owned acquire error variant:

```rust
NetAcquireError::Admission {
    url,
    kind,
    attempts,
    resume,
}
```

Added coarse attempt outcome:

```rust
NetAttemptOutcome::AdmissionRejected
```

Detailed cause lives in `NetAdmissionError`; attempt outcome remains coarse phase classification.

### Backend behavior

#### ureq

Inside the ureq attempt loop:

```text
planned_resume(...)
admit_sync_attempt(...)
build ureq request
send request
```

If `NetAdmissionMode::Shared` is set but resource has no admission handle, acquire fails before HTTP send with:

```text
NetAcquireError::Admission { kind: NetAdmissionError::Unavailable, ... }
```

If admission handle rejects, acquire fails before HTTP send with:

```text
NetAcquireError::Admission { kind: NetAdmissionError::Rejected, ... }
```

Successful admission wait is recorded on that attempt.

#### reqwest

Inside the reqwest attempt loop:

```text
planned_resume(...)
admit_async_attempt(...).await
build reqwest request
send request
```

`acquire_reqwest` now receives the resource admission handle alongside client and delay.

## Tests added

### Pure API

```text
net_admission_defaults_to_unbounded_and_can_be_shared
```

Covers:

```text
default Unbounded
shared_admission() sets Shared
```

### ureq

```text
ureq_shared_admission_records_wait_on_attempt
ureq_shared_admission_rejection_fails_before_request
ureq_retry_enters_shared_admission_per_attempt
```

Covers:

```text
shared admission records attempt admission_wait
rejected admission returns NetAcquireError::Admission before network send path
retry re-enters admission once per outbound attempt
```

### reqwest

```text
reqwest_shared_admission_records_wait_on_attempt
reqwest_shared_admission_rejection_fails_before_request
```

Covers async parity for:

```text
shared admission wait evidence
admission rejection error before request send path
```

## API surface intentionally not added

This implementation does not add:

```text
NetBudgetPolicy
NetRequestBudgetMode
NetBudgetEvidence
NoNetSyncBudget
NoNetAsyncBudget
NetAdmissionPlan
NetBudgetRequest
NetRateEvidence
NetConcurrencyEvidence
```

The implemented public additions are limited to:

```text
NetAdmissionMode
NetAdmissionError
NetAdmissionPermit
NetSyncAdmission
NetAsyncAdmission
NetAttemptEvidence::admission_wait
NetAcquireError::Admission
NetAttemptOutcome::AdmissionRejected
```

## Concrete shared attempt-rate admission

The later short-name API now includes:

```rust
pub struct AttemptRate {
    attempts_per_second: NonZeroU32,
    burst_attempts: NonZeroU32,
}

pub struct RateAdmission { ... }
```

`RateAdmission` uses one governor direct GCRA state shared through `Arc`. It consumes one cell before each outbound attempt. Retry attempts continue to re-enter admission through the existing attempt loop.

Sync and async use the same decision:

```text
check one attempt cell
-> if unavailable, calculate wait
-> sleep/await
-> retry
-> return AdmissionPermit with accumulated requested wait
```

Only the wait effect differs:

```text
SyncAdmission  -> std::thread::sleep
AsyncAdmission -> tokio::time::sleep(...).await
```

This behavior is deliberately narrower than generic resource budgeting:

```text
RateAdmission = attempt-start rate
ByteRatePacer = decoded body-copy byte rate
neither type = maximum in-flight concurrency
```

Concrete ureq and reqwest integration continues to use the existing `with_admission` resource boundary. No convenience factories, middleware, global limiter, or backend-specific policy were added.

Focused additions:

```text
attempt_rate_preserves_rate_and_burst
sync_rate_admission_shares_attempt_budget
async_rate_admission_shares_attempt_budget
ureq_concrete_rate_admission_downloads
reqwest_concrete_rate_admission_downloads
```

## Verification

Fresh ad-hoc verification script:

```text
F:\Stratum\TEMP\hermes-verify-kmclqk_b.py
```

The script was cleaned:

```text
AD_HOC_SCRIPT_CLEANED=F:\Stratum\TEMP\hermes-verify-kmclqk_b.py
```

Marker:

```text
AD_HOC_VERIFY_PASS pulith request admission reduced api
```

Commands executed:

```text
cargo fmt --all --check
cargo check -p pulith --no-default-features
cargo check -p pulith --features "sync local net ureq hash blake3"
cargo check -p pulith --features "async net reqwest hash blake3"
cargo test -p pulith --features "sync local net ureq hash blake3" admission
cargo test -p pulith --features "async net reqwest hash blake3" admission
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::
cargo test -p pulith --features "async net reqwest hash blake3" net::tests::reqwest
cargo test --workspace --all-features
git diff --check -- crates/pulith/src/lib.rs crates/pulith/src/net.rs
```

Results:

```text
sync admission focused tests: 4 passed; 0 failed
async admission focused tests: 3 passed; 0 failed
sync ureq net tests: 25 passed; 0 failed
async reqwest net tests: 11 passed; 0 failed
workspace all-features tests: 66 passed; 0 failed
fmt/check/diff-check: passed
```

### Concrete admission follow-up verification

Fresh ad-hoc script:

```text
F:\Stratum\TEMP\hermes-verify-0n9wwsi3.py
```

Cleanup:

```text
AD_HOC_SCRIPT_CLEANED=F:\Stratum\TEMP\hermes-verify-0n9wwsi3.py
```

Marker:

```text
AD_HOC_VERIFY_PASS pulith concrete shared attempt-rate admission
```

Observed:

```text
AttemptRate API: 1 passed
sync rate_admission: 2 passed
async rate_admission: 2 passed
sync admission family: 6 passed
async admission family: 5 passed
sync net: 39 passed
async net: 36 passed
workspace all-features: 90 passed
fmt/check/diff-check: passed
```

## Remaining future work

Concrete attempt-start rate admission and body-copy byte pacing are now implemented as separate resource behaviors. The next unresolved advertised surface is the empty `object` feature.

Recommended next decision:

1. Search for real callers requiring S3/GCS/Azure/local object-store acquisition and define a typed `ObjectSource` only if that demand exists.
2. If no real caller or source semantic exists, delete the empty `object` feature and optional `object_store` dependency rather than preserving a speculative capability flag.
3. Treat maximum in-flight concurrency as a separate future design because current `AdmissionPermit` does not own a semaphore guard lifetime.
4. Keep raw transport/socket pacing outside `ByteRatePacer`; it remains body-copy pacing.
