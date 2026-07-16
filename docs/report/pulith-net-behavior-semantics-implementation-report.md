# Pulith Net Behavior Semantics Implementation Report

## Status

Completed.

This slice continues the reduced net behavior plan and applies the requested three-part work:

1. Behavior-semantic construction and ADT/method quality cleanup.
2. Unified request-admission design.
3. Byte bandwidth pacing impact analysis against the old behavior tree, preserving orthogonality.

Changed production paths:

```text
crates/pulith/src/net.rs
crates/pulith/src/lib.rs
```

Report path:

```text
docs/report/pulith-net-behavior-semantics-implementation-report.md
```

## Step 1 — Behavior semantics, ADT methods, and initialization reduction

### Problem found

`NetAttemptEvidence` was repeatedly constructed by hand at every attempt terminal point:

```text
network send failure
HTTP status failure
known content-length limit failure
body-copy transport failure
body-copy limit failure
body-copy local I/O failure
success
admission rejection
```

Each site repeated the same absence/default shape:

```text
status: None or Some(status)
bytes: 0
content_length: None or content_length
retry_after: None
planned_delay: None or planned_delay
admission_wait: None or admission_wait
outcome: ...
```

This made every call site manually re-encode the ADT shape instead of asking the evidence type to construct its own valid records.

### Implemented cleanup

Added `NetAttemptEvidence` constructors/builders:

```rust
NetAttemptEvidence::new(attempt, outcome)
NetAttemptEvidence::response(attempt, status, content_length, admission_wait, outcome)
NetAttemptEvidence::transfer(attempt, status, bytes, content_length, admission_wait, outcome)
.with_status(status)
.with_bytes(bytes)
.with_content_length(content_length)
.with_retry_after(retry_after)
.with_planned_delay(planned_delay)
.with_admission_wait(admission_wait)
```

The constructors encode default absence once:

```text
status/content_length/retry_after/planned_delay/admission_wait default to None
bytes defaults to 0
```

Refactored all production `attempts.push(NetAttemptEvidence { ... })` construction sites to use those methods.

Result:

```text
remaining production direct NetAttemptEvidence struct initializations: 0
```

The only remaining direct `NetAttemptEvidence { ... }` is in the constructor regression test, where it intentionally proves the default absence contract.

### Resume evidence reduction

Added private method on the private resume plan:

```rust
PlannedResume::into_evidence(outcome)
```

Refactored repeated `NetResumeEvidence { outcome, partial_path, partial_bytes, validator }` construction in ureq/reqwest resume branches.

This keeps resume construction owned by the planned resume actor rather than duplicating field movement at every branch.

### Resource initialization cleanup

Reduced duplicated `None` initialization in resource constructors:

```rust
UreqResource::from_agent(agent) -> Self { agent, ..Self::default() }
ReqwestResource::from_client(client) -> Self { client, ..Self::default() }
```

This removes repeated manual initialization of:

```text
delay: default delay
admission: None
```

## Step 2 — Unified admission design

The current code now uses one admission vocabulary for sync and async backends:

```text
NetAdmissionMode
NetAdmissionPermit
NetAdmissionError
NetSyncAdmission
NetAsyncAdmission
```

Backend-specific differences are limited to the trait boundary:

```text
ureq: NetSyncAdmission::enter()
reqwest: NetAsyncAdmission::enter() -> Future
```

The behavior remains the same in both backends:

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

Admission outcomes are represented uniformly:

```text
success -> NetAttemptEvidence::admission_wait
failure -> NetAcquireError::Admission { kind, attempts, resume }
coarse attempt outcome -> NetAttemptOutcome::AdmissionRejected
```

No extra budget/rate/concurrency public structs were introduced.

Intentionally still absent:

```text
NetBudgetPolicy
NetBudgetEvidence
NetRateMode
NetConcurrencyEvidence
public no-op admission structs
```

## Step 3 — Byte bandwidth pacing and old behavior tree orthogonality

### Actor-model position

Byte bandwidth pacing belongs to `BodyCopy`, not to request admission.

Current raw body actors:

```text
ureq: copy_response_body(reader, writer, max_bytes, initial_bytes)
reqwest: response.chunk().await -> StagedDownload<Open>::write_chunk(chunk, max_bytes)
```

Those actors are downstream of:

```text
ResumePlan
Admission
RequestBuild
SendRequest
ResponseClassify
```

and upstream of:

```text
Persist
Success evidence
```

### Impact assessment

Pacing should not change or own:

```text
RemoteUrl
RemoteSource
NetAcquirePolicy timeout/max_bytes/headers
NetRetryPolicy
NetResumePolicy / NetResumeMode / NetValidator
NetAdmissionMode / NetSyncAdmission / NetAsyncAdmission
NetAcquireError status/protocol/admission semantics
NetAttemptEvidence retry/resume/admission fields
```

Pacing only affects the time at which body bytes are read/written.

It must not reinterpret:

```text
retry eligibility
Range / If-Range / Content-Range validation
206 append vs 200 restart vs 416 restart
admission rejection
max_bytes limit failure
transport phase classification
```

### Recommended future shape

When implemented, byte pacing should be a per-chunk body-copy actor, for example:

```text
NetBytePacer::before_chunk(len).await
copy/write chunk
```

or sync equivalent:

```text
NetBytePacer::before_chunk(len)
copy/write chunk
```

It should be injected into body-copy resources or policy as a body-copy concern, not merged into request admission.

## Verification summary

A RED test was added first:

```text
net::tests::attempt_evidence_constructors_encode_default_absence
```

It failed before the constructor methods existed, then passed after implementation.

Focused green checks already run during implementation:

```text
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::attempt_evidence_constructors_encode_default_absence
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::ureq_resume_206_appends_after_valid_content_range
cargo test -p pulith --features "async net reqwest hash blake3" net::tests::reqwest_resume_206_appends_after_valid_content_range
```

Final fresh ad-hoc verification is recorded separately in the assistant response.
