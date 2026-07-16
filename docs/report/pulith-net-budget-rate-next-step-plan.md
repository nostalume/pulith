# Pulith Net Budget / Rate Next-Step Plan

## Status

Planning only. No production code was changed in this slice.

This report prepares the next implementation slice after the net-owned error hierarchy execution. The proposed slice is intentionally narrow:

```text
request concurrency budget + optional request start rate budget
```

It deliberately does **not** implement byte-bandwidth throttling yet. Byte bandwidth requires stream-level pacing around body chunks and should be a later slice after request admission is typed and verified.

## Context

Current net acquire state:

- `NetAcquirePolicy` owns per-source behavior policy:
  - timeout
  - max bytes
  - headers
  - retry
  - resume
- `UreqResource` owns shared sync resources:
  - `ureq::Agent`
  - injected sync delay
- `ReqwestResource` owns shared async resources:
  - `reqwest::Client`
  - injected async delay
- `NetAcquireError` now owns net failure semantics:
  - URL/scheme
  - HTTP status
  - transport phase
  - protocol failure
  - byte limit
  - local/staging failure
  - unsafe destination
- Retry records and resume evidence already exist:
  - `NetAttemptEvidence`
  - `NetResumeEvidence`
  - `NetAcquireEvidence`

This is enough foundation to add request admission budgets without adding another global error or a runtime singleton.

## Current gap

There is no shared admission control for net acquire operations.

Missing behaviors:

1. A caller cannot bound simultaneous remote requests across many `RemoteSource`s that share one resource handle.
2. A caller cannot pace request starts across a shared resource.
3. Evidence does not record admission delay/wait decisions.
4. `NetAcquireError` has no budget/rate variant for admission failure.

Existing controls are not substitutes:

- `max_bytes` is a payload limit, not a concurrency budget.
- retry delay is failure recovery pacing, not request admission.
- timeout is request duration bound, not resource fairness.
- `reqwest::Client` connection pooling is backend-specific and not a Pulith behavior budget.

## Research summary

I searched crates before choosing the implementation shape.

Commands used:

```text
cargo search --registry crates-io governor ratelimit leaky-bucket async-rate-limiter tower
cargo info --registry crates-io governor
cargo info --registry crates-io leaky-bucket
cargo info --registry crates-io async-rate-limiter
cargo info --registry crates-io tokio-util
cargo info --registry crates-io async-semaphore
cargo info --registry crates-io ratelimit_meter
```

Observed crates:

| crate | relevant facts | decision |
|---|---|---|
| `governor 0.10.4` | mature GCRA rate limiter; default features include `std`, `dashmap`, `jitter`, `quanta`; supports async-ish futures utilities | good future candidate for request start rate, but too much dependency/API surface for first request-budget slice |
| `leaky-bucket 1.1.2` | async token-bucket/leaky-bucket; small, MIT/Apache-2.0 | useful future async pacing candidate, but sync ureq parity would still need separate handling |
| `async-rate-limiter 1.1.0` | token bucket with tokio/async-std features | less attractive because it pulls runtime-specific choices into a public design too early |
| `tokio-util 0.7.18` | has useful Tokio utilities/features; not a behavior model | not needed for first slice; Pulith already has Tokio only behind reqwest/runtime-tokio |
| `async-semaphore 1.2.1` | async semaphore crate | not needed; Tokio already provides semaphore if async concurrency is implemented later |
| `ratelimit_meter 5.0.0` | older leaky-bucket meter, related to governor lineage | not preferred over governor |

Conclusion:

```text
Do not add governor/leaky-bucket/tower in the first budget slice.
```

The first slice should implement a tiny Pulith-owned admission trait with test resources:

```text
NetSyncBudget
NetAsyncBudget
```

Default behavior is no budget, so existing callers and tests retain behavior. Later slices can provide governor-backed implementations behind a feature if the small trait proves stable.

## Design principles

### Behavior first

Budget/rate is a behavior of shared net resources, not a field on one response or one attempt.

Therefore:

```text
resource owns reusable budget handles
policy chooses whether a source requires admission
evidence records what happened
errors describe admission failure
```

### Shared resource, not hidden global

Budget state must be carried by `UreqResource` / `ReqwestResource` or an explicit budget resource they own.

Avoid:

```text
static GLOBAL_LIMITER
lazy_static rate state
thread-local implicit budget
```

Prefer:

```rust
let resource = UreqResource::default().with_budget(...);
let acquire = UreqAcquire::with_resource(resource.clone());
```

### Admission before request construction

Admission happens after local preflight and before backend request send.

