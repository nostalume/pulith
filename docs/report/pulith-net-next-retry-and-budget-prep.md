# Pulith Net Acquire Next Slice Preparation: Retry, Budget, and Resume Boundary

## Status

This is a planning/research pass after the completed sync `ureq` and Tokio-backed `reqwest` Acquire slices.

No production code is changed in this pass.

Current implemented net Acquire paths:

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

Current constraints already satisfied:

```text
RemoteUrl accepts only http/https.
RemoteSource carries selected URL/destination/policy facts.
Both backends reuse explicit client/agent resources.
Both backends stage downloads before final persist.
Reqwest uses private StagedDownload<Open/Closed> typestate.
Hash verification and LocalApply remain later typed behaviors.
```

## Sources reviewed in this pass

Current Pulith files/reports:

```text
crates/pulith/src/net.rs
crates/pulith/Cargo.toml
Cargo.toml
docs/report/pulith-reqwest-tokio-backed-acquire-execution-report.md
docs/report/pulith-net-acquire-execution-detail-plan.md
```

Skill references:

```text
references/pulith-sync-net-acquire.md
references/pulith-async-runtime-resource-control.md
```

Crates searched:

```bash
cargo info --registry crates-io backoff
cargo info --registry crates-io retry
cargo info --registry crates-io tower
cargo info --registry crates-io governor
cargo info --registry crates-io httpdate
cargo info --registry crates-io retry-after
```

HTTP docs searched:

```text
MDN Retry-After
MDN 429 Too Many Requests
MDN 503 Service Unavailable
MDN Range
MDN Accept-Ranges
MDN If-Range
reqwest header docs/source
ureq response/source
```

Old `pulith-fetch` files reviewed as lessons only:

```text
crates/pulith-fetch/src/rate/backoff.rs
crates/pulith-fetch/src/config/fetch_options.rs
crates/pulith-fetch/src/fetch/fetcher.rs
```

## Research findings

### Retry libraries

#### `backoff`

Findings:

```text
Retry operations with exponential backoff policy.
Features include tokio and async-std integrations.
```

Assessment:

```text
Do not add now.
```

Reason:

```text
Pulith needs backend-typed evidence, explicit retry decisions, and injected sleep/no-sleep tests.
A generic retry crate would hide too much of the domain evidence and runtime boundary.
```

#### `retry`

Findings:

```text
Small utility crate for retrying operations.
Default feature includes random.
```

Assessment:

```text
Do not add now.
```

Reason:

```text
The retry math is small and domain-specific; adding the crate would not solve evidence, Retry-After, or sync/async delay injection.
```

#### `tower`

Findings:

```text
Tower has retry, timeout, limit, buffer, load-shed, and related Service middleware.
Many features are Tokio-oriented.
```

Assessment:

```text
Do not use Tower inside Pulith Acquire now.
```

Reason:

```text
Pulith's public model is typed behavior morphisms, not Service stacks.
Tower would introduce another abstraction layer before the domain boundary requires it.
It may become useful if Pulith later exposes a service/server layer, not for this slice.
```

#### `governor`

Findings:

```text
Mature rate-limiting implementation.
Default includes std/dashmap/jitter/quanta.
```

Assessment:

```text
Defer.
```

Reason:

```text
Pulith first needs a small explicit budget/permit shape. Governor is useful for bandwidth/rate limit later, but initial concurrency and retry controls can be simpler.
```

#### `httpdate`

Findings:

```text
Small crate for HTTP date parsing and formatting.
Rust version 1.56.
```

Assessment:

```text
Good candidate if implementing full Retry-After support.
```

Reason:

```text
Retry-After allows either delay-seconds or HTTP-date. Parsing HTTP-date correctly should use a mature tiny crate rather than hand parsing dates.
```

#### `retry-after`

Findings:

```text
Retry-After header helper for Hyper's old header module.
Documentation URL is old; crate is old.
```

Assessment:

```text
Do not add.
```

Reason:

```text
It is narrower/older than needed. Use `httpdate` plus small Pulith parsing logic instead.
```

## HTTP semantic findings

### Retry-After

MDN confirms:

```text
Retry-After can appear on 503 Service Unavailable.
Retry-After can appear on 429 Too Many Requests.
Retry-After syntax is either <delay-seconds> or <http-date>.
Redirect responses may also use Retry-After, but Pulith currently lets clients handle redirects.
```

Implication:

```text
First retry slice should support Retry-After for 429/503 and transient failures.
It should not invent redirect timing behavior.
```

### Retryable status classes

Conservative first set:

```text
408 Request Timeout
429 Too Many Requests
500 Internal Server Error
502 Bad Gateway
503 Service Unavailable
504 Gateway Timeout
```

Do not retry by default:

```text
400/401/403/404/409/412/416 and other client or semantic errors
```

