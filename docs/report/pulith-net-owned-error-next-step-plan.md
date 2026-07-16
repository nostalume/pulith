# Pulith Net-Owned Error Hierarchy Next-Step Plan

## Status

Planning only. No production code was changed in this slice.

This report analyzes and prepares the next implementation slice after the completed resume validator API reduction. The recommended next slice is:

```text
net-owned NetAcquireError hierarchy
```

This is now safe to plan because the resume/range/validator behavior is explicit:

```text
RestartOnly
Unvalidated Range
IfRange Range + If-Range
206 append after Content-Range validation
200 restart evidence
416 suppress resume and retry full once
```

The plan keeps the prior correction:

```text
NetAcquireError must own net acquire semantics.
PulithError may wrap/delegate to NetAcquireError.
NetAcquireError must not wrap PulithError as its defining field.
```

## Sources inspected

Current code:

```text
crates/pulith/src/error.rs
crates/pulith/src/net.rs
```

Current reports/references:

```text
references/pulith-resume-first-error-design.md
references/pulith-resume-validator-api-reduction-execution.md
docs/report/pulith-resume-validator-api-reduction-execution-report.md
```

External crate/API research performed with `cargo search` / `cargo info`:

```text
thiserror 2.0.18
anyhow 1.0.103
miette 7.6.0
snafu 0.9.1
error-stack 0.8.0
```

Tool output highlights:

```text
thiserror: derive(Error), rust-version 1.68, MIT OR Apache-2.0
anyhow: flexible concrete Error type, rust-version 1.68
miette: fancy diagnostic reporting/protocol, rust-version 1.70
snafu: ergonomic error handling library, rust-version 1.65
error-stack: context-aware reports, rust-version 1.83
```

Current toolchain observed:

```text
rustc 1.97.0-nightly (2026-05-19)
```

## Research summary

### `thiserror`

Best fit for this next slice if adding a dependency is acceptable.

Why:

```text
library-friendly
small surface
keeps typed enum variants
preserves std::error::Error source chains
works well for public API errors
low conceptual overhead
```

Risk:

```text
adds a dependency for derive convenience
```

Mitigation:

```text
The existing code manually implements Display/Error for PulithError, so the first slice can avoid thiserror and use manual impls. Add thiserror only if boilerplate grows after behavior-specific errors spread beyond net.
```

### `anyhow`

Not recommended for public Pulith library errors.

Reason:

```text
anyhow erases error variants behind a flexible concrete type.
Pulith needs stable behavior-specific variants for tests, evidence, recovery, and caller matching.
```

Potential use:

```text
CLI/application boundary later, not core crate public error model.
```

### `miette`

Not recommended for this core slice.

Reason:

```text
miette is diagnostic/reporting-oriented and useful for human CLI UX.
Net acquire needs behavior-owned structured errors first.
```

Potential use:

```text
future CLI/reporting layer after domain errors are stable.
```

### `snafu`

Not recommended for this first slice.

Reason:

```text
SNAFU can express rich context, but it increases macro/context machinery before the domain taxonomy is stable.
```

### `error-stack`

Not recommended.

Reason:

```text
context reports are heavier than needed and require rust-version 1.83.
Pulith currently benefits more from explicit variants + evidence records.
```

### Recommended dependency choice

For the immediate slice:

```text
No new dependency.
Implement NetAcquireError manually first.
```

Reconsider `thiserror` later if:

```text
multiple behavior-specific error enums exist
manual Display/source impls become repetitive
public error variants are stable
```

## Current problem statement

`crates/pulith/src/error.rs` currently owns net acquire errors as top-level global variants:

```rust
InvalidUrl(String)
UnsupportedUrlScheme(String)
HttpStatus { status: u16, url: String }
DownloadLimitExceeded { max: u64, actual: u64 }
NetworkError(String)
Io { action, path, source }
UnsupportedLocalEntry(PathBuf)
```

`crates/pulith/src/net.rs` emits those directly from net behavior.

Problems:

1. Net semantics are distributed across global `PulithError` variants.
2. Net backend errors are collapsed into stringly `NetworkError(String)`.
3. Protocol failures are also stringly `NetworkError(String)`.
4. Local staging failures use generic global `Io` / `UnsupportedLocalEntry`, even when they happen inside net Acquire.
5. Current retry records record attempt facts, but final failures do not carry the attempt/resume context in a typed net error.
6. Caller cannot distinguish:
   ```text
   URL parse failure
   unsupported scheme
   final non-retryable HTTP status
   exhausted retryable HTTP status
   exhausted network send/body error
   invalid resume protocol response
   local staging failure
   byte limit failure
   ```
   without matching global variants or parsing strings.