Order:

```text
destination parent / unsafe destination preflight
planned resume inspection
budget admission
build/send HTTP request
```

Rationale:

- no remote side effects before admission
- no request body opened before admission
- local destination failures are still local failures, not budget failures

### Retry semantics

Admission is per actual outbound attempt.

If a request is retried, each retry must pass admission again.

This means attempt evidence can record admission for each attempt.

### Scope of first slice

Implement:

```text
max concurrent requests
request start delay evidence
budget admission failure error
sync/async parity shape
```

Do not implement:

```text
byte-per-second bandwidth throttling
host-keyed pools
Tower layers
governor-backed public implementation
progress callbacks
object_store integration
```

## Proposed API

### Budget policy on `NetAcquirePolicy`

Add:

```rust
pub struct NetAcquirePolicy {
    pub timeout: Option<Duration>,
    pub max_bytes: Option<u64>,
    pub headers: Vec<(String, String)>,
    pub retry: NetRetryPolicy,
    pub resume: NetResumePolicy,
    pub budget: NetBudgetPolicy,
}
```

Budget policy:

```rust
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NetBudgetPolicy {
    pub request: NetRequestBudgetMode,
}
```

Request budget mode:

```rust
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NetRequestBudgetMode {
    #[default]
    Unbounded,
    Shared,
}
```

Constructors:

```rust
impl NetBudgetPolicy {
    pub fn unbounded() -> Self;
    pub fn shared() -> Self;
}

impl NetAcquirePolicy {
    pub fn budget(mut self, budget: NetBudgetPolicy) -> Self;
}
```

Why not expose `max_concurrent` in `NetAcquirePolicy`?

Because max concurrency is shared state. If stored per source policy, independent sources would each instantiate their own budget and not actually share anything. The budget capacity belongs to the resource handle, not a single source policy.

### Sync budget trait

```rust
#[cfg(feature = "ureq")]
pub trait NetSyncBudget: Send + Sync {
    fn enter(&self) -> Result<NetBudgetPermit, NetBudgetError>;
}
```

### Async budget trait

```rust
#[cfg(feature = "reqwest")]
pub trait NetAsyncBudget: Send + Sync {
    fn enter(&self) -> Pin<Box<dyn Future<Output = Result<NetBudgetPermit, NetBudgetError>> + Send + '_>>;
}
```

Permit:

```rust
pub struct NetBudgetPermit {
    waited: Duration,
}

impl NetBudgetPermit {
    pub fn immediate() -> Self;
    pub fn waited(waited: Duration) -> Self;
    pub fn waited_for(&self) -> Duration;
}
```

The permit is intentionally tiny. It represents completed admission and records observed wait duration. For the first slice it does not need RAII drop semantics unless we implement a real semaphore. If concurrency is implemented with RAII, make the held permit private inside backend code and project only `NetBudgetPermitEvidence` into public evidence.

### No-budget defaults

```rust
pub struct NoNetSyncBudget;
pub struct NoNetAsyncBudget;
```

Both return an immediate permit.

### Simple in-crate sync concurrency budget

First slice can implement only sync concurrency as a tiny std-based resource:

```rust
#[cfg(feature = "ureq")]
pub struct NetSyncConcurrencyBudget {
    state: Arc<(Mutex<usize>, Condvar)>,
    max: usize,
}
```

Public constructor:

```rust
impl NetSyncConcurrencyBudget {
    pub fn new(max: usize) -> Self;
}
```

Behavior:

- `max == 0` should be rejected at construction or represented as `NetBudgetError::InvalidBudget` if fallible constructor is chosen.
- `enter()` blocks until one slot is free.
- returned internal guard releases slot on drop.
- evidence records elapsed wait.

But to avoid public RAII type bloat, prefer this internal split:

```rust
struct HeldSyncBudgetPermit { ... }
```

and public evidence remains:

```rust
NetBudgetEvidence { waited: Duration }
```

### Async concurrency budget

For reqwest, use Tokio when the `reqwest` feature is active because reqwest is already Tokio-backed:

```rust
#[cfg(feature = "reqwest")]
pub struct NetTokioConcurrencyBudget {
    semaphore: Arc<tokio::sync::Semaphore>,
}
```

Constructor:

```rust
impl NetTokioConcurrencyBudget {
    pub fn new(max: usize) -> Self;
}
```

This does not add a dependency because `runtime-tokio` is already required by `reqwest`.

### Resource integration

`UreqResource` gains:

```rust
budget: Arc<dyn NetSyncBudget>
```

Methods:

```rust
impl UreqResource {
    pub fn with_budget(mut self, budget: Arc<dyn NetSyncBudget>) -> Self;
    pub fn budget(&self) -> &Arc<dyn NetSyncBudget>;
}
```

Default:

```rust
budget: Arc::new(NoNetSyncBudget)
```

`ReqwestResource` gains:

```rust
budget: Arc<dyn NetAsyncBudget>
```

Methods:

```rust
impl ReqwestResource {
    pub fn with_budget(mut self, budget: Arc<dyn NetAsyncBudget>) -> Self;
    pub fn budget(&self) -> &Arc<dyn NetAsyncBudget>;
}
```

Default:

```rust
budget: Arc::new(NoNetAsyncBudget)
```

### Evidence

Add budget evidence to each attempt:

```rust
pub struct NetAttemptEvidence {
    pub attempt: u32,
    pub status: Option<u16>,
    pub bytes: u64,
    pub content_length: Option<u64>,
    pub retry_after: Option<Duration>,
    pub planned_delay: Option<Duration>,
    pub budget: Option<NetBudgetEvidence>,
    pub outcome: NetAttemptOutcome,
}
```

Budget evidence:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetBudgetEvidence {
    pub waited: Duration,
}
```

Why per-attempt?

Because retry means multiple outbound attempts. Admission is per outbound attempt, so evidence belongs beside status/retry delay.

### Error

Add to `NetAcquireError`:

```rust
Budget {
    url: url::Url,
    kind: NetBudgetError,
    attempts: Vec<NetAttemptEvidence>,
    resume: Option<NetResumeEvidence>,
}
```

Budget error:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NetBudgetError {
    InvalidBudget,
    Closed,
    Rejected,
}
```

First slice likely only uses:

```text
Closed
Rejected
```

if injected test budgets can fail.

For simple blocking semaphore budgets, normal wait is not an error.

### Attempt outcome

Add:

```rust
NetAttemptOutcome::BudgetRejected
```

But only emit it when admission fails before request send. Do not emit it for waiting; waiting is evidence on the eventual attempt.

## Backend algorithm

### ureq

Inside the attempt loop, after `resume_context` and before constructing/sending the request:

```rust
let budget = if source.policy.budget.request == NetRequestBudgetMode::Shared {
    Some(self.resources.budget.enter().map_err(|kind| NetAcquireError::Budget {
        url: source.url.as_url().clone(),
        kind,
        attempts: attempts.clone(),
        resume: resume.clone(),
    })?)
} else {
    None
};
let budget_evidence = budget.as_ref().map(|permit| NetBudgetEvidence {
    waited: permit.waited_for(),
});
```

Then every `attempts.push(...)` in that attempt gets:

```rust
budget: budget_evidence
```

The held permit lives until the attempt has finished request/body handling.

### reqwest

Same structure, but admission is awaited:

```rust
let budget = if source.policy.budget.request == NetRequestBudgetMode::Shared {
    Some(budget.enter().await.map_err(|kind| NetAcquireError::Budget { ... })?)
} else {
    None
};
```

`acquire_reqwest` must receive the budget resource alongside client and delay:

```rust
async fn acquire_reqwest<I>(
    client: reqwest::Client,
    delay: AsyncDelay,
    budget: Arc<dyn NetAsyncBudget>,
    node: Chosen<I, RemoteSource>,
) -> Result<...>
```

## Tests

### Pure policy tests

```text
net_budget_policy_defaults_to_unbounded
net_acquire_policy_accepts_shared_budget
```

Assertions:

```rust
assert_eq!(NetAcquirePolicy::default().budget, NetBudgetPolicy::unbounded());
assert_eq!(NetAcquirePolicy::default().budget(NetBudgetPolicy::shared()).budget.request, NetRequestBudgetMode::Shared);
```

### ureq no-budget compatibility

Existing ureq tests should pass without resource changes.

This proves default budget is no-op.

### ureq shared budget records wait

Use a test budget implementation, not wall-clock sleeping:

```rust
struct TestSyncBudget {
    waited: Duration,
}

impl NetSyncBudget for TestSyncBudget {
    fn enter(&self) -> Result<NetBudgetPermit, NetBudgetError> {
        Ok(NetBudgetPermit::waited(self.waited))
    }
}
```

Test:

```text
ureq_shared_budget_records_admission_wait
```

Expected:

```rust
assert_eq!(acquired.evidence.attempts[0].budget, Some(NetBudgetEvidence { waited }));
```

### ureq rejected budget fails before request

Use a test server that would record requests. Inject a rejecting budget.