Reason:

```text
These are usually request/material/condition failures, not transient transport failures.
```

### Range/resume

MDN confirms:

```text
Range success returns 206 Partial Content.
Invalid range returns 416 Range Not Satisfiable.
Server may ignore Range and return 200 with the whole resource.
If-Range makes resume conditional on ETag or Last-Modified.
```

Implication:

```text
Do not implement resume next.
```

Reason:

```text
Resume needs durable partial material state, validator evidence, 200-vs-206 recovery, 416 recovery, and possibly Remember behavior. It is not a small policy flag.
```

## Old `pulith-fetch` lessons

Useful lessons:

```text
RetryPolicy { max_retries, base_backoff } is small and useful.
retry_delay uses saturating exponential backoff.
Retry delay provider injection helps avoid sleeping in tests.
Resume offset and progress callbacks should not be pulled into first retry slice.
```

Rejected old API concepts:

```text
FetchOptions
FetchPhase
ProgressCallback
FetchReceipt
Fetcher loop as public choreography
checksum inside fetch
resume_offset in first retry slice
```

## Next-slice recommendation

Recommended next implementation:

```text
Backend-common explicit retry policy + attempt evidence, with injected delay provider and no default hidden global sleep in tests.
```

Not recommended as next:

```text
Range/resume
object_store
multi-source race
bandwidth governor
Tower middleware
runtime-neutral isahc backend
```

Reasoning:

```text
1. Both sync and async HTTP backends now exist.
2. Retry semantics can now be shaped once and applied to both backends.
3. Retry is lower complexity than resume and more immediately useful.
4. Retry must be explicit and evidence-rich to avoid old fetcher-style hidden behavior.
5. Delay/sleep must be injected through resources so tests and callers control runtime effects.
```

## Design goals for retry slice

### Keep behavior/backend/runtime axes separate

```text
net behavior: Acquire
modality: sync / async
runtime: runtime-tokio only where needed
HTTP backend: ureq / reqwest
policy: retry policy owned by RemoteSource/NetAcquirePolicy
resource: agent/client plus delay provider/budget
```

Do not introduce:

```text
fetch feature
runtime switch strings
global retry executor
old FetchOptions compatibility
```

### Retry is explicit, not default hidden magic

Recommended default:

```text
NetRetryPolicy::disabled()
```

Caller opts in with:

```rust
NetAcquirePolicy::default().retry(NetRetryPolicy::exponential(...))
```

Rationale:

```text
Retries can amplify load and delay failure. Default hidden retry would violate resource-control expectations.
```

### Evidence must record attempts

Extend `NetAcquireEvidence` with attempt facts, not just final status:

```rust
pub struct NetAttemptEvidence {
    pub attempt: u32,
    pub status: Option<u16>,
    pub bytes: u64,
    pub retry_after: Option<Duration>,
    pub planned_delay: Option<Duration>,
}
```

Then:

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

This is better than a bare `attempts: u32` because failures before final success remain visible.

### Shared delay resources, no hidden sleeps in tests

Sync resource shape:

```rust
pub type SyncDelay = Arc<dyn Fn(Duration) + Send + Sync>;

pub struct UreqResource {
    pub agent: ureq::Agent,
    pub delay: SyncDelay,
}
```

Async Tokio-backed resource shape:

```rust
pub type AsyncDelayFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
pub type AsyncDelay = Arc<dyn Fn(Duration) -> AsyncDelayFuture + Send + Sync>;

pub struct ReqwestResource {
    pub client: reqwest::Client,
    pub delay: AsyncDelay,
}
```

Defaults:

```text
UreqResource::default(): std::thread::sleep
ReqwestResource::default(): tokio::time::sleep
```

Tests inject no-op delay:

```text
Sync no-op: |_| {}
Async no-op: |_| Box::pin(async {})
```

This keeps real waiting out of tests and avoids hidden global runtime state.

### Delay computation

Add pure function:

```rust
fn retry_delay(policy: &NetRetryPolicy, retry_index: u32) -> Duration
```

Rules:

```text
exponential base * 2^retry_index
saturating arithmetic
clamp to max_backoff when set
Retry-After overrides backoff only when it is lower/equal to policy max_retry_after, or is clamped by that max
```

Suggested policy:

```rust
pub struct NetRetryPolicy {
    pub max_retries: u32,
    pub base_backoff: Duration,
    pub max_backoff: Option<Duration>,
    pub max_retry_after: Option<Duration>,
}
```

Default:

```text
max_retries = 0
base_backoff = 100ms
max_backoff = Some(30s)
max_retry_after = Some(60s)
```

### Retry decision

Add pure decision function:

```rust
fn should_retry_status(status: u16) -> bool
```

Initial true set:

```text
408, 429, 500, 502, 503, 504
```