## Behavior dependency analysis

### Acquire net behavior

Behavior:

```text
RemoteSource -> LocalMaterial
```

Dependencies:

```text
RemoteUrl
NetAcquirePolicy
backend resource
local staging parent
optional resume partial
```

Failure classes owned by this behavior:

```text
request construction / URL validity
backend request send
HTTP status classification
HTTP response body read
resume protocol validation
byte limit
local staging / persist
unsafe destination preflight
```

### Retry behavior

Behavior:

```text
failed attempt -> retry or final failure
```

Dependencies:

```text
attempt index
status or network error kind
retry policy
Retry-After
```

Failure should not own `NetAttemptEvidence` as success evidence, but final error may carry attempt records because they explain the failure history.

Therefore rename direction is still recommended:

```text
NetAttemptEvidence -> NetAttemptRecord
```

But to reduce churn, this next slice can either:

```text
A. keep the name for now and only attach Vec<NetAttemptEvidence> to NetAcquireError
B. rename to NetAttemptRecord in the same slice if test updates are small
```

Recommended for smaller next slice:

```text
Keep NetAttemptEvidence name temporarily.
Add final error context using Vec<NetAttemptEvidence>.
Rename to Record in a later cleanup unless the compiler churn is minimal.
```

### Resume behavior

Behavior:

```text
partial + Range/If-Range -> append/restart/protocol failure
```

Recoverable branches:

```text
200 to ranged request -> RangeIgnoredRestarted evidence
416 to ranged request -> RangeUnsatisfiableRestarted evidence + retry full once
```

True error branches:

```text
206 without a resume request
206 missing/malformed Content-Range
206 Content-Range start != partial bytes
partial copy/read failure
append body failure after retries exhausted
```

This confirms why errors should wait until after resume validator semantics: `200` and `416` are not automatically errors.

## Reduced error model

Do not create one variant per backend library error type. Model domain failure categories first, backend details second.

Recommended public enum:

```rust
pub enum NetAcquireError {
    InvalidUrl {
        input: String,
    },
    UnsupportedScheme {
        scheme: String,
    },
    HttpStatus {
        url: url::Url,
        status: u16,
        retryable: bool,
        attempts: Vec<NetAttemptEvidence>,
        resume: Option<NetResumeEvidence>,
    },
    Transport {
        url: url::Url,
        phase: NetTransportPhase,
        message: String,
        attempts: Vec<NetAttemptEvidence>,
        resume: Option<NetResumeEvidence>,
    },
    Protocol {
        url: url::Url,
        kind: NetProtocolError,
        attempts: Vec<NetAttemptEvidence>,
        resume: Option<NetResumeEvidence>,
    },
    LimitExceeded {
        url: url::Url,
        max: u64,
        actual: u64,
        attempts: Vec<NetAttemptEvidence>,
        resume: Option<NetResumeEvidence>,
    },
    Local {
        url: Option<url::Url>,
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    UnsafeDestination {
        path: PathBuf,
        kind: NetUnsafeDestination,
    },
}
```

### Supporting enums

```rust
pub enum NetTransportPhase {
    SendRequest,
    ReadBody,
}

pub enum NetProtocolError {
    UnexpectedPartialResponse,
    InvalidContentRange {
        expected_start: u64,
        header: Option<String>,
    },
}

pub enum NetUnsafeDestination {
    Symlink,
    NonFile,
}
```

### Why this is the right boundary

`NetAcquireError` owns net semantics:

```text
URL/scheme
HTTP status
transport/backend failure
resume protocol failure
download limit
net staging/persist local failure
unsafe destination
```

`PulithError` remains the umbrella:

```rust
pub enum PulithError {
    NetAcquire(NetAcquireError),
    // existing non-net variants
}
```

Direction is correct:

```text
PulithError -> wraps NetAcquireError
NetAcquireError -> does not wrap PulithError
```

## What to avoid

Do not implement:

```rust
pub struct NetAcquireError {
    pub error: PulithError,
    pub attempts: Vec<NetAttemptEvidence>,
}
```

That repeats the previous ownership inversion.

Do not over-split into:

```text
NetUrlError
NetHttpError
NetTransportError
NetResumeError
NetLocalStageError
```

unless pattern matching pressure proves those sub-enums are necessary. Start with one public `NetAcquireError` enum and only two small supporting enums:

```text
NetTransportPhase
NetProtocolError
```

`NetUnsafeDestination` is optional; it can be replaced with a string/action if the slice must be smaller, but a tiny enum is cleaner than `UnsupportedLocalEntry` leaking from local apply.

## API pseudocode

### `error.rs`