Test:

```text
ureq_budget_rejection_fails_before_request
```

Expected:

```rust
matches!(error, NetAcquireError::Budget { kind: NetBudgetError::Rejected, .. })
server received no request
```

This proves admission happens before HTTP side effects.

### reqwest shared budget records wait

Async equivalent:

```text
reqwest_shared_budget_records_admission_wait
```

Use a no-wait async budget returning `NetBudgetPermit::waited(waited)`.

### reqwest rejected budget fails before request

```text
reqwest_budget_rejection_fails_before_request
```

### Retry admission test

One parity test is enough, preferably ureq:

```text
ureq_retry_enters_budget_per_attempt
```

Server sequence:

```text
503 then 200
```

Policy:

```text
retry max 1
budget shared
```

Injected budget counts `enter()` calls.

Expected:

```text
enter called twice
attempts.len() == 2
both attempts have budget evidence
```

## Implementation sequence

### Step 1: RED tests

Add tests for:

```text
net_budget_policy_defaults_to_unbounded
net_acquire_policy_accepts_shared_budget
ureq_shared_budget_records_admission_wait
ureq_budget_rejection_fails_before_request
reqwest_shared_budget_records_admission_wait
reqwest_budget_rejection_fails_before_request
ureq_retry_enters_budget_per_attempt
```

Run focused tests and confirm compile/test failures due to missing types/fields.

### Step 2: Add public budget types

Add:

```rust
NetBudgetPolicy
NetRequestBudgetMode
NetBudgetEvidence
NetBudgetError
NetBudgetPermit
NetSyncBudget
NetAsyncBudget
NoNetSyncBudget
NoNetAsyncBudget
```

Keep the public shape compact. Do not introduce separate request-plan or admission-plan structs unless tests demand them.

### Step 3: Extend policy and evidence

Add:

```rust
NetAcquirePolicy::budget
NetAcquirePolicy::budget(...)
NetAttemptEvidence::budget
```

Update every `NetAttemptEvidence` construction site.

### Step 4: Extend resources

Add budget field/methods to:

```text
UreqResource
ReqwestResource
```

Defaults must be no-op.

### Step 5: Add error path

Add:

```rust
NetAcquireError::Budget { ... }
NetAttemptOutcome::BudgetRejected
```

Display should be brief and domain-specific:

```text
net acquire budget rejected for {url}: {kind:?}
```

### Step 6: Insert admission before request send

For both ureq and reqwest:

```text
compute resume context
admit budget if policy requests shared budget
build/send request
record budget evidence on attempt result
```

### Step 7: Verify

Use fresh ad-hoc script under:

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
git diff --check -- crates/pulith/src/lib.rs crates/pulith/src/net.rs
```

If `error.rs` changes for budget Display/source, include it too.

## Acceptance criteria

The next slice is complete when:

```text
NetAcquirePolicy has explicit budget policy and defaults to unbounded.
Shared budget is opt-in per source policy.
UreqResource and ReqwestResource own reusable shared budget handles.
Default resources behave exactly as before.
Budget admission occurs before HTTP request send.
Budget admission occurs once per outbound attempt, including retries.
Attempt evidence records budget wait duration when shared budget is active.
Budget rejection fails as NetAcquireError::Budget before remote side effects.
Existing retry/resume/validator behavior remains unchanged.
200/416 resume outcomes remain evidence, not error.
Reqwest remains Tokio-backed and uses response.chunk().await.
No governor/leaky-bucket/tower dependency is added in the first slice.
Fresh ad-hoc verification passes and the temp script is cleaned.
```

## Explicit non-goals

Do not implement in this slice:

```text
byte-per-second bandwidth throttling
chunk pacing
host/domain keyed budget pools
per-host HTTP connection policy
Tower layers
governor-backed implementation
leaky-bucket-backed implementation
object_store budget integration
progress callbacks
sidecar partial metadata
new async runtime backend
```

## Future follow-ups

After request admission is stable:

1. Add optional byte bandwidth pacing around response body copy/chunk boundaries.
2. Consider `governor` for request-start GCRA rate limiting behind an optional feature.
3. Add host-keyed pools only if callers need multi-host fairness.
4. Extend object-store backend with the same `NetBudgetPolicy` semantics.
5. Consider progress callbacks only after bandwidth/pacing evidence is typed.

## Recommended next action

Implement the request admission budget slice exactly as above. Start with RED tests for policy defaults, ureq wait evidence, ureq rejection before request, reqwest wait evidence, reqwest rejection before request, and retry admission count.
