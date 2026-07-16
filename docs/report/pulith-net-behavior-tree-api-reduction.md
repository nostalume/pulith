# Pulith Net Behavior Tree and API Reduction Analysis

## Status

Analysis only. No production code was changed.

This report explains the behavior relationships among:

- attempt
- budget / admission
- retry
- resume
- validator
- error
- future net extensions

It also reduces the previously planned budget/rate API by collapsing duplicate semantics and avoiding unnecessary structs, fields, and functions.

## Core conclusion

The central behavior tree is:

```text
NetAcquire
  ├─ LocalPreflight
  ├─ AttemptLoop
  │   └─ Attempt[n]
  │       ├─ ResumePlan
  │       ├─ Admission
  │       ├─ Request
  │       ├─ ResponseClassify
  │       ├─ BodyCopy
  │       └─ Persist
  └─ Evidence/Error
```

`Attempt`, `Admission`, and `Resume` are not three peer public workflows. They are nested behaviors inside one `NetAcquire` morphism:

```text
Chosen<I, RemoteSource> -> Acquired<I, LocalMaterial, NetAcquireEvidence>
```

The most important reduction rule:

```text
Attempt is the record boundary.
Resume is request/response recovery behavior.
Admission is resource-side pre-request behavior.
Retry is the loop that creates more attempts.
```

Do not promote each internal behavior into a public plan/action/evidence struct unless it has an independent caller-visible state transition.

## Current public model

Current `net.rs` public model already has these stable concepts:

```rust
RemoteUrl
RemoteSource
NetAcquirePolicy
NetRetryPolicy
NetResumePolicy
NetResumeMode
NetValidator
NetAcquireEvidence
NetAttemptEvidence
NetAttemptOutcome
NetResumeEvidence
NetResumeOutcome
NetAcquireError
NetTransportPhase
NetProtocolError
NetUnsafeDestination
```

This is close to the right boundary, but future budget/rate work should not add another parallel hierarchy unless necessary.

## Behavior relationship analysis

### `NetAcquire`

`NetAcquire` is the outer behavior. It owns:

- source URL
- destination path
- policy
- backend resource
- final material/evidence or net-owned error

Behavior law:

```text
A successful NetAcquire produces one durable local file material and one NetAcquireEvidence.
A failed NetAcquire must not partially overwrite the final destination.
```

### `Attempt`

`Attempt` is not a caller-selected behavior. It is the per-outbound-request record boundary created by retry/resume orchestration.

An attempt currently records:

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

Meaning:

- `attempt` is ordinal within one `NetAcquire`.
- `status` is HTTP response status if a response existed.
- `bytes` is body bytes accepted for that attempt.
- `retry_after` is server-provided retry pacing input.
- `planned_delay` is retry output chosen before next attempt.
- `outcome` is the attempt terminal classification.

Attempt is therefore the correct place to attach future per-attempt facts such as admission wait.

Attempt is **not** the right place for:

- selected response validator for future resumes — that belongs to final acquire evidence.
- aggregate resume outcome — that belongs to resume evidence.
- global resource capacity — that belongs to resource handles.

### `Retry`

Retry is the loop behavior that may create more attempts.

Retry consumes:

```text
NetRetryPolicy
Retry-After header
attempt outcome
```

Retry produces:

```text
more attempts
planned_delay in the failed/retryable attempt record
```

Retry does **not** own HTTP classification, resume planning, or admission. It only decides whether to repeat after a retryable failure.

Relationship:

```text
Retry wraps Attempt.
Attempt records Retry output.
```

### `Resume`

Resume is request planning plus response recovery.

Resume consumes:

```text
NetResumePolicy
partial file metadata
optional NetValidator
HTTP status / Content-Range
```

Resume produces:

```text
Range / If-Range request headers
NetResumeEvidence when resume affects the result
```

Current reduced model is correct:

```rust
pub struct NetResumePolicy {
    pub mode: NetResumeMode,
}
```

```rust
pub enum NetResumeMode {
    RestartOnly,
    Unvalidated { partial_path: PathBuf },
    IfRange { partial_path: PathBuf, validator: NetValidator },
}
```

