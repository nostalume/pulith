# Pulith Net-Owned Error Hierarchy Execution Report

## Status

Completed.

This slice implemented the planned net-owned error hierarchy after the reduced resume validator API slice. Production code changed only the Pulith core crate paths needed for the behavior:

- `crates/pulith/src/error.rs`
- `crates/pulith/src/lib.rs`
- `crates/pulith/src/net.rs`

## Goal

Move net acquire failures out of global `PulithError` variants and into a behavior-owned `NetAcquireError` domain.

Correct ownership direction:

```text
PulithError::NetAcquire(NetAcquireError)
```

Rejected direction:

```text
NetAcquireError wrapping PulithError
```

## Implemented API

### Public net error type

Added in `crates/pulith/src/net.rs`:

```rust
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
```

Supporting enums:

```rust
pub enum NetTransportPhase {
    SendRequest,
    ReadBody,
}
```

```rust
pub enum NetProtocolError {
    UnexpectedPartialResponse,
    InvalidContentRange {
        expected_start: u64,
        header: Option<String>,
    },
}
```

```rust
pub enum NetUnsafeDestination {
    Symlink,
    NonFile,
}
```

`NetAcquireError` implements:

```rust
impl fmt::Display for NetAcquireError
impl std::error::Error for NetAcquireError
```

`Local` exposes its underlying `io::Error` through `source()`.

## PulithError direction fixed

`PulithError` now has:

```rust
#[cfg(feature = "net")]
NetAcquire(NetAcquireError)
```

And:

```rust
#[cfg(feature = "net")]
impl From<NetAcquireError> for PulithError {
    fn from(error: NetAcquireError) -> Self {
        Self::NetAcquire(error)
    }
}
```

`PulithError::source()` delegates:

```rust
Self::NetAcquire(error) => Some(error)
```

The old net-specific global variants were removed from `PulithError`:

```text
InvalidUrl
UnsupportedUrlScheme
HttpStatus
DownloadLimitExceeded
NetworkError
```

This keeps the global error type as an umbrella, not the owner of net acquire semantics.

## Behavior trait migration

ureq acquire now exposes the domain error directly:

```rust
impl<I> AcquireNode<Chosen<I, RemoteSource>> for UreqAcquire<UreqResource> {
    type Error = NetAcquireError;
}
```

reqwest acquire now exposes the domain error directly:

```rust
impl<I: 'static> AsyncAcquireNode<Chosen<I, RemoteSource>> for ReqwestAcquire<ReqwestResource> {
    type Error = NetAcquireError;
}
```

`RemoteUrl::parse` now returns:

```rust
Result<RemoteUrl, NetAcquireError>
```

instead of `PulithError`.

## Error mappings

### URL parsing

Now maps to:

```text
NetAcquireError::InvalidUrl
NetAcquireError::UnsupportedScheme
```

### HTTP status

Final non-success status now maps to:

```text
NetAcquireError::HttpStatus
```

It carries:

```text
url
status
retryable
attempts
resume
```

### Backend transport

Request send failures map to:

```text
NetAcquireError::Transport { phase: SendRequest, ... }
```

Body read failures map to:

```text
NetAcquireError::Transport { phase: ReadBody, ... }
```

Both carry attempt records and current resume evidence.

### Resume protocol

A `206 Partial Content` without an active resume request maps to:

```text
NetAcquireError::Protocol {
    kind: NetProtocolError::UnexpectedPartialResponse,
    ...
}
```

A `206 Partial Content` with missing or invalid `Content-Range` maps to:

```text
NetAcquireError::Protocol {
    kind: NetProtocolError::InvalidContentRange { expected_start, header },
    ...
}
```

No stringly `NetworkError("invalid Content-Range for resume")` remains in net code.

### Byte limit

Known oversize and streamed body oversize now map to:

```text
NetAcquireError::LimitExceeded
```

It carries:

```text
url
max
actual
attempts
resume
```

### Local/staging failures

Download parent creation, temp creation, partial copy/open, write, flush, persist, and metadata failures now map to:

```text
NetAcquireError::Local
```

Unsafe final destinations now map to:

```text
NetAcquireError::UnsafeDestination
```

with:

```text
Symlink
NonFile
```

## Success behavior preserved

The successful evidence path remains unchanged in shape:

```rust
NetAcquireEvidence {
    url,
    final_path,
    status,
    bytes,
    content_length,
    attempts,
    resume,
    validator,
}
```

Resume outcomes remain evidence, not errors:

```text
PartialAppended
RangeIgnoredRestarted
RangeUnsatisfiableRestarted
```

The previous rule remains intact:

```text
200 after Range/If-Range -> restart evidence
416 after Range/If-Range -> suppress resume and retry full once
```

## Tests updated

Updated tests now assert domain errors:

```text
remote_url_rejects_unsupported_or_relative_urls
pulith_error_wraps_net_acquire_error_as_source
ureq_acquire_rejects_non_success_status_without_touching_destination
ureq_acquire_enforces_max_bytes_before_persist
ureq_resume_missing_content_range_rejects_without_persist
reqwest_acquire_rejects_non_success_status_without_touching_destination
reqwest_acquire_enforces_max_bytes_before_persist
```

Focused sync net tests now cover 21 net tests.

Focused reqwest tests cover 9 async reqwest tests.

## Explicit non-goals preserved

This slice did not introduce:

```text
budget/rate governor
progress callbacks
bytes_stream
sidecar partial metadata
object_store integration
miette/anyhow/snafu/error-stack
full repository error rewrite
```

Reqwest still uses:

```rust
response.chunk().await
```

## Verification

Fresh focused ad-hoc verification was run from an OS-safe temp script under:

```text
F:\Stratum\TEMP\hermes-verify-6eipzkz0.py
```

The script was cleaned:

```text
AD_HOC_SCRIPT_CLEANED=F:\Stratum\TEMP\hermes-verify-6eipzkz0.py
```

Pass marker:

```text
AD_HOC_VERIFY_PASS pulith net owned error hierarchy
```

Commands executed:

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

Results:

```text
sync ureq net tests: 21 passed; 0 failed
async reqwest net tests: 9 passed; 0 failed
workspace all-features: 60 passed; 0 failed
fmt/check/diff-check: passed
```

## Remaining next slice

The next recommended slice is budget/rate, because net acquire now has:

```text
typed retry records
typed resume evidence
typed validator evidence
typed net-owned errors
```

Budget/rate should build on `NetAcquireError::LimitExceeded`, transport phases, retry evidence, and future rate/budget evidence rather than adding new global `PulithError` variants.
