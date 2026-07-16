# Pulith Resume Validator Next-Step Plan

## Status

This is a docs-only implementation plan. It converts the previous next-step planning into an executable next slice with API pseudocode, code structure, and verification scope.

Current implemented baseline:

```text
retry baseline
explicit NetResumePolicy
Range: bytes=N-
206 + valid Content-Range -> append partial
200 to ranged request -> fresh restart
416 to ranged request -> restart once
ureq + reqwest parity
reqwest keeps Response::chunk().await
```

Current intentional gaps:

```text
If-Range
ETag / Last-Modified validator policy
persisted partial metadata
validator-aware resume records
net-owned NetAcquireError hierarchy
budget/rate accounting
```

The next slice should fill validator semantics before any final error hierarchy.

## Goal

Implement validator-aware resume/range behavior while keeping the body-copy implementation unchanged.

The target behavior is:

```text
partial bytes alone are not enough for safe resume
partial bytes + validator can produce If-Range
If-Range mismatch is usually observed as 200 OK full body
206 still requires Content-Range start == partial_len
416 remains stale-partial recovery
```

This slice should answer:

```text
when do we send If-Range?
which validators are acceptable?
how do we capture validators from responses?
how do we record validator-driven append/restart decisions?
```

It should not yet answer:

```text
what is the final NetAcquireError hierarchy?
how are budgets counted across retries/resume/restarts?
how are partial metadata files persisted across process runs?
```

## Design law

Validator state belongs to resume behavior, not to generic errors.

The ordering remains:

```text
resume/range validator semantics -> resume records -> net-owned errors -> budget/rate
```

Do not introduce:

```rust
NetAcquireError { error: PulithError, ... }
```

If any error type is touched in this slice, it should only be a temporary local helper or a narrow transitional error mapping. The public final error hierarchy waits until validator branches are stable.

## API pseudocode

### NetAcquirePolicy

Current:

```rust
pub struct NetAcquirePolicy {
    pub timeout: Option<Duration>,
    pub max_bytes: Option<u64>,
    pub headers: Vec<(String, String)>,
    pub retry: NetRetryPolicy,
    pub resume: NetResumePolicy,
}
```

Keep this shape. Extend `NetResumePolicy`, not `NetAcquirePolicy` directly.

### NetResumePolicy

Current:

```rust
pub struct NetResumePolicy {
    pub mode: NetResumeMode,
    pub partial_path: Option<PathBuf>,
}
```

Next shape:

```rust
pub struct NetResumePolicy {
    pub mode: NetResumeMode,
    pub partial_path: Option<PathBuf>,
    pub validator: NetResumeValidatorPolicy,
}
```

Builder pseudocode:

```rust
impl NetResumePolicy {
    pub fn restart_only() -> Self;

    pub fn resume_from(partial_path: impl Into<PathBuf>) -> Self;

    pub fn validator(mut self, validator: NetResumeValidatorPolicy) -> Self {
        self.validator = validator;
        self
    }
}
```

Default:

```rust
NetResumePolicy {
    mode: NetResumeMode::RestartOnly,
    partial_path: None,
    validator: NetResumeValidatorPolicy::StrongEtagOnly,
}
```

Rationale:

```text
RestartOnly remains safe.
ResumeIfValidated remains explicit.
Validator policy defaults conservative when resume is explicitly enabled.
```

### Validator policy

Add:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetResumeValidatorPolicy {
    StrongEtagOnly,
    EtagOrLastModified,
    AllowRangeWithoutValidator,
}
```

Semantics:

```text
StrongEtagOnly:
  Send If-Range only when a strong ETag exists.
  Weak ETags are not accepted for resume.

EtagOrLastModified:
  Prefer strong ETag.
  Otherwise send Last-Modified HTTP date.

AllowRangeWithoutValidator:
  Permit Range without If-Range.
  This is explicit and less safe; useful for servers without validators or caller-owned trust.
```

Do not make `AllowRangeWithoutValidator` default.

### Validator value

Add:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NetResumeValidator {
    StrongEtag(String),
    LastModified(SystemTime),
}
```

Optional helper:

```rust
impl NetResumeValidator {
    fn if_range_value(&self) -> String {
        match self {
            Self::StrongEtag(etag) => etag.clone(),
            Self::LastModified(time) => httpdate::fmt_http_date(*time),
        }
    }
}
```

