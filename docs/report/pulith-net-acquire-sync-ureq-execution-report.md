# Pulith Sync Net Acquire Execution Report

## Status

Completed the first `net Acquire` implementation slice:

```text
Chosen<I, RemoteSource>
  -> UreqAcquire
  -> Acquired<I, LocalMaterial, NetAcquireEvidence>
```

This is the sync HTTP baseline using `ureq`.

## Files changed

```text
Cargo.toml
crates/pulith/Cargo.toml
crates/pulith/src/error.rs
crates/pulith/src/lib.rs
crates/pulith/src/net.rs
```

## Feature surface

`crates/pulith/Cargo.toml` now wires net features as implementation capability axes:

```toml
net = ["local", "dep:url"]
ureq = ["net", "sync", "dep:ureq"]
reqwest = ["net", "async", "dep:reqwest", "dep:tokio"]
object = ["net", "async", "dep:object_store"]
```

Rationale:

```text
net Acquire currently produces LocalMaterial, so net depends on local.
RemoteUrl uses url::Url, so net owns dep:url.
ureq is the sync implementation family.
reqwest/object remain future async implementation families.
```

## New module

```text
crates/pulith/src/net.rs
```

Exported from `lib.rs` behind feature gates:

```rust
#[cfg(feature = "net")]
pub mod net;

#[cfg(feature = "net")]
pub use net::{NetAcquireEvidence, NetAcquirePolicy, RemoteSource, RemoteUrl};

#[cfg(feature = "ureq")]
pub use net::{UreqAcquire, UreqResource};
```

## Public typed contract

### `RemoteUrl`

```rust
pub struct RemoteUrl {
    pub url: url::Url,
}
```

Behavior:

```text
RemoteUrl::parse accepts absolute http/https URLs.
Relative URLs are rejected as InvalidUrl.
Non-http schemes are rejected as UnsupportedUrlScheme.
```

### `NetAcquirePolicy`

```rust
pub struct NetAcquirePolicy {
    pub timeout: Option<Duration>,
    pub max_bytes: Option<u64>,
    pub headers: Vec<(String, String)>,
}
```

First slice policies:

```text
timeout -> request-level ureq timeout_global
max_bytes -> hard download byte limit before persist
headers -> request headers
```

Not implemented yet:

```text
retry
resume/range
mirror race
progress callbacks
bandwidth throttling
```

### `RemoteSource`

```rust
pub struct RemoteSource {
    pub url: RemoteUrl,
    pub destination: PathBuf,
    pub policy: NetAcquirePolicy,
}
```

This keeps `AcquireNode` unchanged. The request facts live in the selected source:

```text
Chosen<I, RemoteSource>
```

### `NetAcquireEvidence`

```rust
pub struct NetAcquireEvidence {
    pub url: url::Url,
    pub final_path: PathBuf,
    pub status: u16,
    pub bytes: u64,
    pub content_length: Option<u64>,
}
```

Evidence answers only Acquire questions:

```text
which URL was used?
where are the bytes now?
what HTTP status was observed?
how many bytes were written?
was Content-Length known?
```

It does not assert digest trust, archive safety, or install/apply success.

## Sync implementation

### `UreqResource`

```rust
pub struct UreqResource {
    pub agent: ureq::Agent,
}
```

`UreqResource::default()` uses:

```rust
ureq::Agent::new_with_defaults()
```

The agent is a reusable connection-pool resource. There is no hidden global singleton.

### `UreqAcquire`

```rust
impl<I> AcquireNode<Chosen<I, RemoteSource>> for UreqAcquire<UreqResource>
```

Output:

```rust
Acquired<I, LocalMaterial, NetAcquireEvidence>
```

Material:

```rust
LocalMaterial {
    path: source.destination,
    kind: MaterialKind::File,
}
```

## File placement behavior

Implementation uses same-parent staged file placement:

```text
create destination parent
reject existing destination if symlink or non-file
NamedTempFile::new_in(destination_parent)
stream response body into temp file
enforce max_bytes before writing excessive chunk
flush temp file
persist temp file to final destination
return LocalMaterial::File
```

Important guarantees:

```text
non-2xx status returns before temp file/persist
max_bytes error returns before final destination exists or is replaced
existing symlink destination is rejected
existing directory/special destination is rejected
no pulith-fs Workspace or old Fetcher choreography is used
```

## Network behavior

Implementation uses `ureq` sync HTTP:

```text
reusable ureq::Agent
request-level http_status_as_error(false)
manual success-status check
request-level timeout_global when policy.timeout is set
body_mut().as_reader() streaming read
content_length from response body metadata
```

Only successful HTTP statuses are accepted in this slice:

```text
2xx -> success
non-2xx -> PulithError::HttpStatus
```

## Error additions

Added transitional network errors to `PulithError`:

```rust
InvalidUrl(String)
UnsupportedUrlScheme(String)
HttpStatus { status: u16, url: String }
DownloadLimitExceeded { max: u64, actual: u64 }
NetworkError(String)
```

Existing `PulithError::io` is used for file-placement I/O.

## Tests added

All tests are deterministic and use a local loopback HTTP server built with `std::net`.

No external network dependency is used.

Tests:

```text
remote_url_accepts_http_https
remote_url_rejects_unsupported_or_relative_urls
remote_source_preserves_destination_and_default_policy
ureq_acquire_downloads_file_to_local_material
ureq_acquire_rejects_non_success_status_without_touching_destination
ureq_acquire_enforces_max_bytes_before_persist
net_acquire_flows_into_hash_verify
net_acquire_flows_into_local_apply_after_verify
```

The composition tests prove:

```text
Net Acquire -> Hash Verify
Net Acquire -> Hash Verify -> IdentityPrepare -> LocalApply
```

## Explicitly deferred

Still deferred by design:

```text
async reqwest Acquire
object_store Acquire
retry policy
resume/range downloads
mirror/multi-source race
progress/performance callbacks
bandwidth throttling
checksum verification inside Acquire
archive Prepare integration beyond existing typed composition
```

## Next recommended slice

Stop after this baseline and verify. Then choose one:

```text
A. Add retry policy to NetAcquirePolicy.
B. Add async reqwest mirror of the same evidence/material contract.
C. Add Range/resume only if required by a concrete user path.
```

Recommendation:

```text
Do async reqwest next only after this sync ureq path is accepted.
```