Resume behavior table:

| mode/status | behavior | evidence/error |
|---|---|---|
| `RestartOnly` | no range request | no resume evidence |
| `Unvalidated` | send `Range` only | resume evidence if server reacts/restart/appends |
| `IfRange` | send `Range` + `If-Range` | resume evidence includes validator |
| `206` valid `Content-Range` | append partial into staged temp | `PartialAppended` |
| `200` after range | discard partial and restart full | `RangeIgnoredRestarted` |
| `416` after range | suppress resume and retry full once | `RangeUnsatisfiableRestarted` |
| `206` invalid/missing `Content-Range` | protocol failure | `NetAcquireError::Protocol` |

Resume is related to attempt because each outbound attempt may or may not carry resume headers. But the current public evidence keeps only the result-affecting resume outcome:

```rust
pub resume: Option<NetResumeEvidence>
```

This remains preferable to `Vec<NetResumeEvidence>` until a behavior proves multiple visible resume events matter.

### `Validator`

Validator is not a separate resume policy or response policy. It is one reusable fact:

```rust
pub enum NetValidator {
    Etag(String),
    LastModified(SystemTime),
}
```

Validator appears in two places for distinct reasons:

1. `NetResumeMode::IfRange { validator }`
   - request dependency
   - caller or future sidecar says: use this validator to guard append
2. `NetAcquireEvidence.validator`
   - response evidence
   - final response selected a future validator

This is not duplicate structure because the same type represents the same domain fact at two different phases. Adding `NetResumeValidator`, `NetResponseValidator`, or `NetValidatorPolicy` would reintroduce duplication.

### `Admission` / request budget

Budget should be named by the behavior it performs in the first slice:

```text
Admission
```

The first behavior is not generic budget/rate/bandwidth. It is:

```text
pre-request admission into a shared resource
```

Admission consumes:

```text
source policy says whether shared admission is required
resource-owned admission handle
```

Admission produces:

```text
attempt admission wait evidence
or admission error before HTTP side effects
```

Relationship:

```text
Attempt contains Admission.
Retry creates multiple Attempts; therefore retry creates multiple Admissions.
Admission happens after ResumePlan is computed, before request send.
```

Admission does not belong to `NetResumeEvidence`, because resume describes HTTP range behavior, not resource fairness.

Admission does not belong to `NetAcquireEvidence` as an aggregate field unless a future aggregate summary is needed; per-attempt wait is enough.

### `Error`

`NetAcquireError` is failure evidence, not success evidence.

It currently carries `attempts` and `resume` in final failure variants. That is correct because failed net acquire has no `NetAcquireEvidence`, but caller still needs context.

Relationship:

```text
success -> NetAcquireEvidence { attempts, resume, validator }
failure -> NetAcquireError::{... attempts, resume ...}
```

Do not add a separate `NetAcquireFailure` struct unless many variants start sharing enough fields that enum readability collapses. Even then, prefer private constructor helpers before public failure wrappers.

## Full behavior tree

### Current executable tree

```text
NetAcquire
  Input:
    Chosen<I, RemoteSource>
  Policy:
    timeout
    max_bytes
    headers
    retry
    resume
  Resource:
    ureq agent OR reqwest client
    delay provider
  Output:
    Acquired<I, LocalMaterial, NetAcquireEvidence>
  Error:
    NetAcquireError

  LocalPreflight
    destination_parent
    create parent
    reject unsafe final destination

  AttemptLoop
    max_attempts = retry.max_retries + resume_restart_allowance

    Attempt[n]
      ResumePlan
        RestartOnly -> no headers
        Unvalidated -> Range
        IfRange -> Range + If-Range

      RequestBuild
        headers
        timeout
        resume headers

      SendRequest
        transport failure -> Retry or NetAcquireError::Transport { SendRequest }

      ResponseClassify
        416 + active resume -> record RangeUnsatisfiableRestarted; retry full once
        non-success retryable -> RetryableStatus; maybe retry
        non-success final -> NetAcquireError::HttpStatus
        success -> continue

      KnownOversizeCheck
        content_length > max_bytes -> NetAcquireError::LimitExceeded

      ResumeResponse
        206 without active resume -> NetAcquireError::Protocol
        206 invalid Content-Range -> NetAcquireError::Protocol
        206 valid -> append partial
        200 after resume -> RangeIgnoredRestarted

      BodyCopy
        read failure -> Retry or NetAcquireError::Transport { ReadBody }
        max_bytes exceeded -> NetAcquireError::LimitExceeded
        write failure -> NetAcquireError::Local

      Persist
        flush/persist local failure -> NetAcquireError::Local
        success -> NetAcquireEvidence
```