ETag parser rules:

```text
accept: "abc", "abc-123"
reject for StrongEtagOnly: W/"abc"
keep quoted ETag string exactly for If-Range
```

### Partial metadata

Do not persist metadata in this next slice. Introduce operation-local metadata first.

Current public opt-in:

```rust
NetResumePolicy::resume_from(partial_path)
```

Add explicit validator opt-in without metadata file:

```rust
pub fn resume_from_with_validator(
    partial_path: impl Into<PathBuf>,
    validator: NetResumeValidator,
) -> Self
```

Internal shape:

```rust
struct ActiveResume {
    partial_path: PathBuf,
    partial_bytes: u64,
    validator: Option<NetResumeValidator>,
}
```

Replace current helper:

```rust
fn active_resume(policy: &NetResumePolicy) -> Option<(PathBuf, u64)>
```

with:

```rust
fn active_resume(policy: &NetResumePolicy) -> Option<ActiveResume>
```

This keeps public API explicit and avoids prematurely designing a sidecar metadata format.

### Captured response validators

Add to success evidence:

```rust
pub struct NetAcquireEvidence {
    pub url: url::Url,
    pub final_path: PathBuf,
    pub status: u16,
    pub bytes: u64,
    pub content_length: Option<u64>,
    pub attempts: Vec<NetAttemptEvidence>,
    pub resume: Option<NetResumeEvidence>,
    pub validator: Option<NetResponseValidator>,
}
```

Add:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetResponseValidator {
    pub etag: Option<String>,
    pub last_modified: Option<SystemTime>,
}
```

Capture from any final successful response:

```text
ETag -> raw header string if syntactically usable
Last-Modified -> parsed SystemTime via httpdate
```

This evidence is useful for callers to persist metadata themselves later, without Pulith owning metadata files yet.

### Resume evidence

Current:

```rust
pub struct NetResumeEvidence {
    pub outcome: NetResumeOutcome,
    pub partial_path: PathBuf,
    pub partial_bytes: u64,
}
```

Next:

```rust
pub struct NetResumeEvidence {
    pub outcome: NetResumeOutcome,
    pub partial_path: PathBuf,
    pub partial_bytes: u64,
    pub validator: Option<NetResumeValidator>,
}
```

Extend outcomes minimally:

```rust
pub enum NetResumeOutcome {
    PartialAppended,
    RangeIgnoredRestarted,
    RangeUnsatisfiableRestarted,
    MissingValidatorRestarted,
}
```

Meaning:

```text
MissingValidatorRestarted:
  ResumeIfValidated was requested, but policy required a validator and none was available.
  Pulith chose safe restart rather than unsafe Range.
```

Do not add too many outcomes yet. Avoid distinguishing every header reason until error taxonomy is designed.

## Request construction pseudocode

Shared semantic helper:

```rust
struct ResumeRequestPlan {
    active: Option<ActiveResume>,
    range_header: Option<String>,
    if_range_header: Option<String>,
    restart_reason: Option<NetResumeOutcome>,
}

fn plan_resume_request(policy: &NetResumePolicy) -> ResumeRequestPlan {
    let Some(active) = active_resume(policy) else {
        return ResumeRequestPlan::restart();
    };

    match (&policy.validator, &active.validator) {
        (StrongEtagOnly, Some(StrongEtag(etag))) => ResumeRequestPlan::range_if_range(active, etag),
        (StrongEtagOnly, _) => ResumeRequestPlan::restart_reason(active, MissingValidatorRestarted),

        (EtagOrLastModified, Some(StrongEtag(etag))) => ResumeRequestPlan::range_if_range(active, etag),
        (EtagOrLastModified, Some(LastModified(date))) => ResumeRequestPlan::range_if_range(active, fmt_http_date(date)),
        (EtagOrLastModified, _) => ResumeRequestPlan::restart_reason(active, MissingValidatorRestarted),

        (AllowRangeWithoutValidator, _) => ResumeRequestPlan::range(active),
    }
}
```

Backend request application:

```rust
if let Some(range) = plan.range_header {
    request = request.header("Range", range);
}
if let Some(if_range) = plan.if_range_header {
    request = request.header("If-Range", if_range);
}
```

For reqwest use constants:

```rust
reqwest::header::RANGE
reqwest::header::IF_RANGE
```

For ureq string names are acceptable if kept localized:

```rust
"Range"
"If-Range"
```

## Response classification pseudocode

Replace scattered status logic with a small private classifier.

Do not create a public state machine yet.

```rust
enum ResumeResponseAction {
    Fresh,
    Append(ActiveResume),
    Restart(NetResumeEvidence),
    Fail(PulithError), // transitional only
}