Minimal transition:

```rust
#[cfg(feature = "net")]
use crate::net::NetAcquireError;

pub enum PulithError {
    // existing variants

    #[cfg(feature = "net")]
    NetAcquire(NetAcquireError),
}

impl From<NetAcquireError> for PulithError {
    fn from(error: NetAcquireError) -> Self {
        Self::NetAcquire(error)
    }
}
```

Display:

```rust
impl fmt::Display for PulithError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NetAcquire(error) => write!(f, "{error}"),
            // existing variants
        }
    }
}
```

Source:

```rust
impl std::error::Error for PulithError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NetAcquire(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
```

### `net.rs`

Add near net public API types:

```rust
#[derive(Debug)]
pub enum NetAcquireError {
    InvalidUrl { input: String },
    UnsupportedScheme { scheme: String },
    HttpStatus {
        url: url::Url,
        status: u16,
        retryable: bool,
        attempts: Vec<NetAttemptEvidence>,
        resume: Option<NetResumeEvidence>,
    },
    Transport {
        url: url::Url,
        phase: NetTransportPhase,
        message: String,
        attempts: Vec<NetAttemptEvidence>,
        resume: Option<NetResumeEvidence>,
    },
    Protocol {
        url: url::Url,
        kind: NetProtocolError,
        attempts: Vec<NetAttemptEvidence>,
        resume: Option<NetResumeEvidence>,
    },
    LimitExceeded {
        url: url::Url,
        max: u64,
        actual: u64,
        attempts: Vec<NetAttemptEvidence>,
        resume: Option<NetResumeEvidence>,
    },
    Local {
        url: Option<url::Url>,
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    UnsafeDestination {
        path: PathBuf,
        kind: NetUnsafeDestination,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetTransportPhase {
    SendRequest,
    ReadBody,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NetProtocolError {
    UnexpectedPartialResponse,
    InvalidContentRange {
        expected_start: u64,
        header: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetUnsafeDestination {
    Symlink,
    NonFile,
}
```

Manual Display/Error first:

```rust
impl fmt::Display for NetAcquireError { ... }
impl std::error::Error for NetAcquireError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Local { source, .. } => Some(source),
            _ => None,
        }
    }
}
```

### Constructors to reduce call-site noise

Add only a few private constructors/helpers, not one per variant if avoidable:

```rust
impl NetAcquireError {
    fn local(
        url: Option<&RemoteUrl>,
        action: &'static str,
        path: impl AsRef<Path>,
        source: io::Error,
    ) -> Self;

    fn transport(
        url: &RemoteUrl,
        phase: NetTransportPhase,
        message: impl Into<String>,
        attempts: Vec<NetAttemptEvidence>,
        resume: Option<NetResumeEvidence>,
    ) -> Self;
}
```

Avoid many tiny public constructors unless callers need them.

### Result types

Current trait impls return `PulithError`:

```rust
impl<I> AcquireNode<Chosen<I, RemoteSource>> for UreqAcquire<UreqResource> {
    type Error = PulithError;
}
```

Recommended transition for typed behavior:

```rust
impl<I> AcquireNode<Chosen<I, RemoteSource>> for UreqAcquire<UreqResource> {
    type Error = NetAcquireError;
}
```

And async:

```rust
impl<I> AsyncAcquireNode<Chosen<I, RemoteSource>> for ReqwestAcquire<ReqwestResource> {
    type Error = NetAcquireError;
}
```

This makes net behavior truly own net errors. Composed callers that need umbrella errors can rely on:

```rust
impl From<NetAcquireError> for PulithError
```

If changing associated error type causes too much churn, use a two-step transition:

```text
Step A: net internals return NetAcquireError; boundary maps into PulithError
Step B: trait impl Error associated type changes to NetAcquireError
```

Recommended for this next slice:

```text
Do Step B directly if tests are local and compile churn is contained.
```

## Call-site migration plan

### URL parsing

Current:

```rust
url::Url::parse(input).map_err(|_| PulithError::InvalidUrl(input.to_string()))?;
Err(PulithError::UnsupportedUrlScheme(scheme.to_string()))
```

Plan:

```rust
url::Url::parse(input)
    .map_err(|_| NetAcquireError::InvalidUrl { input: input.to_string() })?;
Err(NetAcquireError::UnsupportedScheme { scheme: scheme.to_string() })
```

This means `RemoteUrl::parse` should return:

```rust
Result<Self, NetAcquireError>
```

If non-net callers want `PulithError`, they can use `?` through `From<NetAcquireError>`.

### HTTP status

Current:

```rust
return Err(PulithError::HttpStatus { status, url: ... });
```

Plan:

```rust
return Err(NetAcquireError::HttpStatus {
    url: source.url.as_url().clone(),
    status,
    retryable,
    attempts,
    resume,
});
```

Important ownership rule:

```text
Move attempts/resume only on final failure.
On retry branches they remain mutable records.
```

### Backend send/body errors

Current:

```rust
PulithError::NetworkError(err.to_string())
```

Plan:

```rust
NetAcquireError::Transport {
    url: source.url.as_url().clone(),
    phase: NetTransportPhase::SendRequest,
    message: err.to_string(),
    attempts,
    resume,
}
```

Body read failure:

```rust
phase: NetTransportPhase::ReadBody
```

### Resume protocol failures

Current:

```rust
PulithError::NetworkError("partial response without resume request".to_string())
PulithError::NetworkError("invalid Content-Range for resume".to_string())
```

Plan:

```rust
NetAcquireError::Protocol {
    url: source.url.as_url().clone(),
    kind: NetProtocolError::UnexpectedPartialResponse,
    attempts,
    resume,
}
```

and:

```rust
NetAcquireError::Protocol {
    url: source.url.as_url().clone(),
    kind: NetProtocolError::InvalidContentRange {
        expected_start: resume_context.partial_bytes,
        header: content_range_header,
    },
    attempts,
    resume,
}
```

This removes string parsing from tests and callers.

### Download limit

Current:

```rust
PulithError::DownloadLimitExceeded { max, actual }
```

Plan:

```rust
NetAcquireError::LimitExceeded {
    url: source.url.as_url().clone(),
    max,
    actual,
    attempts,
    resume,
}
```

For helper reuse, change:

```rust
reject_known_oversize(...) -> Result<(), NetLimitExceeded>
copy_response_body(...) -> Result<u64, NetBodyCopyError>
```

But do not introduce those extra types yet unless matching gets ugly. Simpler next-slice approach:

```text
Keep helper logic local and map PulithError::DownloadLimitExceeded only temporarily inside net.
Then replace helper error returns after tests pass.
```

Better final shape:

```rust
fn reject_known_oversize(
    content_length: Option<u64>,
    max_bytes: Option<u64>,
) -> Result<(), (u64, u64)>;
```

Where `(max, actual)` is converted into `NetAcquireError::LimitExceeded` at the call site with URL/attempt context.

### Local staging failures

Current:

```rust
PulithError::io("create download parent", &parent, err)
PulithError::UnsupportedLocalEntry(destination.to_path_buf())
```

Plan:

```rust
NetAcquireError::Local {
    url: Some(source.url.as_url().clone()),
    action: "create download parent",
    path: parent,
    source: err,
}
```

Unsafe destination:

```rust
NetAcquireError::UnsafeDestination {
    path: destination.to_path_buf(),
    kind: NetUnsafeDestination::Symlink | NetUnsafeDestination::NonFile,
}
```

For `destination_parent` before source move, pass url if available where useful. If not, `url: None` is acceptable for path-only preflight helpers; but prefer URL at call site where possible.

## Tests for next slice

### Pure/API tests

```text
net_acquire_error_display_includes_domain_context
pulith_error_wraps_net_acquire_error_as_source
remote_url_parse_returns_net_acquire_error
```

### ureq tests

Update existing assertions:

Current:

```rust
assert!(matches!(error, PulithError::HttpStatus { status: 404, .. }));
assert!(matches!(error, PulithError::NetworkError(_)));
assert!(matches!(error, PulithError::DownloadLimitExceeded { max: 3, .. }));
```

Planned:

```rust
assert!(matches!(
    error,
    NetAcquireError::HttpStatus { status: 404, retryable: false, .. }
));
```

```rust
assert!(matches!(
    error,
    NetAcquireError::Protocol {
        kind: NetProtocolError::InvalidContentRange { .. },
        ..
    }
));
```

```rust
assert!(matches!(
    error,
    NetAcquireError::LimitExceeded { max: 3, .. }
));
```

Add final-failure attempt context test:

```text
ureq_http_status_error_carries_attempt_records_after_retries
```

Scenario:

```text
503 then 503, retry max 1
expect NetAcquireError::HttpStatus { status: 503, retryable: true, attempts.len() == 2 }
```

### reqwest parity tests

Do not duplicate every ureq error test. Add minimal parity:

```text
reqwest_http_status_error_carries_attempt_records_after_retries
reqwest_invalid_content_range_is_protocol_error
```

### Feature checks

```text
cargo check -p pulith --no-default-features
cargo check -p pulith --features "sync local net ureq hash blake3"
cargo check -p pulith --features "async net reqwest hash blake3"
```

