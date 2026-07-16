# Pulith Resume/Range Execution Report

## Status

Implemented the first minimal resume/range behavior slice for `net Acquire`.

This slice follows the corrected order:

```text
retry baseline -> resume/range semantics -> records -> net-owned errors later
```

No net-owned error hierarchy is introduced yet. `PulithError` remains the transitional error type for this slice because resume behavior needed to be settled first.

## TDD notes

RED was observed before implementation. The first focused test run failed because the new resume API and parser did not exist:

```text
no field `resume` on type `NetAcquirePolicy`
cannot find type `NetResumePolicy`
cannot find function `parse_content_range`
no field `resume` on type `NetAcquireEvidence`
cannot find type `NetResumeOutcome`
```

Then the minimal implementation was added and the focused tests passed.

## Implemented public policy/evidence

Added:

```rust
NetResumePolicy
NetResumeMode
NetResumeEvidence
NetResumeOutcome
```

`NetAcquirePolicy` now includes:

```rust
pub resume: NetResumePolicy
```

Default remains conservative:

```rust
NetResumePolicy::restart_only()
```

Explicit opt-in:

```rust
NetAcquirePolicy::default()
    .resume(NetResumePolicy::resume_from(partial_path))
```

`NetAcquireEvidence` now includes:

```rust
pub resume: Option<NetResumeEvidence>
```

This is success evidence only. It records successful resume/restart outcome after a completed Acquire.

## Implemented resume outcomes

```rust
NetResumeOutcome::PartialAppended
NetResumeOutcome::RangeIgnoredRestarted
NetResumeOutcome::RangeUnsatisfiableRestarted
```

Meaning:

```text
PartialAppended:
  server returned 206 and Content-Range matched the partial byte offset.

RangeIgnoredRestarted:
  server returned 200 to a ranged request; Pulith treated this as safe full restart.

RangeUnsatisfiableRestarted:
  server returned 416 to a ranged request; Pulith treated stale partial as recoverable and restarted once.
```

## Implemented behavior

### Request construction

When resume is enabled and the partial file exists with nonzero length:

```text
Range: bytes=<partial_len>-
```

The first slice does not yet add `If-Range` validators. Validator design remains next work.

### 206 Partial Content

For resume requests, 206 is accepted only if:

```text
Content-Range exists
Content-Range unit is bytes
start == partial_len
end >= start
total, if present, is consistent
```

Then Pulith copies the partial into a same-parent temp stage, appends the response body, closes the stage, and persists only the closed stage.

### 200 OK to ranged request

Treated as recoverable restart:

```text
ignore partial
write fresh body from byte 0
record RangeIgnoredRestarted
```

This matches the resume-first design: 200 to Range is not necessarily an error.

### 416 Range Not Satisfiable

Treated as stale/invalid partial and restarted once:

```text
record RangeUnsatisfiableRestarted
disable resume for the next internal request
restart full GET
```

If the restart fails, existing transitional errors still apply.

### Invalid 206 Content-Range

A 206 without a valid `Content-Range` fails before append and before final persist.

Current transitional error:

```text
PulithError::NetworkError("invalid Content-Range for resume")
```

This is intentionally not a final error taxonomy. Net-owned errors come after resume semantics stabilize.

## Backend coverage

Implemented for both current net backends:

```text
sync ureq Read loop
async reqwest chunk loop
```

`reqwest::Response::chunk().await` remains the body-copy primitive. `bytes_stream()` was not enabled.

## Staging behavior

Sync ureq:

```text
NamedTempFile in destination parent
copy partial into temp for 206
append response body into same temp
flush
persist closed temp
```

Async reqwest:

```text
StagedDownload<Open>::new_in(parent)
StagedDownload<Open>::from_partial(parent, partial)
write chunk loop
finish -> StagedDownload<Closed>
Closed::persist(destination)
```

The existing closed-stage persist law is preserved.

## Added tests

Policy/parser:

```text
resume_policy_defaults_to_restart_only
content_range_requires_expected_resume_start
```

Sync ureq:

```text
ureq_resume_206_appends_after_valid_content_range
ureq_resume_200_to_range_restarts_full_with_fresh_stage
ureq_resume_missing_content_range_rejects_without_persist
```

Async reqwest:

```text
reqwest_resume_206_appends_after_valid_content_range
reqwest_resume_416_restarts_once_without_persisting_partial
```

## Verification performed before final ad-hoc

Focused sync tests:

```text
cargo fmt --all
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::
```

Result:

```text
15 passed; 0 failed
```

Focused async reqwest tests:

```text
cargo test -p pulith --features "async net reqwest hash blake3" net::tests::reqwest
```

Result:

```text
8 passed; 0 failed
```

Compile checks:

```text
cargo check -p pulith --no-default-features
cargo check -p pulith --features "sync local net ureq hash blake3"
cargo check -p pulith --features "async net reqwest hash blake3"
```

Result:

```text
all passed
```

## Final ad-hoc verification

Fresh ad-hoc verification passed.

Script:

```text
F:\Stratum\TEMP\hermes-verify-hchuy0ja.py
```

Cleanup:

```text
AD_HOC_SCRIPT_CLEANED=F:\Stratum\TEMP\hermes-verify-hchuy0ja.py
```

Marker:

```text
AD_HOC_VERIFY_PASS pulith resume range execution
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
git diff --check -- crates/pulith/src/net.rs crates/pulith/src/lib.rs docs/report/pulith-resume-range-execution-report.md
```

Result summary:

```text
sync ureq net tests: 15 passed; 0 failed
async reqwest net tests: 8 passed; 0 failed
local/hash tests: 9 passed; 0 failed
workspace all-features tests: 53 passed; 0 failed
git diff --check: passed
```

## Intentional non-goals

Not implemented in this slice:

```text
If-Range
ETag/Last-Modified validator policy
persisted partial metadata
net-owned NetAcquireError hierarchy
bytes_stream()
shared budget/rate accounting
governor/Tower/http-range crates
runtime-neutral async backend
```

## Next step

The next slice should add validators before error hierarchy:

```text
NetResumeValidator
If-Range request header
ETag / Last-Modified capture
validator mismatch evidence
206/200/416 tests with validators
```

Only after that should Pulith split net-owned errors away from the transitional `PulithError` umbrella.