Transport/read errors:

```text
Retry network/read errors before persist if max_retries remains.
Do not retry persist failure; that is local filesystem/target failure.
Do not retry max_bytes failure; that is caller policy failure.
Do not retry UnsupportedLocalEntry; that is preflight failure.
```

### Retry-After parsing

Use `httpdate` for HTTP-date:

```text
Retry-After: 120
Retry-After: Wed, 21 Oct 2015 07:28:00 GMT
```

Backend extraction:

```text
reqwest Response has headers().
ureq response is http::Response<Body>; headers() is available through http::Response.
```

Do not hand-parse HTTP-date.

## Concrete execution slices

### Slice 1 — pure retry types and functions

Files:

```text
Cargo.toml
crates/pulith/Cargo.toml
crates/pulith/src/net.rs
```

Add optional workspace dependency:

```toml
httpdate = "1.0"
```

Feature wiring:

```toml
net = ["local", "dep:url", "dep:httpdate"]
```

Types/functions:

```text
NetRetryPolicy
NetAttemptEvidence
NetAcquirePolicy::retry(...)
retry_delay(...)
should_retry_status(...)
parse_retry_after(...)
```

Tests:

```text
retry_policy_defaults_to_disabled
retry_delay_saturates_and_clamps
retry_after_parses_delay_seconds
retry_after_parses_http_date
retry_after_rejects_negative_or_invalid
retry_status_set_is_conservative
```

### Slice 2 — evidence extension without behavior change

Update current ureq/reqwest success paths to populate:

```text
attempts = vec![NetAttemptEvidence { attempt: 1, status: Some(200), ... }]
```

Tests update existing assertions.

Goal:

```text
Change evidence shape before adding retry loops, so failures are easier to isolate.
```

### Slice 3 — sync ureq retry loop

Implement retry loop around `UreqAcquire` with:

```text
fresh request per attempt
same destination preflight before attempts
new staged temp per attempt
no final destination touch until successful attempt
manual status decision
SyncDelay resource injection
```

Tests:

```text
ureq_retries_503_then_succeeds
ureq_respects_retry_after_with_injected_delay
ureq_does_not_retry_404
ureq_does_not_retry_max_bytes
ureq_records_attempt_evidence
```

### Slice 4 — async reqwest retry loop

Implement same policy around `ReqwestAcquire` with:

```text
fresh request per attempt
new StagedDownload per attempt
AsyncDelay resource injection
no hidden tokio runtime creation
```

Tests:

```text
reqwest_retries_503_then_succeeds
reqwest_respects_retry_after_with_injected_delay
reqwest_does_not_retry_404
reqwest_does_not_retry_max_bytes
reqwest_records_attempt_evidence
```

### Slice 5 — fresh verification and report

Ad-hoc script under:

```text
F:\Stratum\TEMP\hermes-verify-*.py
```

Commands:

```text
cargo fmt --all --check
cargo check -p pulith --no-default-features
cargo check -p pulith --features "sync local net ureq"
cargo check -p pulith --features "runtime-tokio"
cargo check -p pulith --features "async net reqwest"
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::retry
cargo test -p pulith --features "async net reqwest hash blake3" net::tests::retry
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::
cargo test -p pulith --features "async net reqwest hash blake3" net::tests::
cargo check --workspace --all-features
cargo test --workspace --all-features
git diff --check -- changed paths
```

Structural markers:

```text
NetRetryPolicy
NetAttemptEvidence
parse_retry_after
httpdate
SyncDelay
AsyncDelay
should_retry_status
retry_delay
ureq_retries_503_then_succeeds
reqwest_retries_503_then_succeeds
```

## Why not resource semaphore first?

A shared concurrency budget is important, but retry should remain explicit and disabled by default. The minimal retry slice can be safe without a semaphore if:

```text
max_retries defaults to 0
retry opt-in is per RemoteSource policy
sleep is resource-injected
attempt evidence is recorded
```

Recommended follow-up after retry:

```text
NetBudget permits for max concurrent transfers per resource.
```

Do not combine full budget/semaphore and retry in one code slice unless the retry slice grows too risky. Keeping them separate preserves reviewability.

## Why not resume next?

Resume must be a later design slice because it needs:

```text
partial material state
Range header construction
If-Range validators from ETag/Last-Modified
200 full response recovery when server ignores Range
206 partial response handling
416 recovery
Remember/persistence boundary for partial attempts
```

Adding resume before retry/evidence would recreate old fetcher complexity.

## Recommended next action

Implement:

```text
NetRetryPolicy + NetAttemptEvidence + backend-common conservative retry
```

in the slice order above.

Keep defaults conservative:

```text
no retry unless caller opts in
no retry on policy/local errors
retry only selected transient status/transport failures
no real sleep in tests
```