### Next reduced tree with admission

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

Admission must be inserted before request send:

```text
ResumePlan -> Admission -> RequestBuild/SendRequest
```

Why after `ResumePlan`?

Because resume planning may discover there is no partial to use, and it determines whether this attempt will be range/full. Admission evidence then describes the actual outbound request attempt.

Why before `RequestBuild`?

Because rejected admission must prove no HTTP side effect occurred.

## Future behavior tree

### Request-start rate

Future request-start rate is the same Admission node, not a new top-level behavior.

It can be represented by the same admission trait/resource:

```text
Admission = concurrency gate + optional rate pace
```

Evidence still only needs:

```text
admission_wait: Option<Duration>
```

No new `NetRateEvidence` is necessary unless a future caller needs to distinguish rate wait from concurrency wait.

### Byte bandwidth pacing

Bandwidth is not the same as request admission.

Bandwidth sits inside `BodyCopy`:

```text
BodyCopy
  ReadChunk
  BandwidthPace
  WriteChunk
```

It should not be mixed into first admission slice. It may need separate evidence later:

```text
body_wait: Duration
```

But do not add it now.

### Progress callbacks

Progress observes `BodyCopy`; it does not change acquire semantics unless callback failure can abort.

Future tree:

```text
BodyCopy
  ReadChunk
  LimitCheck
  BandwidthPace
  WriteChunk
  ProgressObserve
```

Progress should probably be resource-owned callback, not policy-owned behavior, if it only observes.

Do not add `NetProgressEvidence` unless persisted progress is useful after completion.

### Sidecar partial metadata

Sidecar metadata belongs to resume/remember boundary, not attempt.

Future tree:

```text
RememberPartial
  selected validator
  partial path
  bytes
  maybe content length / etag / last-modified
```

Resume can later consume sidecar facts to build `NetResumeMode::IfRange`.

Avoid adding `NetResumeSidecar`, `NetPartialRecord`, and `NetValidatorRecord` simultaneously. The durable record can be one type only if needed.

### Multi-source / mirror

Mirror is above NetAcquire, not inside attempt.

Future tree:

```text
SelectRemote
  candidate sources
  source policy/evidence history
  choose RemoteSource
NetAcquire(chosen source)
```

Attempt evidence stays inside one selected source acquire. Do not put mirror ranking into `NetAttemptEvidence`.

### object_store backend

Object store is another backend family for `Acquire`, not another public behavior.

It should reuse:

```text
NetAcquirePolicy
NetAcquireEvidence
NetAcquireError
NetAttemptEvidence
NetResumePolicy where supported
Admission where supported
```

Unsupported features should be explicit errors or no-op policies, not parallel object-store-specific policy structs unless behavior diverges.

### Runtime-neutral async backend

Future Isahc/smol/compio backend is an implementation family:

```text
AsyncAcquireNode<Chosen<I, RemoteSource>>
```

It should not change the behavior tree. It may require resource-specific admission implementation.

## Semantic duplication audit

### Duplicate: budget vs admission vs rate

The previous budget plan used:

```text
NetBudgetPolicy
NetRequestBudgetMode
NetBudgetEvidence
NetBudgetError
NetBudgetPermit
NetSyncBudget
NetAsyncBudget
```

This is too generic for the first behavior. The behavior being added is request admission.

Reduced names:

```text
NetAdmissionMode
NetAdmissionError
NetAdmissionPermit
NetSyncAdmission
NetAsyncAdmission
```

