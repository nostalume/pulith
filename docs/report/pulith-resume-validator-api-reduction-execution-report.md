# Pulith Resume Validator API Reduction Execution Report

## Status

Completed.

This slice implemented the reduced resume validator design rather than the earlier structure-heavy plan. The implementation followed the requested method:

```text
1. determine behavior
2. determine behavior dependencies
3. remove duplicated semantics
4. reduce public structs/functions before adding more behavior
```

Production files changed:

```text
crates/pulith/src/net.rs
crates/pulith/src/lib.rs
```

No `NetAcquireError` hierarchy was introduced. No budget/rate implementation was introduced. `ReqwestAcquire` continues to use `response.chunk().await`; no `bytes_stream()` feature was added.

## Behavior dependency result

The behavior dependency analysis reduced resume validator semantics to one core decision:

```text
Does this resume request have a validator that must be sent as If-Range?
```

That eliminated the need for separate public concepts for:

```text
validator policy
resume validator
response validator
request plan
response action
missing-validator outcome
```

The implementation now encodes behavior dependencies directly in `NetResumeMode`.

## Public API implemented

### `NetResumePolicy`

`NetResumePolicy` now owns only one semantic field:

```rust
pub struct NetResumePolicy {
    pub mode: NetResumeMode,
}
```

Constructors:

```rust
impl NetResumePolicy {
    pub fn restart_only() -> Self;
    pub fn unvalidated(partial_path: impl Into<PathBuf>) -> Self;
    pub fn if_range(partial_path: impl Into<PathBuf>, validator: NetValidator) -> Self;
}
```

Removed old shape:

```rust
pub struct NetResumePolicy {
    pub mode: NetResumeMode,
    pub partial_path: Option<PathBuf>,
}
```

Removed old constructor:

```rust
NetResumePolicy::resume_from(...)
```

The old name hid the unsafe-ish distinction between range without validator and range with validator. The new names make behavior explicit:

```text
unvalidated = Range only
if_range    = Range + If-Range
```

### `NetResumeMode`

Implemented reduced behavior variants:

```rust
pub enum NetResumeMode {
    RestartOnly,
    Unvalidated {
        partial_path: PathBuf,
    },
    IfRange {
        partial_path: PathBuf,
        validator: NetValidator,
    },
}
```

Removed old variant:

```rust
ResumeIfValidated
```

Reason: it implied validation while still allowing missing validator state. The new `IfRange` variant cannot be constructed without a validator.

### `NetValidator`

Added one shared validator type:

```rust
pub enum NetValidator {
    Etag(String),
    LastModified(SystemTime),
}
```

Constructors/helpers:

```rust
impl NetValidator {
    pub fn strong_etag(value: impl Into<String>) -> Option<Self>;
    pub fn last_modified(time: SystemTime) -> Self;
}
```

Behavior:

```text
strong ETag is accepted
weak ETag is rejected
Last-Modified is accepted as an explicit validator value
```

No separate `NetResumeValidator` and `NetResponseValidator` types were added. The same fact is used both as future resume evidence and as the request-side If-Range dependency.

### `NetAcquireEvidence`

Extended with one selected response validator:

```rust
pub struct NetAcquireEvidence {
    // existing fields
    pub validator: Option<NetValidator>,
}
```

Selection rule:

```text
prefer strong ETag
else Last-Modified
else None
```

Weak ETags are not selected.

### `NetResumeEvidence`

Extended with the validator actually used for the resume branch:

```rust
pub struct NetResumeEvidence {
    // existing fields
    pub validator: Option<NetValidator>,
}
```

This records behavior that actually happened:

```text
Unvalidated resume -> validator: None
IfRange resume     -> validator: Some(...)
```

No `MissingValidatorRestarted` outcome was added because missing validator is no longer a runtime branch for validated resume. If the caller has no validator, it cannot construct `IfRange`.

## Private implementation structure

The implementation uses one private operation-local structure:

```rust
struct PlannedResume {
    partial_path: PathBuf,
    partial_bytes: u64,
    validator: Option<NetValidator>,
}
```

And one private planner helper:

```rust
fn planned_resume(policy: &NetResumePolicy) -> Option<PlannedResume>;
```

This replaces the previous helper:

```rust
active_resume(policy) -> Option<(PathBuf, u64)>
```

No `ResumeRequestPlan` or `ResumeResponseAction` was introduced. Header emission stays backend-local because it is short and tied to backend request builders.