## Implementation sequence

### Step 1: RED tests

Add/adjust tests to expect `NetAcquireError`:

```text
remote_url_parse_returns_net_acquire_error
ureq_http_status_error_carries_attempt_records_after_retries
ureq_invalid_content_range_is_protocol_error
reqwest_http_status_error_carries_attempt_records_after_retries
```

Run focused tests and confirm RED.

### Step 2: Add `NetAcquireError` type

Add in `net.rs` near public net types:

```text
NetAcquireError
NetTransportPhase
NetProtocolError
NetUnsafeDestination
```

Manual `Display` and `Error` impls.

### Step 3: Add umbrella wrapper

In `error.rs`:

```text
PulithError::NetAcquire(NetAcquireError)
impl From<NetAcquireError> for PulithError
Display delegates
source returns Some(error)
```

Feature-gate with `#[cfg(feature = "net")]` if needed.

### Step 4: Change behavior trait error type

Change:

```rust
type Error = PulithError;
```

to:

```rust
type Error = NetAcquireError;
```

for:

```text
UreqAcquire
ReqwestAcquire
acquire_reqwest
```

### Step 5: Convert URL/scheme errors

Change `RemoteUrl::parse` to return:

```rust
Result<Self, NetAcquireError>
```

### Step 6: Convert status/transport/protocol/limit/local failures

Do in this order:

```text
HTTP status
send request errors
resume protocol errors
body read errors
limit failures
local staging/persist failures
unsafe destination
```

Reason:

```text
HTTP/status/protocol are net-owned and easiest to test.
Local staging may require helper signatures to change.
```

### Step 7: Re-run focused checks and tests

```text
cargo fmt --all
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::
cargo test -p pulith --features "async net reqwest hash blake3" net::tests::reqwest
```

### Step 8: Fresh ad-hoc verification

Use required script path pattern:

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
git diff --check -- crates/pulith/src/error.rs crates/pulith/src/lib.rs crates/pulith/src/net.rs
```

## Acceptance criteria

The next slice is complete when:

```text
NetAcquireError exists and owns net failure semantics.
PulithError wraps NetAcquireError; NetAcquireError does not wrap PulithError.
UreqAcquire and ReqwestAcquire expose NetAcquireError as their behavior Error type.
RemoteUrl::parse returns NetAcquireError.
HTTP status final failure includes status, retryable flag, attempts, resume context.
Transport final failure distinguishes SendRequest vs ReadBody.
Invalid resume Content-Range is NetProtocolError, not NetworkError(String).
Download limit failure is NetAcquireError::LimitExceeded.
Local staging/persist failures are NetAcquireError::Local with source error.
Unsafe destination preflight is net-owned or deliberately left global with a documented reason.
Existing success evidence remains unchanged.
200/416 resume branches remain evidence, not errors.
Reqwest still uses chunk().await; no bytes_stream.
No budget/rate behavior is introduced.
Fresh ad-hoc verification passes and script is cleaned.
```

## Non-goals

Do not do in the next slice:

```text
budget/rate policy
governor/Tower integration
progress callback
sidecar partial metadata
bytes_stream()
object_store error hierarchy
miette/anyhow conversion layer
CLI diagnostic UX
rename every non-net PulithError variant
repository-wide error rewrite
```

## Risk controls

### Risk: large signature churn

Changing `RemoteUrl::parse` and behavior trait `type Error` may affect tests and composition.

Control:

```text
Keep changes inside net tests first.
Use From<NetAcquireError> for PulithError for higher-level composition.
Run feature matrix early.
```

### Risk: attempt records cloned/moved awkwardly

Final errors need `attempts`, but retry loops mutate attempts.

Control:

```text
Move attempts only at final return points.
Avoid attaching attempts to intermediate helper errors.
Convert helper returns at call site where attempts/resume are available.
```

### Risk: over-modeling local IO

Local staging errors could explode into many variants.

Control:

```text
Use one NetAcquireError::Local { action, path, source } variant.
Do not create separate CreateParent/OpenPartial/Flush/Persist variants.
```

### Risk: stringly backend errors remain

Transport errors may still store `message: String`.

Control:

```text
This is acceptable for backend-specific send/read errors because reqwest and ureq error types differ and should not leak as public generic parameters.
The domain-owned part is phase + URL + attempts/resume.
```

## Final recommendation

Implement next:

```text
NetAcquireError domain enum + PulithError wrapper + net behavior Error type migration
```

Use manual `Display`/`Error` first. Do not add `thiserror` yet.

Keep the slice narrow:

```text
error taxonomy only
no budget/rate
no progress
no stream API changes
no full PulithError cleanup outside net
```
