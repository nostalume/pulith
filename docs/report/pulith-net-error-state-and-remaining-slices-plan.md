# Pulith Net Error/State Boundary and Remaining Slice Plan

## Status

This is an optimized planning report after the user correction:

> first distinguish state records from errors; errors should capture the information we need, not the other way around.

No production code is changed by this report.

## Correction accepted

The previous next-step wording treated missing failure attempt history as a reason to add more state/evidence plumbing. That direction is backwards.

Correct model:

```text
State/evidence records successful behavior transitions.
Error captures failure context needed to diagnose or decide recovery.
```

Therefore:

```text
Acquired + NetAcquireEvidence
  = successful net Acquire state.

NetAcquireError / NetAcquireFailure
  = failed Acquire diagnostic/recovery information.
```

Do not make successful state carry fields just because a failure path wants them.

## Current implementation baseline

Current net retry code already has:

```text
NetRetryPolicy
NetAttemptEvidence
NetAttemptOutcome
NetAcquirePolicy.retry
NetAcquireEvidence.attempts
UreqResource injected SyncDelay
ReqwestResource injected AsyncDelay
ureq retry loop
reqwest retry loop
Retry-After parsing through httpdate
```

Current gap:

```text
attempt history is only returned in NetAcquireEvidence on success.
when all attempts fail, Acquire returns PulithError and loses the collected attempt facts.
```

The fix should not be “make state/evidence broader.” The fix should be “make net Acquire error capture failure facts.”

## Research performed

### Repo inspected

```text
crates/pulith/src/net.rs
crates/pulith/src/error.rs
docs/report/pulith-net-retry-execution-report.md
docs/report/pulith-net-retry-next-slice-plan.md
```

Key current constraints:

```text
AcquireNode::Error for UreqAcquire is currently PulithError.
AsyncAcquireNode::Error for ReqwestAcquire is currently PulithError.
PulithError is a broad transitional enum.
PulithError::source() only exposes Io source today.
NetAttemptEvidence is currently public and tied to NetAcquireEvidence success state.
```

### Error handling references searched/read

Commands/pages inspected:

```text
https://doc.rust-lang.org/std/error/trait.Error.html
https://docs.rs/thiserror/latest/thiserror/
cargo info --registry crates-io thiserror
cargo info --registry crates-io miette
```

Findings:

```text
std::error::Error is the standard trait for Result error values.
Error::source() is the standard way to expose underlying causes.
thiserror derives Display/Error/From/source ergonomically without becoming part of public API shape.
miette is for rich human diagnostics; too broad for this library boundary now.
```

Plan implication:

```text
Use ordinary typed errors first.
Do not add miette/reporting dependencies now.
Consider thiserror only if manual Display/source becomes noisy; current project already has a manual PulithError and does not depend on thiserror in pulith.
```

### HTTP retry/range references searched/read

Commands/pages inspected:

```text
MDN 429 Too Many Requests
MDN Retry-After header
MDN Range header
MDN If-Range header
cargo info --registry crates-io governor
cargo info --registry crates-io http-range-header
```

Findings:

```text
429 commonly uses Retry-After to signal when to retry.
Retry-After can be delay seconds or HTTP-date.
Range requests may yield 206 Partial Content.
Invalid ranges may yield 416 Range Not Satisfiable.
If-Range makes Range conditional: if validator matches, server returns 206; if not, server returns 200 full body.
Range/resume requires validators such as ETag or Last-Modified and careful 200/206/416 branching.
governor is a mature rate limiter, but hidden global rate limiting is not the right first Pulith budget boundary.
http-range-header parses Range headers, but Pulith needs to construct simple byte ranges before parsing arbitrary user Range headers.
```

Plan implication:

```text
Finish error boundary before budget/rate.
Finish budget/rate before Range/resume.
Do not add governor until a typed resource/budget API exists.
Do not add http-range-header for first resume slice unless parsing arbitrary Range/Content-Range becomes necessary.
```

## Optimized architecture decision

### State vs error boundary

State records should only describe a completed successful transition:

```text
Chosen<I, RemoteSource>
  --Acquire succeeds-->
Acquired<I, LocalMaterial, NetAcquireEvidence>
```

Error records should describe why the transition did not complete and carry the information needed for recovery/diagnosis:

```text
Chosen<I, RemoteSource>
  --Acquire fails-->
Err(NetAcquireError)
```

So the next cleanup should split net Acquire error from global `PulithError`:

```rust
pub enum NetAcquireError {
    Url(PulithError),              // or keep pre-parse errors before RemoteUrl
    Destination(PulithError),      // destination guard / parent creation / local staging
    HttpStatus(NetFailureReport),
    Network(NetFailureReport),
    Limit(NetFailureReport),
    LocalIo(NetFailureReport),
}
```

But to avoid over-designing variants, start smaller:

```rust
pub struct NetAcquireFailure {
    pub error: PulithError,
    pub attempts: Vec<NetAttemptRecord>,
}
```

and:

```rust
pub enum NetAcquireError {
    Failed(NetAcquireFailure),
}
```

or simply:

```rust
pub struct NetAcquireError {
    pub error: PulithError,
    pub attempts: Vec<NetAttemptRecord>,
}
```

Preferred first implementation:

```rust
pub struct NetAcquireError {
    error: PulithError,
    attempts: Vec<NetAttemptRecord>,
}
```

with accessors:

```rust
error()
attempts()
into_parts()
```

Rationale:

```text
keeps function count and variant count low;
lets errors capture facts;
keeps successful state from becoming a diagnostic dump;
can still implement Display/Error/source cleanly;
future variants can be added only when needed.
```

### Rename attempt evidence conceptually

`NetAttemptEvidence` currently names attempt facts as evidence. For the corrected model, attempts are not necessarily successful behavior evidence. They are operation records.

Recommended next cleanup:

```rust
NetAttemptEvidence -> NetAttemptRecord
NetAttemptOutcome -> NetAttemptOutcome   // name is OK
```

Then:

```rust
NetAcquireEvidence {
    ...,
    attempts: Vec<NetAttemptRecord>,  // successful operation history
}

NetAcquireError {
    error: PulithError,
    attempts: Vec<NetAttemptRecord>,  // failed operation history
}
```

This resolves the naming tension:

```text
attempt record = neutral operation fact
acquire evidence = successful transition evidence
acquire error = failure context
```

If public rename churn is undesirable, keep `NetAttemptEvidence` for one more slice, but document it as an operation record. Since this code is still young and no compatibility promise exists, prefer renaming now.

## Remaining slices, optimized

### Slice 1 — Net Acquire error boundary

Goal:

```text
Errors capture failure facts; success state stays success evidence.
```

Changes:

```rust
pub struct NetAcquireError {
    error: PulithError,
    attempts: Vec<NetAttemptRecord>,
}

impl NetAcquireError {
    pub fn new(error: PulithError, attempts: Vec<NetAttemptRecord>) -> Self;
    pub fn error(&self) -> &PulithError;
    pub fn attempts(&self) -> &[NetAttemptRecord];
    pub fn into_parts(self) -> (PulithError, Vec<NetAttemptRecord>);
}
```

Trait changes:

```rust
impl<I> AcquireNode<Chosen<I, RemoteSource>> for UreqAcquire<UreqResource> {
    type Error = NetAcquireError;
}

impl<I: 'static> AsyncAcquireNode<Chosen<I, RemoteSource>> for ReqwestAcquire<ReqwestResource> {
    type Error = NetAcquireError;
}
```

Error behavior:

```text
If failure occurs before any HTTP attempt, attempts is empty.
If failure occurs after retry attempts, attempts contains all completed/failed attempt records.
If final status is non-success, error is PulithError::HttpStatus and attempts includes final status record.
If final network failure occurs, error is PulithError::NetworkError and attempts includes final network record.
If local staging/persist fails, error is PulithError::Io and attempts captures records only if attempt had started; no retry.
```

Tests first:

```text
ureq_retry_failure_returns_attempt_records
reqwest_retry_failure_returns_attempt_records
ureq_destination_guard_error_has_empty_attempts
reqwest_limit_error_returns_limit_attempt_record
```

Quality constraints:

```text
No new retry executor.
No miette.
No thiserror unless Display/source code gets meaningfully worse.
No extra state fields.
No hidden global error log.
```

Verification:

```text
cargo fmt --all --check
cargo check -p pulith --features "sync local net ureq hash blake3"
cargo check -p pulith --features "async net reqwest hash blake3"
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::
cargo test -p pulith --features "async net reqwest hash blake3" net::tests::reqwest
cargo test --workspace --all-features
fresh F:\Stratum\TEMP\hermes-verify-* script
```

### Slice 2 — Trim successful state evidence

After `NetAcquireError` exists, revisit `NetAcquireEvidence.attempts`.

Options:

1. Keep attempts on success.
2. Move attempts out of success evidence and keep only final successful facts.
3. Keep only compact success fields plus maybe `attempt_count`.

Recommended decision:

```text
Keep attempts on success for now, but name them records, not evidence.
```

Reason:

```text
A successful Acquire with retries is still operationally important: the state was acquired after degraded network behavior.
But successful state should not need failure-specific fields beyond neutral attempt records.
```

If later too noisy:

```rust
NetAcquireEvidence {
    url,
    final_path,
    status,
    bytes,
    content_length,
    attempt_count,
}
```

and detailed attempt records live only in optional diagnostics.

Do not do this before error boundary tests exist.

### Slice 3 — Budget/rate resource behavior

Goal:

```text
resource controls constrain retry loops without hidden globals.
```

Inputs from research:

```text
governor is mature and useful, but should not be hidden in global state.
Pulith should first define explicit budget resources before choosing governor internally.
```

Design first:

```rust
pub struct NetBudgetPolicy {
    pub max_attempts: Option<u32>,
    pub max_sleep: Option<Duration>,
    pub max_elapsed: Option<Duration>,
}
```

or split:

```text
policy = per-operation limits
resource = shared limiter/token source
```

Potential resource shape:

```rust
pub trait NetBudget {
    fn before_attempt(&self, planned_attempt: u32) -> Result<(), PulithError>;
    fn before_sleep(&self, duration: Duration) -> Result<(), PulithError>;
}
```

But avoid public trait until needed. First slice can be concrete:

```rust
NetAcquirePolicy::max_total_delay(Duration)
```

Tests:

```text
retry_stops_before_exceeding_total_sleep_budget
budget_exhaustion_error_carries_attempt_records
budget_does_not_touch_destination
```

Do not use governor until:

```text
shared multi-operation rate limit is required and tested.
```

### Slice 4 — Range/resume design report before code

Do not code Range/resume immediately.

Required knowledge from MDN:

```text
Range success -> 206 Partial Content.
Invalid range -> 416 Range Not Satisfiable.
Server may ignore Range and return 200 OK.
If-Range true -> 206 partial; If-Range false -> 200 full body.
Validators: ETag or Last-Modified.
```

Design states before implementation:

```rust
struct PartialDownload<Validated> { ... }
struct PartialDownload<UnknownValidator> { ... }
struct ResumedDownload<Closed> { ... }
```

or simpler first:

```text
No persisted partial cache yet.
If streaming fails, retry restarts from byte 0.
```

Resume requires separate policy:

```rust
NetResumePolicy {
    enabled: bool,
    validator: ResumeValidatorPolicy,
    max_partial_bytes: Option<u64>,
}
```

Tests before code:

```text
server_returns_206_appends_to_partial
server_ignores_range_200_restarts_full_download
if_range_mismatch_200_restarts_full_download
range_416_deletes_partial_and_restarts_or_errors_by_policy
resume_never_persists_unvalidated_partial_as_final
```

### Slice 5 — Optional diagnostics/report formatting

Only after typed errors are stable, decide whether human diagnostics need richer formatting.

Options:

```text
manual Display/Error: enough for library now
thiserror: reduce boilerplate, no public API dependency
miette: only for CLI-facing annotated diagnostics, not core library yet
```

Recommendation:

```text
Stay manual for now.
Do not add miette.
Maybe add thiserror only if PulithError/NetAcquireError boilerplate grows after behavior-specific errors split.
```

## Revised immediate next action

The next implementation task should be:

```text
Implement NetAcquireError and rename NetAttemptEvidence to NetAttemptRecord.
```

Acceptance:

```text
1. RED tests prove failed retries return attempt records through error.
2. Successful Acquire still returns Acquired state with success evidence.
3. Destination guard errors have empty attempts.
4. Final non-retry status error carries final attempt record.
5. No new global state, no generic retry executor, no miette/governor/range dependencies.
6. Fresh ad-hoc verification under F:\Stratum\TEMP\hermes-verify-*.
```

## Updated next-step order

```text
1. NetAcquireError + NetAttemptRecord rename.
2. Decide/trim success evidence attempts only after error path is correct.
3. Add explicit sleep/attempt budget policy.
4. Research/write Range/resume design with validators and partial state.
5. Implement Range/resume only after design acceptance.
```

This order preserves the corrected principle:

```text
successful state records successful transition;
error captures failure information;
resource policy constrains execution;
resume introduces new partial-state behavior only after failure/error boundaries are correct.
```