Private parser/selector helpers added:

```rust
fn parse_strong_etag(value: &str) -> Option<String>;
fn selected_response_validator(
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Option<NetValidator>;
```

## ureq behavior implemented

For `NetResumeMode::Unvalidated` with non-empty partial:

```text
Range: bytes=<partial_bytes>-
```

No `If-Range` header is sent.

For `NetResumeMode::IfRange` with non-empty partial:

```text
Range: bytes=<partial_bytes>-
If-Range: <validator>
```

On `206`:

```text
validate Content-Range
append partial
persist final destination
record NetResumeOutcome::PartialAppended
record resume.validator
record selected response validator in NetAcquireEvidence.validator
```

On `200` after range request:

```text
restart full fresh stage
record NetResumeOutcome::RangeIgnoredRestarted
```

On `416` after range request:

```text
suppress resume
retry full GET once
record NetResumeOutcome::RangeUnsatisfiableRestarted
```

## reqwest behavior implemented

Reqwest mirrors ureq behavior:

```text
Unvalidated -> Range only
IfRange     -> Range + If-Range
206         -> validate Content-Range then append
200         -> fresh restart evidence
416         -> suppress resume and retry full once
```

Body copy remains:

```rust
while let Some(chunk) = response.chunk().await? {
    stage.write_chunk(&chunk, source.policy.max_bytes).await?;
}
```

No `bytes_stream()` dependency or stream API was added.

## Tests added/updated

Pure behavior tests:

```text
resume_policy_modes_encode_restart_unvalidated_and_if_range
strong_etag_parser_rejects_weak_etag
selected_response_validator_prefers_strong_etag_over_last_modified
```

ureq tests:

```text
ureq_if_range_resume_sends_range_and_if_range_and_appends_206
ureq_unvalidated_resume_sends_range_without_if_range
```

reqwest parity test:

```text
reqwest_if_range_resume_sends_range_and_if_range_and_appends_206
```

Existing resume tests were updated from:

```rust
NetResumePolicy::resume_from(&partial)
```

to:

```rust
NetResumePolicy::unvalidated(&partial)
```

Test helper update:

```text
local test server now records raw HTTP requests so tests assert actual Range / If-Range headers
```

## Semantic reduction achieved

### Removed/avoided public concepts

The implementation does not contain:

```text
NetResumeValidatorPolicy
NetResumeValidator
NetResponseValidator
MissingValidatorRestarted
ResumeRequestPlan
ResumeResponseAction
```

### Removed old ambiguity

Old model:

```text
mode = ResumeIfValidated
partial_path = Option<PathBuf>
```

Problem:

```text
validated resume could exist without validator as a separate fact
```

New model:

```text
RestartOnly
Unvalidated { partial_path }
IfRange { partial_path, validator }
```

The behavior dependency is encoded at construction time.

## Verification

Focused pre-ad-hoc verification passed:

```text
cargo fmt --all
cargo test -p pulith --features 'sync local net ureq hash blake3' net::tests::
  20 passed; 0 failed
cargo test -p pulith --features 'async net reqwest hash blake3' net::tests::reqwest
  9 passed; 0 failed
```

Fresh ad-hoc verification script:

```text
F:\Stratum\TEMP\hermes-verify-pp3hl03y.py
```

Cleanup marker:

```text
AD_HOC_SCRIPT_CLEANED=F:\Stratum\TEMP\hermes-verify-pp3hl03y.py
```

Pass marker:

```text
AD_HOC_VERIFY_PASS pulith resume validator api reduction
```

Commands run by the ad-hoc script:

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

Results:

```text
sync ureq net tests: 20 passed; 0 failed
async reqwest net tests: 9 passed; 0 failed
workspace all-features tests: 59 passed; 0 failed
git diff --check for changed code paths: passed
```

## Remaining next step

The next design slice can now analyze net-owned errors from stable resume behavior:

```text
resume/range validator semantics are now explicit
state/evidence records are distinct from errors
200/206/416 branches are behavior-owned
If-Range mismatch remains represented as server 200 + restart evidence
```

Recommended next slice:

```text
net-owned NetAcquireError hierarchy
```

Constraint for that slice:

```text
NetAcquireError must be domain-first.
PulithError may wrap NetAcquireError.
NetAcquireError must not wrap PulithError as its defining field.
```
