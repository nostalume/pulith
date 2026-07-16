# Pulith Resume Validator API Reduction Plan

## Status

This report optimizes the previous validator-aware resume plan by applying the requested method:

```text
1. identify behavior
2. identify behavior dependencies
3. delete semantic duplicates
4. reduce public structs/functions
5. keep implementation private until behavior proves it needs public shape
```

This is docs-only. No production code was changed.

## Problem in the previous plan

The previous plan was behavior-correct but API-heavy. It proposed several names that partially described the same concept:

```text
NetResumeValidatorPolicy
NetResumeValidator
NetResponseValidator
ActiveResume
ResumeRequestPlan
ResumeResponseAction
MissingValidatorRestarted
```

The duplication came from treating these as separate nouns too early:

```text
validator policy
validator value
response validator evidence
resume request plan
resume response classifier
missing-validator branch
```

But by behavior dependency analysis, most of these are not independent public concepts.

## Behavior dependency analysis

### Behavior: Acquire remote material

Owner:

```text
RemoteSource + UreqAcquire / ReqwestAcquire
```

Dependencies:

```text
URL
destination
policy
resource handle
```

Output:

```text
Acquired<I, LocalMaterial, NetAcquireEvidence>
```

Acquire owns final success evidence. It should not expose internal response classification machinery.

### Behavior: Retry acquire attempt

Owner:

```text
NetRetryPolicy
```

Dependencies:

```text
retryable status/network error
attempt index
optional Retry-After
injected delay provider
```

Output/record:

```text
NetAttemptEvidence
```

Retry does not need to know whether body bytes came from fresh download or resumed append. It only observes attempt status/outcome/bytes.

### Behavior: Resume acquire body

Owner:

```text
NetResumePolicy
```

Dependencies:

```text
partial file path
partial file byte length
optional HTTP validator used as If-Range condition
```

Output/record:

```text
NetResumeEvidence
```

Resume does not own response body streaming. It only decides request headers and staging branch:

```text
fresh staging
append partial staging
restart after ignored range
restart after unsatisfiable range
```

### Behavior: Validate ranged response

Owner:

```text
private net helper
```

Dependencies:

```text
status code
Content-Range header
partial byte length
```

This should not become a public `ResumeResponseAction` type yet. It is a branch in Acquire implementation.

### Behavior: Capture future resume condition

Owner:

```text
NetAcquireEvidence
```

Dependencies:

```text
successful final response headers
```

This should record one selected future resume condition, not a separate `NetResponseValidator { etag, last_modified }` object unless callers need both values later.

## Reduced semantic model

### Keep one public validator concept

Replace the previous split:

```rust
NetResumeValidator
NetResponseValidator
```

with one public concept:

```rust
pub enum NetValidator {
    Etag(String),
    LastModified(SystemTime),
}
```

Rules:

```text
Etag(String) must be a strong ETag when used for If-Range.
Weak ETags are rejected at construction/parsing boundary.
The raw quoted ETag string is preserved for header emission.
LastModified stores parsed HTTP-date as SystemTime.
```

Why this is enough:

```text
A validator is the same domain fact whether it came from a previous response or is being used for the next If-Range.
Do not create separate request/response validator types until their behavior diverges.
```

### Collapse validator policy into resume mode

Previous plan:

```rust
pub struct NetResumePolicy {
    pub mode: NetResumeMode,
    pub partial_path: Option<PathBuf>,
    pub validator: NetResumeValidatorPolicy,
}

pub enum NetResumeValidatorPolicy {
    StrongEtagOnly,
    EtagOrLastModified,
    AllowRangeWithoutValidator,
}
```

Reduced plan:

```rust
pub struct NetResumePolicy {
    pub mode: NetResumeMode,
}

pub enum NetResumeMode {
    RestartOnly,
    Unvalidated { partial_path: PathBuf },
    IfRange { partial_path: PathBuf, validator: NetValidator },
}
```

This directly encodes behavior:

```text
RestartOnly:
  never send Range.

Unvalidated:
  send Range only. This is explicit and unsafe-ish.

IfRange:
  send Range + If-Range using a validator.
```

This deletes the need for:

```text
NetResumeValidatorPolicy
MissingValidatorRestarted
```

Because there is no "missing validator" runtime policy branch anymore. If the caller wants validated resume, they must construct `IfRange { validator }`. If they do not have a validator, they choose `RestartOnly` or explicitly choose `Unvalidated`.

### Public constructors

Keep API small and behavior-named:

```rust
impl NetResumePolicy {
    pub fn restart_only() -> Self;

    pub fn unvalidated(partial_path: impl Into<PathBuf>) -> Self;

    pub fn if_range(
        partial_path: impl Into<PathBuf>,
        validator: NetValidator,
    ) -> Self;
}
```

Optional compatibility choice:

```rust
pub fn resume_from(partial_path: impl Into<PathBuf>) -> Self
```

But if kept, it should simply delegate to the behavior-named constructor:

```rust
Self::unvalidated(partial_path)
```

Recommendation: prefer adding `unvalidated(...)` and either remove or document `resume_from(...)` as the old name. Since this project accepts breaking API changes, the cleaner target is:

```text
restart_only
unvalidated
if_range
```

No extra `validator(...)` builder is needed because validator is not an orthogonal setting; it changes behavior from Range-only to If-Range.

### NetAcquireEvidence

Previous plan added:

```rust
pub validator: Option<NetResponseValidator>
```

Reduced plan:

```rust
pub struct NetAcquireEvidence {
    pub url: url::Url,
    pub final_path: PathBuf,
    pub status: u16,
    pub bytes: u64,
    pub content_length: Option<u64>,
    pub attempts: Vec<NetAttemptEvidence>,
    pub resume: Option<NetResumeEvidence>,
    pub validator: Option<NetValidator>,
}
```

`validator` means:

```text
selected validator from final successful response that is suitable for a future If-Range resume.
```

Selection rule:

```text
prefer strong ETag
else Last-Modified
else None
```

This avoids exposing a response-header bag while still giving callers the one thing needed for future resume.

If a later behavior needs both ETag and Last-Modified at once, introduce a broader evidence type then. Do not prepay now.

### NetResumeEvidence

Current:

```rust
pub struct NetResumeEvidence {
    pub outcome: NetResumeOutcome,
    pub partial_path: PathBuf,
    pub partial_bytes: u64,
}
```

Reduced next shape:

```rust
pub struct NetResumeEvidence {
    pub outcome: NetResumeOutcome,
    pub partial_path: PathBuf,
    pub partial_bytes: u64,
    pub validator: Option<NetValidator>,
}
```

Keep outcomes as-is for now:

```rust
pub enum NetResumeOutcome {
    PartialAppended,
    RangeIgnoredRestarted,
    RangeUnsatisfiableRestarted,
}
```

Do not add `MissingValidatorRestarted` because the reduced API has no implicit missing-validator branch.

If needed later, use a general restart reason enum only after more reasons accumulate. Current three outcomes are enough.

## Reduced private implementation design

### Delete separate request-plan and response-action concepts

Previous private plan proposed:

```rust
ResumeRequestPlan
ResumeResponseAction
plan_resume_request(...)
classify_resume_response(...)
```

Reduced plan:

```rust
struct PlannedResume {
    partial_path: PathBuf,
    partial_bytes: u64,
    validator: Option<NetValidator>,
}

fn planned_resume(policy: &NetResumePolicy) -> Option<PlannedResume>;
```

This single helper replaces:

```text
active_resume
plan_resume_request
```

Reason:

```text
There is only one question before sending a request: do we have a usable partial, and if yes, is it Range-only or If-Range?
```

Header emission can stay inline in each backend:

```rust
if let Some(resume) = &planned_resume {
    request = request.header("Range", format!("bytes={}-", resume.partial_bytes));
    if let Some(validator) = &resume.validator {
        request = request.header("If-Range", validator.if_range_value());
    }
}
```

