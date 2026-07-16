# Pulith Net Retry Next Slice Plan

## Status

This is a planning/research pass for the next implementation slice after the behavior-first API/resource cleanup.

No production code is changed by this report.

## Current baseline

Implemented and freshly verified before this planning pass:

```text
RemoteUrl is private-invariant and http/https-only.
RemoteSource fields are crate-visible with public accessors.
Chosen/Acquired/Verified/Prepared/Applied/Remembered are behavior-constructed states.
UreqResource and ReqwestResource expose explicit from_agent/from_client resource constructors.
No Tokio runtime is owned by Pulith library resources.
ureq and reqwest Acquire stream to same-parent staged files.
Known content-length oversize fails before staging/streaming.
```

Current Acquire shapes:

```text
sync:
Chosen<I, RemoteSource>
  -> UreqAcquire<UreqResource>
  -> Acquired<I, LocalMaterial, NetAcquireEvidence>

async Tokio-backed:
Chosen<I, RemoteSource>
  -> ReqwestAcquire<ReqwestResource>
  -> Acquired<I, LocalMaterial, NetAcquireEvidence>
```

Current `NetAcquireEvidence`:

```rust
pub struct NetAcquireEvidence {
    pub url: url::Url,
    pub final_path: PathBuf,
    pub status: u16,
    pub bytes: u64,
    pub content_length: Option<u64>,
}
```

Current `NetAcquirePolicy`:

```rust
pub struct NetAcquirePolicy {
    pub timeout: Option<Duration>,
    pub max_bytes: Option<u64>,
    pub headers: Vec<(String, String)>,
}
```

## Research performed

### Current code/reports inspected

```text
crates/pulith/src/net.rs
crates/pulith/src/error.rs
crates/pulith/Cargo.toml
docs/report/pulith-behavior-first-api-resource-execution-report.md
docs/report/pulith-net-next-retry-and-budget-prep.md
docs/report/pulith-current-design-performance-api-review.md
```

### Old implementation inspected as lessons only

```text
crates/pulith-fetch/src/rate/backoff.rs
crates/pulith-fetch/src/config/fetch_options.rs
crates/pulith-fetch/src/fetch/fetcher.rs
```

Old fetcher findings:

```text
RetryPolicy default was max_retries=3, base_backoff=100ms.
Retry loop retried only Network/Timeout errors.
retry_delay used base * 2^retry_count with saturating math.
Async delay provider was injectable.
Old fetcher mixed retry, progress, HEAD, checksum, resume, workspace commit, and receipt.
```

Useful lesson:

```text
Keep saturating backoff and injectable delay.
Do not copy old Fetcher shape; retry belongs inside typed Net Acquire evidence/policy, not a monolithic fetch workflow.
```

### Crates searched

Commands run:

```text
cargo search --registry crates-io retry-after --limit 5
cargo info --registry crates-io httpdate
cargo info --registry crates-io backoff
cargo info --registry crates-io retry
cargo info --registry crates-io tower
cargo info --registry crates-io governor
cargo info --registry crates-io reqwest-retry-after
cargo info --registry crates-io retry-after
```

Findings:

```text
httpdate 1.0.3
  Small HTTP date parsing/formatting crate.
  MIT/Apache-2.0.
  Rust 1.56.
  Good candidate for Retry-After HTTP-date parsing.

backoff 0.4.0
  Retry operations with exponential backoff policy.
  Optional tokio/async-std integrations.
  Do not use now: would hide Pulith attempt evidence and resource delay boundary.

retry 2.2.0
  General retry utilities, default random feature.
  Do not use now: retry math is small; evidence and backend semantics are Pulith-owned.

tower 0.5.3
  Service middleware with retry/timeout/limit features.
  Do not use now: Service stack is the wrong public abstraction for typed behavior morphisms.

governor 0.10.4
  Mature rate limiter with std/dashmap/jitter/quanta defaults.
  Defer: useful for later rate/budget work, not the first retry slice.

reqwest-retry-after 0.2.1
  Retry-After support for reqwest.
  Do not use now: reqwest-specific; Pulith needs shared ureq/reqwest policy/evidence.

retry-after 0.4.0
  Old Hyper header-module helper.
  Do not use now: old/narrow; prefer httpdate + small Pulith parser.
```

## Design decision

Implement **explicit backend-common retry** next.

Do not implement yet:

```text
resume / Range
multi-source race
object_store
bandwidth governor
Tower middleware
progress callbacks
HEAD preflight
checksum-in-acquire
```

