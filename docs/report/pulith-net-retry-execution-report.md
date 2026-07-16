# Pulith Net Retry Execution Report

## Status

Implemented the planned net retry slice with bounded public surface and limited helper proliferation.

This slice changes production code and tests.

## Scope

Implemented:

- `NetRetryPolicy` as opt-in operation policy.
- `NetAttemptEvidence` and `NetAttemptOutcome` for visible retry history.
- `NetAcquirePolicy.retry(...)` builder.
- `NetAcquireEvidence.attempts`.
- injected delay resources for sync `ureq` and Tokio-backed `reqwest`.
- retry loops for `UreqAcquire` and `ReqwestAcquire`.
- Retry-After parsing through `httpdate`.
- focused tests for policy, sync retry, and async retry.

Not implemented:

- Range/resume.
- HEAD preflight.
- checksum/archive/apply retry.
- global budget/rate limiting.
- hidden backend middleware.
- failure evidence carried out of final `Err`.

## Function-count discipline

The implementation intentionally keeps helper count small.

New production helpers are limited to retry-specific pure logic:

```text
retry_delay
planned_retry_delay
should_retry_status
parse_retry_after
```

No Tower middleware, no new retry executor abstraction, no request builder factory, no FetchOptions-style bag, and no backend-specific retry framework were introduced.

One test helper was added:

```text
serve_sequence
```

`serve_once` now delegates to `serve_sequence`, so sequence tests reuse the existing local TCP server shape.

## Public API additions

### Policy

```rust
pub struct NetRetryPolicy {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Option<Duration>,
    pub respect_retry_after: bool,
}
```

Default remains disabled:

```rust
NetRetryPolicy::disabled()
NetAcquirePolicy::default().retry == NetRetryPolicy::disabled()
```

Builder methods:

```rust
NetRetryPolicy::disabled()
NetRetryPolicy::exponential(max_retries, base_delay)
.max_delay(max_delay)
.respect_retry_after(bool)

NetAcquirePolicy::default().retry(policy)
```

### Evidence

```rust
pub struct NetAttemptEvidence {
    pub attempt: u32,
    pub status: Option<u16>,
    pub bytes: u64,
    pub content_length: Option<u64>,
    pub retry_after: Option<Duration>,
    pub planned_delay: Option<Duration>,
    pub outcome: NetAttemptOutcome,
}
```

```rust
pub enum NetAttemptOutcome {
    Success,
    RetryableStatus,
    RetryableNetworkError,
    NonRetryableStatus,
    NonRetryableNetworkError,
    LocalFailure,
    LimitExceeded,
}
```

`NetAcquireEvidence` now includes:

```rust
pub attempts: Vec<NetAttemptEvidence>
```

These types are re-exported from `lib.rs` behind `feature = "net"`.

## Resource design

### Sync ureq

`UreqResource` now owns:

```text
ureq::Agent
SyncDelay
```

Delay type:

```rust
pub type SyncDelay = Arc<dyn Fn(Duration) + Send + Sync>;
```

Default delay:

```rust
std::thread::sleep
```

Injected delay is available for tests and callers that need custom scheduling:

```rust
UreqResource::default().with_delay(delay)
UreqResource::delay()
```

### Tokio-backed reqwest

`ReqwestResource` now owns:

```text
reqwest::Client
AsyncDelay
```

Delay type:

```rust
pub type AsyncDelayFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
pub type AsyncDelay = Arc<dyn Fn(Duration) -> AsyncDelayFuture + Send + Sync>;
```

Default delay:

```rust
tokio::time::sleep
```

This preserves the runtime boundary:

- `ReqwestResource` does not create or own a Tokio runtime.
- The delay future is awaited inside the caller-owned Tokio execution context.
- `reqwest::Client` remains the shared HTTP resource.

## Retry behavior

Attempts are indexed from zero.

Total attempts:

```text
1 + max_retries
```

Retryable statuses:

```text
408, 429, 500, 502, 503, 504
```

Non-retry statuses remain final errors, including:

```text
400, 401, 403, 404, 409, 412, 416
```