fn classify_resume_response(
    status: u16,
    headers: &NetHeaders,
    resume_plan: &ResumeRequestPlan,
) -> ResumeResponseAction {
    match (resume_plan.active.as_ref(), status) {
        (Some(active), 206) => {
            if valid_content_range(headers.content_range(), active.partial_bytes) {
                Append(active.clone())
            } else {
                Fail(PulithError::NetworkError("invalid Content-Range for resume".into()))
            }
        }
        (Some(active), 200) => {
            Restart(NetResumeEvidence::range_ignored(active))
        }
        (Some(active), 416) => {
            Restart(NetResumeEvidence::range_unsatisfiable(active))
        }
        (None, 206) => {
            Fail(PulithError::NetworkError("partial response without resume request".into()))
        }
        _ => Fresh,
    }
}
```

`NetHeaders` should not be a public abstraction in this slice. Use a tiny private input or two backend-specific helpers if fewer functions is cleaner.

## Code structure plan

Keep this inside `crates/pulith/src/net.rs` for now. Do not split a `net/resume.rs` module until the file becomes unmanageable or reuse pressure is real.

Recommended internal section order:

```text
public API types
  RemoteUrl
  NetAcquirePolicy
  NetRetryPolicy
  NetResumePolicy
  NetResumeValidatorPolicy
  NetResumeValidator
  RemoteSource
  NetAcquireEvidence
  NetResumeEvidence
  NetResponseValidator
  NetAttemptEvidence

resource types
  UreqResource
  ReqwestResource

backend implementations
  UreqAcquire
  ReqwestAcquire

private shared helpers
  destination_parent
  reject_existing_unsafe_destination
  reject_known_oversize
  retry_delay / planned_retry_delay / parse_retry_after
  active_resume
  plan_resume_request
  parse_content_range
  parse_response_validator
  parse_strong_etag
```

Keep helper count small. The next slice should add at most these durable helpers:

```rust
active_resume(policy) -> Option<ActiveResume>
plan_resume_request(policy) -> ResumeRequestPlan
parse_response_validator(headers) -> NetResponseValidator
parse_strong_etag(value) -> Option<String>
```

If backend header APIs make a generic `parse_response_validator` ugly, use two tiny backend-local extraction blocks instead and keep only:

```rust
parse_etag_value(value: &str) -> Option<String>
parse_last_modified(value: &str) -> Option<SystemTime>
```

## Backend edit plan

### ureq path

Current flow:

```text
active_resume -> Range header -> response -> 416 branch -> status branch -> 206 branch -> temp staging
```

Next flow:

```text
plan_resume_request
apply Range / If-Range headers
call request
capture status/content_length/retry_after
if 416 + active -> restart once
status branch
classify 206/200/fresh
capture response validator from final successful response
stage fresh or append
persist
record NetResumeEvidence with validator
record NetAcquireEvidence.validator
```

Important: if plan says `MissingValidatorRestarted`, do not send Range. Just do fresh GET and record resume evidence if success.

### reqwest path

Same semantic flow as ureq.

Keep:

```rust
while let Some(chunk) = response.chunk().await? { ... }
```

Do not enable:

```rust
bytes_stream()
```

Add only:

```text
If-Range header when plan supplies validator
validator capture from response.headers()
```

## Test plan

Use local loopback server tests only. No external network.

### Pure tests

```text
resume_policy_requires_validator_by_default_when_resume_enabled
resume_policy_allows_explicit_range_without_validator
strong_etag_parser_rejects_weak_etag
last_modified_parser_accepts_http_date
```

Expected assertions:

```rust
assert_eq!(NetResumePolicy::resume_from(path).validator,
           NetResumeValidatorPolicy::StrongEtagOnly);