Reason:

```text
Retry now has enough behavioral foundation: explicit resources, streaming staging, public evidence, and behavior-constructed states.
Retry is also smaller and less stateful than Range/resume or bandwidth budgets.
```

## Retry behavior laws

### Default law

Retry is disabled by default.

```text
NetAcquirePolicy::default().retry == NetRetryPolicy::disabled()
```

Rationale:

```text
Retry amplifies traffic and latency. It must be caller opt-in.
```

### Attempt law

```text
total attempts = 1 + max_retries
attempt index starts at 0
retry count means attempts after the initial attempt
```

### Staging law

Each attempt must use fresh operation staging:

```text
ureq: new request + new NamedTempFile per attempt
reqwest: new request + new StagedDownload<Open> per attempt
```

Never reuse a partially-written temp file between attempts in this slice.

Reason:

```text
Partial material from a failed stream is not valid acquired material.
Resume requires Range validators and is a later behavior.
```

### Persist law

Only the final successful attempt persists to the destination.

Failed attempts must not touch/replace the destination.

### Retryable failures

First conservative retry set:

```text
HTTP 408 Request Timeout
HTTP 429 Too Many Requests
HTTP 500 Internal Server Error
HTTP 502 Bad Gateway
HTTP 503 Service Unavailable
HTTP 504 Gateway Timeout
network send/call/stream errors before final persist
```

Do not retry:

```text
400/401/403/404/409/412/416
UnsupportedUrlScheme / InvalidUrl
DownloadLimitExceeded
UnsupportedLocalEntry / unsafe destination
create temp / write temp / flush / persist local IO failures
hash/archive/apply errors
```

The line between network stream error and local write error matters:

```text
reqwest response.chunk().await error => retryable network failure
stage.write_chunk(...) IO error => local/staging failure, do not retry
ureq reader.read(...) currently maps to NetworkError, but the writer.write_all(...) maps to Io and must stay non-retryable
```

## Proposed public types

### NetRetryPolicy

Add to `net.rs`:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetRetryPolicy {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Option<Duration>,
    pub respect_retry_after: bool,
}
```

Constructors/builders:

```rust
impl NetRetryPolicy {
    pub fn disabled() -> Self;
    pub fn exponential(max_retries: u32, base_delay: Duration) -> Self;
    pub fn max_delay(mut self, max_delay: Duration) -> Self;
    pub fn respect_retry_after(mut self, respect: bool) -> Self;
}
```

Default:

```rust
impl Default for NetRetryPolicy {
    fn default() -> Self {
        Self::disabled()
    }
}
```

Why include `max_delay` now:

```text
Backoff without a cap is hostile with large retry counts.
A cap is simple and avoids baking an unbounded public policy.
```

Do not add jitter now:

```text
Jitter is useful for fleet-wide clients, but it introduces randomness and evidence/test complexity.
Add deterministic policy first.
```

### NetAttemptEvidence

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
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

### NetAttemptOutcome

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
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

Keep outcome coarse. Do not embed raw error strings in evidence yet unless a test requires it.

### NetAcquireEvidence extension

Change to:

```rust
pub struct NetAcquireEvidence {
    pub url: url::Url,
    pub final_path: PathBuf,
    pub status: u16,
    pub bytes: u64,
    pub content_length: Option<u64>,
    pub attempts: Vec<NetAttemptEvidence>,
}
```

Compatibility note:

```text
This is a public struct shape change. Pulith is still pre-stable in this migration; no shim is needed.
```

## Delay providers in resources

### Sync ureq

```rust
pub type SyncDelay = Arc<dyn Fn(Duration) + Send + Sync>;
```

Add to resource:

```rust
pub struct UreqResource {
    agent: ureq::Agent,
    delay: SyncDelay,
}
```

Constructors:

```rust
impl UreqResource {
    pub fn from_agent(agent: ureq::Agent) -> Self;
    pub fn with_delay(mut self, delay: SyncDelay) -> Self;
    pub fn delay(&self) -> &SyncDelay;
}
```

Default delay:

```rust
Arc::new(std::thread::sleep)
```

Test delay:

```rust
Arc::new(|_| {})
```

### Async reqwest

```rust
pub type AsyncDelayFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
pub type AsyncDelay = Arc<dyn Fn(Duration) -> AsyncDelayFuture + Send + Sync>;
```

Add to resource:

```rust
pub struct ReqwestResource {
    client: reqwest::Client,
    delay: AsyncDelay,
}
```

Constructors:

```rust
impl ReqwestResource {
    pub fn from_client(client: reqwest::Client) -> Self;
    pub fn with_delay(mut self, delay: AsyncDelay) -> Self;
    pub fn delay(&self) -> &AsyncDelay;
}
```

Default delay:

```rust
Arc::new(|duration| Box::pin(tokio::time::sleep(duration)))
```

This is acceptable because `reqwest` already implies `runtime-tokio` in this crate. It still does not create or own a Tokio runtime.

Test delay:

```rust
Arc::new(|_| Box::pin(async {}))
```

## Retry-After parser

Add optional dependency:

```toml
httpdate = { workspace = true, optional = true }
```

Feature wiring:

```toml
net = ["local", "dep:url", "dep:httpdate"]
```

Alternative:

```text
Make `httpdate` non-optional in pulith because net requires it.
```

Preferred:

```text
Optional workspace dependency activated by net.
```

Parser shape:

```rust
fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let retry_at = httpdate::parse_http_date(value).ok()?;
    retry_at.duration_since(now).ok()
}
```

Notes:

```text
Past HTTP-date => None or zero. Prefer zero for explicit immediate retry? First slice should use None to avoid tight retry surprises.
Invalid header => ignore and use policy backoff.
Use SystemTime::now inside backend unless tests inject a helper call directly.
```

## Backoff function

```rust
fn retry_delay(policy: NetRetryPolicy, retry_index: u32) -> Duration {
    let raw = policy.base_delay.saturating_mul(2_u32.saturating_pow(retry_index));
    match policy.max_delay {
        Some(max) => raw.min(max),
        None => raw,
    }
}
```

Decision order:

```text
if Retry-After is present, valid, and policy.respect_retry_after:
    planned_delay = retry_after