Response branch can also stay inline for now because it is short and already backend-local:

```text
416 + planned_resume -> suppress resume and retry fresh once
!success -> existing retry/status path
206 -> validate Content-Range and append
200 + planned_resume -> fresh restart record
else -> fresh
```

Only extract `classify_resume_response` if both backends become too duplicated after the validator slice.

### Helper count target

Target additions:

```rust
fn planned_resume(policy: &NetResumePolicy) -> Option<PlannedResume>
fn parse_strong_etag(value: &str) -> Option<String>
fn selected_response_validator(etag: Option<&str>, last_modified: Option<&str>) -> Option<NetValidator>
```

Already exists:

```rust
parse_content_range(...)
```

Do not add:

```text
plan_resume_request
parse_response_validator struct builder
parse_last_modified wrapper unless needed
ResumeResponseAction
NetHeaders
```

Backend header APIs are different enough that passing raw header values into `selected_response_validator(...)` is simpler than inventing a generic header abstraction.

## Reduced API summary

### Public additions: 1 enum, 1 field addition

Add:

```rust
pub enum NetValidator {
    Etag(String),
    LastModified(SystemTime),
}
```

Change:

```rust
pub struct NetResumePolicy {
    pub mode: NetResumeMode,
}

pub enum NetResumeMode {
    RestartOnly,
    Unvalidated { partial_path: PathBuf },
    IfRange { partial_path: PathBuf, validator: NetValidator },
}
```

Add to existing evidence structs:

```rust
NetAcquireEvidence.validator: Option<NetValidator>
NetResumeEvidence.validator: Option<NetValidator>
```

No new public:

```text
NetResumeValidatorPolicy
NetResumeValidator
NetResponseValidator
MissingValidatorRestarted
```

### Public constructor surface

```rust
NetResumePolicy::restart_only()
NetResumePolicy::unvalidated(partial_path)
NetResumePolicy::if_range(partial_path, validator)
```

Optional parser constructors:

```rust
NetValidator::strong_etag(value: impl Into<String>) -> Option<Self>
NetValidator::last_modified(time: SystemTime) -> Self
```

Prefer associated constructors over free public parser functions. Keep raw parsers private unless callers truly need them.

## Behavior branch table

| Policy mode | Partial file exists? | Request headers | Response 206 | Response 200 | Response 416 |
|---|---:|---|---|---|---|
| `RestartOnly` | ignored | none | protocol failure if unexpected 206 | fresh success | normal status failure |
| `Unvalidated` | no | none | protocol failure if unexpected 206 | fresh success | normal status failure |
| `Unvalidated` | yes | `Range` | append if `Content-Range` valid | fresh restart, record `RangeIgnoredRestarted` | restart once, record `RangeUnsatisfiableRestarted` |
| `IfRange` | no | none | protocol failure if unexpected 206 | fresh success | normal status failure |
| `IfRange` | yes | `Range` + `If-Range` | append if `Content-Range` valid | fresh restart, record `RangeIgnoredRestarted` | restart once, record `RangeUnsatisfiableRestarted` |

Important: `IfRange` mismatch is expressed by server returning `200`; no extra Pulith-side `ValidatorMismatch` type is needed yet.

## Tests after reduction

### Pure tests

Replace previous policy tests with fewer behavior tests:

```text
resume_policy_modes_encode_restart_unvalidated_and_if_range
strong_etag_parser_rejects_weak_etag
selected_response_validator_prefers_strong_etag_over_last_modified
```

### ureq tests

```text
ureq_if_range_resume_sends_range_and_if_range_and_appends_206
ureq_unvalidated_resume_sends_range_without_if_range
ureq_if_range_mismatch_200_restarts_and_records_validator
```

Current existing tests already cover:

```text
ureq 206 append
ureq 200 restart
ureq missing Content-Range reject
```

Adjust them to use `unvalidated(...)` if the old `resume_from(...)` name is removed.

### reqwest tests