Avoid a separate `NetBudgetPolicy` wrapper. Put the mode directly on `NetAcquirePolicy`:

```rust
pub struct NetAcquirePolicy {
    pub timeout: Option<Duration>,
    pub max_bytes: Option<u64>,
    pub headers: Vec<(String, String)>,
    pub retry: NetRetryPolicy,
    pub resume: NetResumePolicy,
    pub admission: NetAdmissionMode,
}
```

```rust
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NetAdmissionMode {
    #[default]
    Unbounded,
    Shared,
}
```

Constructors:

```rust
impl NetAcquirePolicy {
    pub fn admission(mut self, admission: NetAdmissionMode) -> Self;
    pub fn shared_admission(self) -> Self;
}
```

This removes one public struct and one level of `.budget.request` field access.

### Duplicate: budget evidence struct

Previous plan proposed:

```rust
pub struct NetBudgetEvidence {
    pub waited: Duration,
}

pub struct NetAttemptEvidence {
    pub budget: Option<NetBudgetEvidence>,
}
```

For the first slice this is semantically equivalent to one field:

```rust
pub struct NetAttemptEvidence {
    pub admission_wait: Option<Duration>,
}
```

Reduced form:

```rust
pub struct NetAttemptEvidence {
    pub attempt: u32,
    pub status: Option<u16>,
    pub bytes: u64,
    pub content_length: Option<u64>,
    pub retry_after: Option<Duration>,
    pub planned_delay: Option<Duration>,
    pub admission_wait: Option<Duration>,
    pub outcome: NetAttemptOutcome,
}
```

This avoids `NetBudgetEvidence` until evidence needs more than one scalar.

Future bandwidth wait should not reuse `admission_wait`; it can add `body_wait` later if needed.

### Duplicate: no-op admission structs

Previous plan had:

```text
NoNetSyncBudget
NoNetAsyncBudget
```

These can be avoided by resource fields using `Option<Arc<dyn ...>>`:

```rust
pub struct UreqResource {
    agent: ureq::Agent,
    delay: SyncDelay,
    admission: Option<Arc<dyn NetSyncAdmission>>,
}
```

```rust
pub struct ReqwestResource {
    client: reqwest::Client,
    delay: AsyncDelay,
    admission: Option<Arc<dyn NetAsyncAdmission>>,
}
```

Rules:

- `NetAdmissionMode::Unbounded` ignores the resource admission handle.
- `NetAdmissionMode::Shared` requires the resource admission handle.
- missing handle under `Shared` fails with `NetAdmissionError::Unavailable` before request send.

This removes two public no-op structs.

### Duplicate: request budget mode wrapper

Previous:

```rust
NetBudgetPolicy { request: NetRequestBudgetMode }
```

Reduced:

```rust
NetAdmissionMode
```

The word `request` is unnecessary because the first behavior is already named admission, and admission is defined as pre-request. Future byte bandwidth should be a separate field, not nested under this one.

### Duplicate: attempt outcome categories

Current attempt outcomes:

```rust
Success
RetryableStatus
RetryableNetworkError
NonRetryableStatus
NonRetryableNetworkError
LocalFailure
LimitExceeded
```

Potential addition:

```rust
AdmissionRejected
```

Do not add separate:

```text
BudgetRejected
RateLimited
ConcurrencyRejected
AdmissionClosed
AdmissionUnavailable
```

Those details belong to `NetAdmissionError`, not `NetAttemptOutcome`.

Reduced rule:

```text
NetAttemptOutcome is coarse phase classification.
Detailed reason lives in NetAcquireError variant/kind.
```

### Duplicate: error context fields

Most `NetAcquireError` variants repeat:

```rust
url
attempts
resume
```

This repetition is currently acceptable because public enum variants remain readable and no extra wrapper is needed. Do not add:

```rust
NetAcquireFailureContext
NetAcquireFailure
NetFailureEvidence
```

yet.

If a future slice adds more failure variants and duplication becomes unmanageable, first add private constructors:

```rust
NetAcquireError::http_status(...)
NetAcquireError::transport(...)
NetAcquireError::admission(...)
```

Only add a public context struct if callers need to manipulate failure context independently.

### Duplicate: resume records vs sidecar records

Current:

```rust
NetResumeEvidence
NetValidator
```

Future sidecar should reuse these facts, not define three new concepts.

Reduced future shape:

```rust
pub struct NetPartialRecord {
    pub partial_path: PathBuf,
    pub bytes: u64,
    pub validator: Option<NetValidator>,
}
```

But do not add this until durable partial remember behavior exists.

## Reduced next-step API recommendation

This supersedes the previous budget/rate plan's heavier names.

### Add one enum on policy

```rust
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NetAdmissionMode {
    #[default]
    Unbounded,
    Shared,
}
```

```rust
pub struct NetAcquirePolicy {
    pub timeout: Option<Duration>,
    pub max_bytes: Option<u64>,
    pub headers: Vec<(String, String)>,
    pub retry: NetRetryPolicy,
    pub resume: NetResumePolicy,
    pub admission: NetAdmissionMode,
}
```

```rust
impl NetAcquirePolicy {
    pub fn admission(mut self, admission: NetAdmissionMode) -> Self;
    pub fn shared_admission(self) -> Self;
}
```

### Add one scalar evidence field

```rust
pub struct NetAttemptEvidence {
    pub attempt: u32,
    pub status: Option<u16>,
    pub bytes: u64,
    pub content_length: Option<u64>,
    pub retry_after: Option<Duration>,
    pub planned_delay: Option<Duration>,
    pub admission_wait: Option<Duration>,
    pub outcome: NetAttemptOutcome,
}
```