Retryable network failures:

- request send/call errors;
- response body stream/read errors.

Non-retry failures:

- URL/scheme validation;
- destination guard failures;
- temp creation / local write / flush / persist errors;
- download limit exceeded;
- downstream hash/archive/apply failures.

## Retry-After

Added workspace dependency:

```toml
httpdate = "1.0.3"
```

`net` feature now includes:

```toml
net = ["local", "dep:url", "dep:httpdate"]
```

Parser behavior:

- integer seconds accepted;
- HTTP-date accepted through `httpdate::parse_http_date`;
- invalid values ignored;
- past HTTP-date ignored;
- if `respect_retry_after` is true, Retry-After wins over exponential delay.

## Fresh staging law

Each retry attempt rebuilds the HTTP request and creates its own staged temp file.

Only a successful final attempt persists to destination.

No Range/resume or partial file reuse was introduced.

## Tests added

Pure policy/parser test:

```text
retry_policy_is_disabled_by_default_and_computes_delay
```

Sync retry test:

```text
ureq_retries_retryable_status_and_records_attempts
```

Async retry test:

```text
reqwest_retries_retryable_status_and_records_attempts
```

Both backend retry tests use `serve_sequence` with:

```text
503 Retry-After: 2
200 OK
```

and injected no-op/capturing delay resources to avoid real sleeps.

## Quality notes

The retry loop is intentionally explicit in each backend. There is some repeated control flow between ureq and reqwest, but it avoids introducing a generic retry executor or middleware layer before the sync/async differences settle.

This matches the requested constraint: fewer new functions, clear behavior, and no over-abstracted helper stack.

## Files changed

```text
Cargo.toml
crates/pulith/Cargo.toml
crates/pulith/src/lib.rs
crates/pulith/src/net.rs
docs/report/pulith-net-retry-execution-report.md
```

## Verification

Fresh ad-hoc verification passed.

Script:

```text
F:\Stratum\TEMP\hermes-verify-99h1hrfs.py
```

Cleanup:

```text
AD_HOC_SCRIPT_CLEANED=F:\Stratum\TEMP\hermes-verify-99h1hrfs.py
```

Marker:

```text
AD_HOC_VERIFY_PASS pulith net retry execution final
```

Commands:

```text
cargo fmt --all --check
cargo check -p pulith --no-default-features
cargo check -p pulith --features "sync local net ureq hash blake3"
cargo check -p pulith --features "async net reqwest hash blake3"
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::
cargo test -p pulith --features "async net reqwest hash blake3" net::tests::reqwest
cargo test -p pulith --features "sync local hash blake3 sha2"
cargo check --workspace --all-features
cargo test --workspace --all-features
git diff --check -- Cargo.toml crates/pulith/Cargo.toml crates/pulith/src/lib.rs crates/pulith/src/net.rs docs/report/pulith-net-retry-execution-report.md
```

Summary:

```text
sync ureq net tests: 10 passed; 0 failed
async reqwest net tests: 6 passed; 0 failed
local/hash tests: 9 passed; 0 failed
workspace all-features tests: 46 passed; 0 failed
git diff --check: passed
```

## Next-step plan

### Next slice A: failure evidence carrier

Current `NetAcquireEvidence.attempts` is available only on success. If all attempts fail, the returned `PulithError` does not carry attempt evidence.

Next design should add an explicit failure carrier without bloating every error variant, for example:

```rust
pub struct NetAcquireFailure {
    pub error: PulithError,
    pub attempts: Vec<NetAttemptEvidence>,
}
```

Then decide whether `AcquireNode::Error` for net should remain `PulithError` or become a net-specific error wrapper.

### Next slice B: budget/rate behavior

After failure evidence, add explicit budget behavior:

```text
max total attempts across sources
max total sleep
optional request budget token
```

Do not use hidden global governor.

### Next slice C: Range/resume

Defer until retry/failure evidence is stable.

Range/resume requires independent state laws:

```text
partial temp state
ETag / Last-Modified validators
If-Range
200 vs 206 vs 416 branching
corruption-safe restart path
```