```text
reqwest_if_range_resume_sends_range_and_if_range_and_appends_206
reqwest_if_range_mismatch_200_restarts_and_records_validator
```

Existing reqwest tests already cover:

```text
reqwest 206 append
reqwest 416 restart
```

Do not mirror every ureq case.

## Implementation sequence

### Step 1: RED pure tests

Add pure tests for the reduced API:

```text
resume_policy_modes_encode_restart_unvalidated_and_if_range
strong_etag_parser_rejects_weak_etag
selected_response_validator_prefers_strong_etag_over_last_modified
```

Expected failures:

```text
missing NetValidator
missing NetResumeMode::Unvalidated / IfRange payloads
missing constructor names
missing evidence validator field
```

### Step 2: change public API in one place

Modify only the public type section first:

```text
NetResumePolicy
NetResumeMode
NetValidator
NetAcquireEvidence
NetResumeEvidence
lib.rs re-exports
```

Run focused compile.

### Step 3: replace active_resume with planned_resume

Replace current:

```rust
active_resume(policy) -> Option<(PathBuf, u64)>
```

with:

```rust
planned_resume(policy) -> Option<PlannedResume>
```

Update ureq/reqwest header emission inline.

### Step 4: update existing resume tests to reduced naming

Rename old `resume_from(...)` usages to:

```rust
NetResumePolicy::unvalidated(partial_path)
```

This preserves current behavior while making the unsafe-ish choice explicit.

### Step 5: add If-Range behavior tests and implementation

Add only the two backend parity cases needed to prove behavior.

### Step 6: record selected final validator

Capture final response validator into:

```rust
NetAcquireEvidence.validator
```

Record used resume validator into:

```rust
NetResumeEvidence.validator
```

### Step 7: verification

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
git diff --check -- crates/pulith/src/lib.rs crates/pulith/src/net.rs docs/report/<execution-report>.md
```

## Acceptance criteria

The optimized design is accepted when:

```text
Public validator concepts reduce to one NetValidator.
No NetResumeValidatorPolicy exists.
No NetResponseValidator exists.
No MissingValidatorRestarted outcome exists.
Resume behavior is encoded directly in NetResumeMode variants.
Unvalidated Range is explicit in the API name.
IfRange mode sends Range + If-Range.
200 response remains restart evidence, not an error.
206 still validates Content-Range before append.
ureq and reqwest remain behavior-compatible.
No bytes_stream feature is introduced.
No final NetAcquireError hierarchy is introduced yet.
```

## Final recommended next-slice API

```rust
pub struct NetResumePolicy {
    pub mode: NetResumeMode,
}

pub enum NetResumeMode {
    RestartOnly,
    Unvalidated { partial_path: PathBuf },
    IfRange { partial_path: PathBuf, validator: NetValidator },
}

pub enum NetValidator {
    Etag(String),
    LastModified(SystemTime),
}

impl NetResumePolicy {
    pub fn restart_only() -> Self;
    pub fn unvalidated(partial_path: impl Into<PathBuf>) -> Self;
    pub fn if_range(partial_path: impl Into<PathBuf>, validator: NetValidator) -> Self;
}

impl NetValidator {
    pub fn strong_etag(value: impl Into<String>) -> Option<Self>;
    pub fn last_modified(time: SystemTime) -> Self;
}
```

Evidence additions:

```rust
pub struct NetAcquireEvidence {
    // existing fields
    pub validator: Option<NetValidator>,
}

pub struct NetResumeEvidence {
    // existing fields
    pub validator: Option<NetValidator>,
}
```

Private implementation:

```rust
struct PlannedResume {
    partial_path: PathBuf,
    partial_bytes: u64,
    validator: Option<NetValidator>,
}

fn planned_resume(policy: &NetResumePolicy) -> Option<PlannedResume>;
fn selected_response_validator(etag: Option<&str>, last_modified: Option<&str>) -> Option<NetValidator>;
fn parse_strong_etag(value: &str) -> Option<String>;
```

This is the reduced structure/API target for the next implementation slice.