### Add one error kind enum

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NetAdmissionError {
    Unavailable,
    Closed,
    Rejected,
}
```

Avoid `InvalidBudget` in the first slice by making constructors assert/reject zero-capacity locally. If a fallible public constructor is added, it can return `Result<Self, NetAdmissionError>` later.

### Add one error variant

```rust
NetAcquireError::Admission {
    url: url::Url,
    kind: NetAdmissionError,
    attempts: Vec<NetAttemptEvidence>,
    resume: Option<NetResumeEvidence>,
}
```

### Add one outcome

```rust
NetAttemptOutcome::AdmissionRejected
```

### Add sync/async admission traits

```rust
#[cfg(feature = "ureq")]
pub trait NetSyncAdmission: Send + Sync {
    fn enter(&self) -> Result<NetAdmissionPermit, NetAdmissionError>;
}
```

```rust
#[cfg(feature = "reqwest")]
pub trait NetAsyncAdmission: Send + Sync {
    fn enter(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<NetAdmissionPermit, NetAdmissionError>> + Send + '_>>;
}
```

### Keep one permit type

```rust
pub struct NetAdmissionPermit {
    waited: Duration,
    // private hold field allowed later for semaphore permits
}

impl NetAdmissionPermit {
    pub fn immediate() -> Self;
    pub fn waited(waited: Duration) -> Self;
    pub fn waited_for(&self) -> Duration;
}
```

The permit type is justified because a real concurrency budget needs to hold a slot until the attempt finishes. But only `waited_for()` is public evidence.

### Resource fields

```rust
pub struct UreqResource {
    agent: ureq::Agent,
    delay: SyncDelay,
    admission: Option<Arc<dyn NetSyncAdmission>>,
}
```

```rust
pub struct ReqwestResource {
    client: reqwest::Client,
    delay: AsyncDelay,
    admission: Option<Arc<dyn NetAsyncAdmission>>,
}
```

Methods:

```rust
pub fn with_admission(mut self, admission: Arc<dyn NetSyncAdmission>) -> Self;
pub fn admission(&self) -> Option<&Arc<dyn NetSyncAdmission>>;
```

and async equivalent.

Do not add separate `with_budget`, `budget`, `budget_policy`, `request_budget`, or no-op resource types.

## Reduced implementation algorithm

### Shared helper concept, private only

Use one private helper per backend or one generic private function if it stays readable:

```text
admit_attempt(...)
```

But do not expose:

```text
NetAdmissionPlan
NetBudgetRequest
NetAdmissionEvidence
```

### Attempt loop pseudocode

```rust
let resume_context = planned_resume(...);
let (admission_permit, admission_wait) = match source.policy.admission {
    NetAdmissionMode::Unbounded => (None, None),
    NetAdmissionMode::Shared => {
        let admission = self.resources.admission.as_ref().ok_or_else(|| {
            NetAcquireError::Admission {
                url: source.url.as_url().clone(),
                kind: NetAdmissionError::Unavailable,
                attempts: attempts.clone(),
                resume: resume.clone(),
            }
        })?;
        match admission.enter() {
            Ok(permit) => {
                let waited = permit.waited_for();
                (Some(permit), Some(waited))
            }
            Err(kind) => {
                attempts.push(NetAttemptEvidence {
                    attempt,
                    status: None,
                    bytes: 0,
                    content_length: None,
                    retry_after: None,
                    planned_delay: None,
                    admission_wait: None,
                    outcome: NetAttemptOutcome::AdmissionRejected,
                });
                return Err(NetAcquireError::Admission {
                    url: source.url.as_url().clone(),
                    kind,
                    attempts,
                    resume,
                });
            }
        }
    }
};

// hold admission_permit until attempt completes
```

Every attempt record gets:

```rust
admission_wait
```

## Why this is smaller than the previous plan

Previous plan added or implied:

```text
NetBudgetPolicy
NetRequestBudgetMode
NetBudgetEvidence
NetBudgetError
NetBudgetPermit
NoNetSyncBudget
NoNetAsyncBudget
NetSyncBudget
NetAsyncBudget
```

Reduced plan adds:

```text
NetAdmissionMode
NetAdmissionError
NetAdmissionPermit
NetSyncAdmission
NetAsyncAdmission
```

And one field:

```text
NetAttemptEvidence::admission_wait: Option<Duration>
```

Removed concepts:

```text
NetBudgetPolicy wrapper
NetRequestBudgetMode wrapper enum name
NetBudgetEvidence one-field struct
NoNetSyncBudget
NoNetAsyncBudget
budget.request nested field
with_budget/budget naming ambiguity
```

This reduces public structs while making the behavior name more precise.

## Tests for reduced API

### Pure tests

```text
net_admission_defaults_to_unbounded
net_acquire_policy_accepts_shared_admission
```

### ureq

```text
ureq_shared_admission_records_wait_on_attempt
ureq_missing_shared_admission_fails_before_request
ureq_rejected_admission_fails_before_request
ureq_retry_enters_admission_per_attempt
```

### reqwest

```text
reqwest_shared_admission_records_wait_on_attempt
reqwest_rejected_admission_fails_before_request
```

### Existing behavior guards

Existing tests must still pass:

```text
sync ureq net tests
async reqwest net tests
workspace all-features
```

## Future expansion without semantic duplication

### Add request-start rate later

Reuse `NetSyncAdmission` / `NetAsyncAdmission`.

A rate limiter is just an admission implementation that waits before granting the permit.

No new public policy field is needed unless the caller must choose between multiple resource handles.

### Add concurrency later

Reuse same admission traits.

A semaphore is an admission implementation that holds a private permit until attempt completion.

No new `NetConcurrencyPolicy` is needed.

### Add byte bandwidth later

Do not reuse admission.

Add a separate body-copy behavior only when needed:

```text
NetBodyPace
body_wait or body_delay evidence
```

This avoids conflating request admission with stream pacing.

### Add progress later

Progress is observation, not control.

Add callback resource only when needed; do not add evidence unless durable progress matters.

### Add sidecar later

Sidecar belongs to `RememberPartial` / future resume initialization.

Use `NetValidator` and maybe one `NetPartialRecord`; do not split validator/resume/response record types again.

## Final recommendation

Implement next slice as:

```text
request admission, not generic budget/rate
```

Use the reduced API:

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

Do not add:

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

This keeps the behavior tree complete enough for future extension while reducing the immediate public API surface.