assert_eq!(parse_strong_etag("\"abc\""), Some("\"abc\"".to_string()));
assert_eq!(parse_strong_etag("W/\"abc\""), None);
```

### ureq behavior tests

```text
ureq_resume_with_strong_etag_sends_if_range_and_appends_206
ureq_resume_without_required_validator_restarts_without_range
ureq_resume_with_last_modified_policy_sends_if_range_date
ureq_resume_if_range_mismatch_200_records_restart
```

Key server assertions:

```text
Range == bytes=N-
If-Range == "etag"
```

For missing-validator restart:

```text
Range header absent
If-Range header absent
final body is full response
resume outcome == MissingValidatorRestarted
```

### reqwest behavior tests

Mirror only the most important parity cases to avoid test bloat:

```text
reqwest_resume_with_strong_etag_sends_if_range_and_appends_206
reqwest_resume_without_required_validator_restarts_without_range
reqwest_resume_if_range_mismatch_200_records_restart
```

Existing tests already cover:

```text
reqwest 206 append
reqwest 416 restart
```

Do not duplicate every ureq case unless regressions appear.

## Implementation sequence

### Step 1: RED tests for policy/parser

Add pure tests first:

```text
resume_policy_requires_validator_by_default_when_resume_enabled
strong_etag_parser_rejects_weak_etag
```

Expected compile failures:

```text
missing NetResumeValidatorPolicy
missing NetResumeValidator
missing parser/helper
```

### Step 2: Add API types

Add public enums/structs, re-export in `lib.rs`:

```rust
NetResumeValidatorPolicy
NetResumeValidator
NetResponseValidator
```

Keep existing constructors working.

### Step 3: Replace active_resume tuple

Replace:

```rust
Option<(PathBuf, u64)>
```

with:

```rust
Option<ActiveResume>
```

Then update existing ureq/reqwest branches without changing behavior.

Run focused tests before adding If-Range behavior.

### Step 4: Add request planner

Add `plan_resume_request` and use it in both backends.

Initially support:

```text
StrongEtagOnly + missing validator -> MissingValidatorRestarted / fresh GET
StrongEtagOnly + strong ETag -> Range + If-Range
AllowRangeWithoutValidator -> existing Range behavior
```

### Step 5: Add response validator capture

Capture final response validators into `NetAcquireEvidence.validator`.

This gives callers enough evidence to persist metadata outside Pulith.

### Step 6: Add behavior tests

Add local loopback tests for ureq and reqwest as listed above.

### Step 7: Report and fresh verification

Run fresh ad-hoc verification under:

```text
F:\Stratum\TEMP\hermes-verify-*.py
```

Required commands:

```text
cargo fmt --all --check
cargo check -p pulith --no-default-features
cargo check -p pulith --features "sync local net ureq hash blake3"
cargo check -p pulith --features "async net reqwest hash blake3"
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::
cargo test -p pulith --features "async net reqwest hash blake3" net::tests::reqwest
cargo test --workspace --all-features
git diff --check -- crates/pulith/src/lib.rs crates/pulith/src/net.rs docs/report/<execution-report>.md
```

## Acceptance criteria

The slice is done only when:

```text
Resume with strong ETag sends If-Range and appends 206 safely.
Weak ETag is rejected for StrongEtagOnly.
Missing required validator does not send unsafe Range.
200 response to If-Range mismatch restarts safely and records restart.
Final success evidence includes response validators.
ureq and reqwest remain behavior-compatible.
reqwest still uses chunk().await, no bytes_stream feature added.
No net-owned error hierarchy is introduced yet.
Fresh ad-hoc verification passes and script is cleaned.
```

## Non-goals for this next slice

Do not implement:

```text
sidecar partial metadata files
multi-process partial discovery
checksums in Acquire
progress callback
bytes_stream()
object_store resume
bandwidth budgets
rate governor
final NetAcquireError hierarchy
```

## Why this is the right next slice

The current resume implementation proves Range mechanics and staging safety. The remaining correctness risk is validator semantics:

```text
Partial bytes from an old representation can be appended to a new representation unless If-Range/validator policy is explicit.
```

Therefore validator semantics must come before error hierarchy and before budget/rate. Once validator outcomes are stable, the later `NetAcquireError` can describe real net-acquire failures without being distorted by recoverable 200/416 resume branches.