else:
    planned_delay = retry_delay(policy, retry_index)
```

Cap interaction:

```text
First slice: cap applies only to exponential backoff, not Retry-After.
Reason: server Retry-After is an explicit server instruction.
If this is too risky later, add max_retry_after separately.
```

## Implementation shape

### Split single-attempt helpers first

Before writing retry loops, extract current backend bodies into single-attempt helpers.

Sync:

```rust
fn acquire_ureq_attempt<I>(
    resources: &UreqResource,
    node_input: I,
    source: &RemoteSource,
    attempt: u32,
) -> Result<AttemptResult<I>, PulithError>
```

But avoid over-abstracting around `I` ownership. Better concrete shape:

```rust
fn acquire_ureq_once(
    resources: &UreqResource,
    source: &RemoteSource,
    parent: &Path,
) -> Result<NetAttemptSuccess, NetAttemptFailure>
```

Then the outer behavior owns `node.input` and final `Acquired::from_acquire(...)`.

Async:

```rust
async fn acquire_reqwest_once(
    client: reqwest::Client,
    source: &RemoteSource,
    parent: &Path,
) -> Result<NetAttemptSuccess, NetAttemptFailure>
```

### Internal attempt result types

Private internal structs:

```rust
struct NetAttemptSuccess {
    status: u16,
    bytes: u64,
    content_length: Option<u64>,
}

struct NetAttemptFailure {
    evidence: NetAttemptEvidence,
    retryable: bool,
    error: PulithError,
}
```

This keeps public evidence clean and keeps retry loop logic readable.

### Ureq retry loop

Pseudo-flow:

```text
resolve parent
create parent dir once
reject unsafe destination once
attempts = Vec::new()
for attempt in 0..=max_retries:
    result = acquire_ureq_once(...)
    if success:
        push success evidence
        return Acquired(... attempts)
    push failure evidence
    if !retryable or attempt == max_retries:
        return original/final error
    delay = decide_delay(...)
    update last attempt evidence planned_delay
    resources.delay(delay)
