# Pulith Resume-First Error Design Plan

## Status

This is a corrected planning report after the user pointed out two issues:

1. The previous `NetAcquireError { error: PulithError, ... }` shape inverts ownership.
2. Resume/Range can change error behavior, so error design should not be finalized before resume behavior is understood.

No production code is changed by this report.

## Correction accepted

The prior proposal:

```rust
pub struct NetAcquireError {
    error: PulithError,
    attempts: Vec<NetAttemptRecord>,
}
```

is wrong as a durable design because `PulithError` is broader and less behavior-specific than `NetAcquireError`.

That creates a dependency inversion problem:

```text
behavior-specific net error depends on global umbrella error
```

when the direction should be:

```text
behavior-specific errors define precise semantics;
umbrella error, if needed, wraps or delegates to behavior-specific errors.
```

Correct long-term direction:

```rust
pub enum NetAcquireError { ... }        // owns net-acquire failure semantics
pub enum PulithError { Net(NetAcquireError), ... }  // optional umbrella, later
```

or, for typed behavior traits:

```rust
impl AcquireNode<...> for UreqAcquire<...> {
    type Error = NetAcquireError;
}
```

No `NetAcquireError` should contain `PulithError` as its primary source of meaning.

## Why resume must come before final error design

The current retry-only model has failure categories like:

```text
HTTP status
network failure
local staging failure
download limit
```

But resume/range introduces new branches that are not merely variants of those categories:

```text
server supports range and returns 206
server ignores range and returns 200
server rejects range and returns 416
If-Range validator matches
If-Range validator mismatches
stored partial validator is missing/weak/stale
partial length disagrees with Content-Range
server sends malformed Content-Range
resume policy chooses restart vs error
partial staging becomes unsafe and must be discarded
```

If we design errors before these branches, we either:

1. collapse them into vague `HttpStatus` / `NetworkError`; or
2. create an error type that must be rewritten immediately after resume.

Therefore, optimize order:

```text
resume/range behavior model first -> net error taxonomy second -> implementation slices third
```

## Current baseline inspected

Current implementation has:

```text
RemoteUrl
RemoteSource
NetAcquirePolicy { timeout, max_bytes, headers, retry }
NetRetryPolicy
NetAttemptEvidence
NetAttemptOutcome
NetAcquireEvidence { url, final_path, status, bytes, content_length, attempts }
UreqResource { agent, delay }
ReqwestResource { client, delay }
retry loops in ureq and reqwest
```

Current error state:

```text
UreqAcquire::Error = PulithError
ReqwestAcquire::Error = PulithError
PulithError contains InvalidUrl, UnsupportedUrlScheme, HttpStatus, DownloadLimitExceeded, NetworkError, Io, etc.
```

This is acceptable as transitional implementation, but not as the next design target.

## Research performed

### HTTP Range / Resume references

Searched/read:

```text
MDN Range header
MDN If-Range header
MDN Content-Range header
MDN ETag header
MDN Last-Modified header
MDN Accept-Ranges header
MDN 206 Partial Content
MDN 416 Range Not Satisfiable
```

Key behavior facts:

```text
Range requests ask for part of a resource.
Successful range response is 206 Partial Content.
Invalid range can return 416 Range Not Satisfiable.
Server may ignore Range and return 200 OK with the full representation.
If-Range makes Range conditional: if validator matches, return 206; if not, return 200 full body.
Validators can be ETag or Last-Modified.
Content-Range is required to understand the returned byte interval and total length for 206.
Accept-Ranges advertises support but is not sufficient as proof; behavior is decided by response status and headers.
```

### Rust/crate search

Commands run:

```text
cargo search --registry crates-io content-range --limit 10
cargo search --registry crates-io http-range --limit 10
cargo info --registry crates-io headers
cargo info --registry crates-io http-range-header
```

Findings:

```text
headers 0.4.1
  typed HTTP headers from hyperium; may help if Pulith adopts the `http` header ecosystem broadly.

http-range-header 0.4.2
  no-dependency Range header parser; useful if parsing arbitrary Range headers is needed.

http-content-range 0.2.5
  Content-Range response parser candidate; would need inspection before use.

range-requests / async_http_range_reader / http-range-client
  larger client abstractions; likely too much for Pulith's typed behavior boundary.
```

Decision for now:

```text
Do not add a Range crate in the design slice.
For first implementation, construct simple `Range: bytes=N-` ourselves.
Parse only the response fields Pulith needs: status, Content-Range start/end/complete length, ETag/Last-Modified.
Re-evaluate `http-content-range` only if parser code grows beyond a tiny focused parser.
```

## Resume behavior model before errors

### New concepts

Resume is not just a retry modifier. It introduces partial material state.

Potential public policy:

```rust
pub struct NetResumePolicy {
    pub mode: NetResumeMode,
    pub validator: NetResumeValidatorPolicy,
}

pub enum NetResumeMode {
    RestartOnly,
    ResumeIfValidated,
}

pub enum NetResumeValidatorPolicy {
    StrongEtagOnly,
    EtagOrLastModified,
}
```

Default should remain conservative:

```text
RestartOnly
```

Rationale:

```text
Range/resume can corrupt final material if validators and byte ranges are wrong.
It must be explicit caller policy, not automatic retry behavior.
```

### Internal partial state

Resume needs operation-local partial state, not public acquired material.

Internal shapes could be:

```rust
struct PartialDownload {
    path: PathBuf,
    bytes: u64,
    validator: Option<ResumeValidator>,
}

struct ResumeValidator {
    etag: Option<String>,
    last_modified: Option<SystemTime>,
}
```

But this must not become public success state.

Public success remains:

```text
Acquired<I, LocalMaterial, NetAcquireEvidence>
```

### Response branches

The resume decision table should be designed and tested before error variants are named.

#### No partial exists

```text
Request: normal GET
Expected success: 200
If stream fails: retry may restart from byte 0
No Range header
```

#### Partial exists and policy allows resume

If validator exists:

```text
Request:
Range: bytes=<partial_len>-
If-Range: <etag or http-date>
```

If no valid validator:

```text
Do not send Range.
Restart from byte 0 or error depending policy.
```

#### Server returns 206 Partial Content

Required checks:

```text
Content-Range exists.
Content-Range start == partial_len.
Content-Range end >= start.
Content-Range total, if known, is compatible.
Response is appended to partial staging.
Final bytes equal total if total known.
Only closed/validated staged download persists.
```

Failure examples:

```text
missing Content-Range
Content-Range starts at wrong offset
Content-Range total smaller than partial_len
append would exceed max_bytes
```

These are resume-protocol errors, not generic `HttpStatus`.

#### Server returns 200 OK to ranged request

Meaning:

```text
server ignored Range or If-Range validator failed.
```

Behavior should be policy-driven:

```text
safe default: discard partial and restart full download into fresh staging.
record outcome as RestartedFromScratch, not failure.
```

This means a 200 response to a Range request is not necessarily an error.

This is why error design must wait for resume behavior.

#### Server returns 416 Range Not Satisfiable

Possible meanings:

```text
partial is longer than current representation
representation changed
partial metadata is stale/corrupt
server does not accept requested range
```

Policy decision:

```text
safe first behavior: discard partial and restart full download once.
if restart also fails, error should mention stale/invalid range path.
```

Again, 416 may be recoverable, not final error.

#### If-Range mismatch

Observed as:

```text
server returns 200 full body
```

Behavior:

```text
restart full; do not append.
record resume outcome as validator mismatch/restart.
```

No error if restart succeeds.

## Evidence vs record vs error after resume

After modeling resume, the naming should be:

```text
NetAttemptRecord     = one HTTP operation attempt record, success or failure.
NetResumeRecord      = resume-specific decision/outcome record.
NetAcquireEvidence   = successful Acquire evidence; may include compact operation records.
NetAcquireError      = failed Acquire semantics; owns attempt/resume records relevant to failure.
PulithError          = transitional umbrella only, not owned by NetAcquireError.
```

Recommended success evidence shape after resume:

```rust
pub struct NetAcquireEvidence {
    pub url: url::Url,
    pub final_path: PathBuf,
    pub status: u16,
    pub bytes: u64,
    pub content_length: Option<u64>,
    pub attempts: Vec<NetAttemptRecord>,
    pub resume: Option<NetResumeRecord>,
}
```

But do not commit to this exact shape until tests decide how much resume detail belongs to success evidence.

Recommended error direction:

```rust
pub enum NetAcquireError {
    Request(NetRequestError),
    Response(NetResponseError),
    Resume(NetResumeError),
    Staging(NetStagingError),
    Limit(NetLimitError),
}
```

with diagnostic payload owned by net types:

```rust
pub struct NetFailureContext {
    pub attempts: Vec<NetAttemptRecord>,
    pub resume: Option<NetResumeRecord>,
}
```

But do not implement this before the resume decision table is tested.

## Revised slice order

### Slice 1 — Resume design and tests only

Write a dedicated design report and RED tests for resume semantics before changing error types.

Test names:

```text
resume_policy_defaults_to_restart_only
resume_partial_request_uses_range_and_if_range_when_validator_exists
resume_206_appends_only_when_content_range_starts_at_partial_len
resume_200_to_range_discards_partial_and_restarts_full
resume_416_discards_stale_partial_and_restarts_once
resume_missing_content_range_on_206_is_protocol_error
resume_wrong_content_range_start_is_protocol_error
resume_never_persists_partial_after_failed_append
```

No new error hierarchy yet. Tests can initially assert current broad errors or placeholder internal result enum.

### Slice 2 — Internal resume engine, no public error split

Implement internal helpers/types with minimal public surface:

```text
NetResumePolicy
NetResumeMode
ResumeValidator
PartialDownload internal state
Content-Range parser internal helper
```

Keep Acquire error as current `PulithError` temporarily while resume branching stabilizes.

Rationale:

```text
avoid designing NetAcquireError before knowing which resume failures are final, recoverable, or successful restarts.
```

### Slice 3 — Stabilize attempt/resume records

Rename:

```text
NetAttemptEvidence -> NetAttemptRecord
```

Add if needed:

```text
NetResumeRecord
NetResumeOutcome
```

Outcome examples:

```text
NotAttempted
RestartOnly
RangeRequested
PartialAppended
RangeIgnoredRestarted
RangeUnsatisfiableRestarted
ProtocolRejected
```

### Slice 4 — Net-owned error hierarchy

Only after resume behavior is stable, introduce net-owned error types.

Direction:

```rust
pub enum NetAcquireError {
    InvalidRequest(NetInvalidRequest),
    HttpFinal(NetHttpFailure),
    Network(NetNetworkFailure),
    ResumeProtocol(NetResumeProtocolFailure),
    Staging(NetStagingFailure),
    Limit(NetLimitFailure),
}
```

`PulithError` should not be inside these as the semantic source.

If a global umbrella remains:

```rust
pub enum PulithError {
    NetAcquire(NetAcquireError),
    Archive(ArchiveError),
    Local(LocalError),
    Hash(HashError),
    Io { ... }, // only if genuinely cross-domain
}
```

or keep `PulithError` only for transitional non-net behavior while net uses associated `type Error = NetAcquireError`.

### Slice 5 — Budget/rate after resume+error

Only then decide budget behavior.

Reason:

```text
resume can transform a would-be failure into restart success;
budget must count both retry attempts and resume restarts;
therefore budget errors need final resume semantics first.
```

Budget dimensions:

```text
max attempts
max restarts
max total bytes written including discarded partials
max total sleep
max elapsed time
shared limiter/resource if needed
```

Do not add governor until a shared limiter resource is explicitly modeled.

## Immediate next task

Do not implement `NetAcquireError` yet.

Next task should be:

```text
Write resume/range behavior design + RED tests around decision table.
```

Implementation should initially be allowed to keep broad `PulithError` so the resume behavior can settle before final error hierarchy.

Acceptance:

```text
1. Design covers 200/206/416, If-Range, ETag, Last-Modified, Content-Range.
2. Tests prove 200-to-Range and 416 can be recovery paths, not always errors.
3. Tests prove malformed/mismatched Content-Range is protocol failure.
4. Tests prove partial material is never persisted after failed append.
5. Only after this, design `NetAcquireError` as net-owned, not wrapping PulithError.
```

## Updated correction summary

```text
Old wrong order:
retry -> NetAcquireError wrapping PulithError -> budget -> resume

Corrected order:
retry baseline -> resume/range behavior semantics -> attempt/resume records -> net-owned error hierarchy -> budget/rate
```

This avoids both issues:

```text
No PulithError/NetAcquireError ownership inversion.
No premature error taxonomy before resume changes failure/recovery semantics.
```