```

Parent creation/unsafe destination checks stay outside attempts:

```text
They are local preconditions, not transient network operations.
```

### Reqwest retry loop

Same logic but async:

```text
(resources.delay)(delay).await
```

Pass `resources.client.clone()` per attempt.

## Required tests

### Pure tests

```text
net_retry_policy_default_is_disabled
retry_delay_exponential_saturating_and_capped
parse_retry_after_accepts_seconds
parse_retry_after_accepts_http_date
parse_retry_after_rejects_invalid_or_past_date
should_retry_status_is_conservative
```

### Ureq tests

```text
ureq_retries_503_then_succeeds
ureq_does_not_retry_404
ureq_respects_retry_after_seconds_with_injected_delay
ureq_records_attempt_evidence
ureq_does_not_sleep_when_retry_disabled
ureq_does_not_retry_download_limit_exceeded
ureq_uses_fresh_temp_each_attempt_and_only_persists_success
```

### Reqwest tests

```text
reqwest_retries_503_then_succeeds
reqwest_does_not_retry_404
reqwest_respects_retry_after_seconds_with_injected_delay
reqwest_records_attempt_evidence
reqwest_does_not_sleep_when_retry_disabled
reqwest_does_not_retry_download_limit_exceeded
```

Test server requirement:

```text
Current serve_once only handles one request.
Retry tests need a small sequential server that accepts N connections and returns scripted responses.
```

Suggested test helper:

```rust
fn serve_sequence(responses: Vec<TestResponse>) -> TestServer
```

Where:

```rust
struct TestResponse {
    status: u16,
    body: &'static [u8],
    headers: &'static [(&'static str, &'static str)],
}
```

Also capture request count and possibly requested paths/headers if needed.

## Error strategy

Do not add `MaxRetriesExceeded` yet unless tests prove the final error is ambiguous.

Preferred first-slice behavior:

```text
Return the final attempt's PulithError.
Use `NetAcquireEvidence.attempts` on success to inspect retries.
On failure no evidence is returned today, so MaxRetriesExceeded would only wrap without a typed failure output.
```

If failure evidence becomes necessary, that is a separate behavior/API decision:

```text
NetAcquireFailure { error, attempts }
```

Do not add it in this slice.

## Avoided designs

### Do not add retry to resource only

Bad:

```rust
ReqwestResource { client, retry_policy }
```

Reason:

```text
Retry policy is operation policy. Different RemoteSource values may need different retry policy.
Delay provider is resource/execution capability. Put delay in resource.
```

### Do not add runtime to resource

Bad:

```rust
ReqwestResource { runtime, client }
```

Reason:

```text
Runtime is caller-owned execution context. Reqwest resource is only shared transport/delay capability.
```

### Do not copy old FetchOptions

Bad:

```text
checksum + retry + progress + resume + expected_bytes + headers in one fetch bag
```

Reason:

```text
Checksum is Verify, resume is separate Range behavior, progress is observability, retry is Net Acquire policy.
```

### Do not use Tower/backoff/retry crate first

Reason:

```text
The hard part is Pulith evidence/state/resource semantics, not writing the retry loop.
```

## Slice order

### Slice A: pure policy and evidence

```text
Add NetRetryPolicy to net.rs.
Add NetAttemptEvidence / NetAttemptOutcome.
Extend NetAcquirePolicy with retry: NetRetryPolicy.
Extend NetAcquireEvidence with attempts: Vec<NetAttemptEvidence>.
Add retry_delay, should_retry_status, parse_retry_after.
Add httpdate dependency through net feature.
Pure tests only.
```

### Slice B: sync ureq retry

```text
Add SyncDelay to UreqResource.
Extract acquire_ureq_once.
Implement retry loop.
Add serve_sequence test helper.
Add ureq retry tests.
```

### Slice C: async reqwest retry

```text
Add AsyncDelay to ReqwestResource.
Extract acquire_reqwest_once.
Implement async retry loop.
Reuse serve_sequence helper.
Add reqwest retry tests.
```

### Slice D: cleanup and report

```text
Run fresh ad-hoc verification with temporary F:\Stratum\TEMP\hermes-verify-* script.
Update execution report.
Patch skill reference if implementation reveals new gotchas.
```

## Fresh verification command set for implementation

Use a temporary script under `F:\Stratum\TEMP` with prefix `hermes-verify-`.

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
git diff --check -- changed paths
```

Structural checks:

```text
NetRetryPolicy exists and defaults disabled.
NetAcquirePolicy includes retry policy.
NetAttemptEvidence exists.
NetAcquireEvidence includes attempts.
retry_delay is saturating and capped.
parse_retry_after uses httpdate.
UreqResource has injected sync delay.
ReqwestResource has injected async delay.
Retry tests use no-op injected delays.
No Tokio runtime is stored or created by library code.
No Range/resume code added.
No Tower/backoff/retry dependency added.
```

## Recommendation

Proceed with **Slice A only first**.

Reason:

```text
Slice A locks the public policy/evidence vocabulary and can be verified with pure tests.
It avoids touching backend loops until the behavior vocabulary is correct.
```

Only after Slice A is green should we implement ureq, then reqwest.
