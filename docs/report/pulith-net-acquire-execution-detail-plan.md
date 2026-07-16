# Pulith Net Acquire Execution Detail Plan

## Status

This report converts the returned `net Acquire` task list into concrete execution slices before implementation.

No code changes are made in this pass. The work here is design/detailing plus source research for HTTP/download quality.

## Sources inspected

### Current Pulith single-crate state

Files read:

```text
Cargo.toml
crates/pulith/Cargo.toml
crates/pulith/src/behavior.rs
crates/pulith/src/local.rs
crates/pulith/src/hash.rs
crates/pulith/src/archive.rs
```

Current relevant feature surface:

```toml
sync = []
async = []
local = ["dep:same-file", "dep:tempfile", "dep:walkdir"]
net = []
reqwest = ["async", "dep:reqwest", "dep:tokio"]
ureq = ["sync", "dep:ureq"]
object = ["async", "dep:object_store"]
```

Workspace deps already include:

```toml
ureq = { version = "3.3", default-features = false, features = ["rustls"] }
reqwest = { version = "0.13", default-features = false, features = ["rustls"] }
url = { version = "2.5", features = ["serde"] }
tempfile = "3"
```

Current typed behavior path already available:

```text
Intent -> WithSource -> Chosen -> Acquired -> Verified -> Prepared -> Applied -> Remembered
```

`net Acquire` must join this path by producing `Acquired<I, LocalMaterial, NetAcquireEvidence>`.

### Old `pulith-fetch` implementation read

Files read:

```text
crates/pulith-fetch/Cargo.toml
crates/pulith-fetch/src/net/http.rs
crates/pulith-fetch/src/fetch/fetcher.rs
crates/pulith-fetch/src/config/fetch_options.rs
crates/pulith-fetch/src/fetch/multi_source.rs
crates/pulith-fetch/src/fetch/resumable.rs
crates/pulith-fetch/src/rate/backoff.rs
crates/pulith-fetch/src/segment/validation.rs
crates/pulith-fetch/src/error.rs
```

Useful old behavior to keep as lessons, not API:

```text
retry policy with max_retries/base_backoff
progress phases: Connecting/Downloading/Verifying/Committing/Completed
stream to staging before commit
content length evidence when available
checksum can be separate Verify stage instead of integrated fetch
multi-source strategy exists but should not be first net Acquire slice
Range/resume exists but should be deferred
redirect recognition exists but should rely on client behavior first
```

Old behavior to reject or avoid:

```text
pulith_fs::Workspace dependency
old Fetcher/FetchReceipt public choreography
progress/performance metrics in first slice
resume/checkpoint machinery in first slice
multi-source/race behavior in first slice
checksum verification inside Acquire as default path
mock tests that assert only "ok or err" instead of real behavior
```

### External crate/documentation research

Commands run:

```bash
cargo info --registry crates-io ureq
cargo info --registry crates-io reqwest
cargo info --registry crates-io url
cargo info --registry crates-io tiny_http
```

Docs/source read:

```text
ureq README/docs and local crate source
reqwest docs/source snippets
tiny_http crate info/source snippets
url docs
MDN HTTP status overview
```

Findings:

#### ureq 3.3

`cargo info` findings:

```text
Simple, safe HTTP client
version: 3.3.0
rust-version: 1.85
features default = [rustls, gzip]
rustls feature available
```

Docs/source findings:

```text
blocking I/O by design
Agent holds connection pool and can be cheaply cloned
timeout_global is available via Agent::config_builder()
body is read through response.body_mut()
HTTP 4xx/5xx are errors by default via http_status_as_error=true
http_status_as_error can be disabled
```

Quality implications:

```text
Use ureq first for sync Acquire.
Use an Agent resource, not one-off global calls.
Configure timeout_global from NetAcquireNeed.
Do not enable gzip by default unless wanted; current workspace uses default-features=false + rustls, so no implicit decompression feature.
Map ureq status errors distinctly where possible.
```

#### reqwest 0.13

`cargo info` findings:

```text
higher level HTTP client library
version: 0.13.4
rust-version: 1.85
stream feature exists
default includes default-tls/charset/http2/system-proxy, but workspace disables defaults and enables rustls
```

Docs/source findings:

```text
Response exposes status(), content_length(), error_for_status()
bytes_stream exists behind stream feature
ClientBuilder has timeout/connect_timeout related APIs
async implementation needs tokio/runtime policy
```

Quality implications:

```text
Do not implement reqwest first.
When implementing async, enable reqwest stream feature if using streaming body.
Reuse a reqwest::Client resource.
Keep sync/async traits separate; do not hide tokio runtime creation in library code.
```

#### url 2.5

Findings:

```text
Url::parse parses absolute URLs
Url::as_str is fast and returns stored serialization
```

Quality implications:

```text
Define RemoteUrl/HttpUrl as a typed wrapper around url::Url.
Reject non-http/non-https schemes at construction.
Avoid accepting raw String directly in AcquireNode.
```

#### tiny_http 0.12

Findings:

```text
small local HTTP server crate
no async runtime required
can construct responses/status/headers
```

Quality implications:

```text
Good dev-dependency for sync ureq tests.
Use local loopback server for deterministic tests.
Avoid external network in test suite.
```

#### HTTP status semantics

MDN summary confirms:

```text
2xx = success class
3xx = redirect class
4xx = client error
5xx = server error
```

Quality implications:

```text
First sync Acquire should accept only successful responses.
Redirect behavior can use client default initially; do not implement custom redirect loop logic in first slice.
Record final status seen by client where available.
```

## Net Acquire semantic contract

### Behavior role

`net Acquire` is an Acquire behavior. It is not Verify, Prepare, Apply, Store, or Install.

It consumes:

```text
Chosen<I, RemoteUrl>
```

It produces:

```text
Acquired<I, LocalMaterial, NetAcquireEvidence>
```

It must not:

```text
install into final target
extract archives
verify hashes implicitly as the default path
remember/persist global state
race mirrors or solve policy
```

It may:

```text
download remote bytes into a local staged/cache path
return LocalMaterial::File
record enough evidence for later Verify/Apply
```

### Type shape

Add feature-gated module:

```rust
#[cfg(feature = "net")]
pub mod net;
```

Core types:

```rust
pub struct RemoteUrl {
    pub url: url::Url,
}

pub struct NetAcquireNeed {
    pub destination: PathBuf,
    pub timeout: Option<Duration>,
    pub max_bytes: Option<u64>,
    pub headers: Vec<(String, String)>,
}

pub struct NetAcquireEvidence {
    pub url: url::Url,
    pub final_path: PathBuf,
    pub status: u16,
    pub bytes: u64,
    pub content_length: Option<u64>,
}

pub struct UreqAcquire<R = UreqResource> {
    pub resources: R,
}

pub struct UreqResource {
    pub agent: ureq::Agent,
}
```

Preferred shape for the first slice:

```rust
impl<I> AcquireNode<Chosen<I, RemoteUrl>> for UreqAcquire {
    type Material = LocalMaterial;
    type Evidence = NetAcquireEvidence;
    type Error = PulithError;
    type Output = Acquired<I, LocalMaterial, NetAcquireEvidence>;

    fn acquire_node(&self, node: Chosen<I, RemoteUrl>) -> Result<Self::Output, Self::Error>;
}
```

Open design point:

`AcquireNode` currently has no `Need` associated type. For LocalAcquire that is fine, but Net Acquire needs destination/cache/timeout/limits. Options:

```text
A. Put destination/timeout/limits in UreqAcquire resource/config.
B. Add a NetChosen wrapper that carries RemoteUrl + NetAcquireNeed.
C. Add a separate trait or extend AcquireNode with Need, which is a broader behavior change.
```

Recommendation for first slice:

```text
Use option B: Chosen<I, RemoteSource> where RemoteSource includes url + destination policy.
Do not alter core AcquireNode yet.
```

Concrete first-slice source type:

```rust
pub struct RemoteSource {
    pub url: RemoteUrl,
    pub destination: PathBuf,
    pub policy: NetAcquirePolicy,
}

pub struct NetAcquirePolicy {
    pub timeout: Option<Duration>,
    pub max_bytes: Option<u64>,
    pub headers: Vec<(String, String)>,
}
```

Then:

```text
Chosen<I, RemoteSource> -> Acquired<I, LocalMaterial, NetAcquireEvidence>
```

This avoids modifying the trait system while keeping request facts typed.

## Concrete execution slices

### Slice 0 — compile-only skeleton

Files:

```text
crates/pulith/src/net.rs
crates/pulith/src/lib.rs
crates/pulith/src/error.rs
crates/pulith/Cargo.toml
Cargo.toml
```

Tasks:

```text
1. Add url optional dependency to pulith crate if not already wired.
2. Make net feature include dep:url.
3. Add #[cfg(feature = "net")] pub mod net;
4. Export RemoteUrl, RemoteSource, NetAcquirePolicy, NetAcquireEvidence.
5. Add minimal PulithError variants for URL/network/status/limit.
6. Add no network behavior yet; compile feature matrix.
```

Acceptance:

```bash
cargo check -p pulith --features "sync local net ureq"
cargo check -p pulith --features "async net reqwest"
```

### Slice 1 — RemoteUrl / RemoteSource contract

Tasks:

```text
1. RemoteUrl::parse(input) -> Result<Self, PulithError>
2. Accept only http and https schemes.
3. Preserve url::Url in evidence, not raw strings.
4. RemoteSource::new(url, destination) and policy builder helpers.
5. Reject destination paths with no parent.
```

Tests:

```text
remote_url_accepts_http_https
remote_url_rejects_file_ftp_relative
remote_source_preserves_destination_and_default_policy
```

### Slice 2 — sync ureq downloader core

Tasks:

```text
1. UreqAcquire owns/reuses ureq::Agent.
2. Configure agent timeout from policy or resource default.
3. Perform GET request.
4. Accept only successful response.
5. Stream body to NamedTempFile in destination parent.
6. Enforce max_bytes while reading.
7. Count bytes written.
8. Persist temp file using CreateOrReplace semantics for acquired cache/destination path.
9. Return LocalMaterial { path, kind: File }.
10. Return NetAcquireEvidence { url, final_path, status, bytes, content_length }.
```

Implementation details:

```text
Use response.body_mut().as_reader() or equivalent reader API.
Use std::io::copy-like loop with explicit buffer so max_bytes can be checked before writing excess.
Use tempfile::NamedTempFile::new_in(destination_parent).
Do not write directly to destination.
Do not use old pulith-fs workspace.
```

Tests with local server:

```text
ureq_acquire_downloads_file_to_local_material
ureq_acquire_records_status_bytes_content_length
ureq_acquire_rejects_non_success_status
ureq_acquire_enforces_max_bytes_before_persist
ureq_acquire_does_not_touch_existing_destination_on_failed_download
```

Test server:

```text
tiny_http as dev-dependency or std::net minimal HTTP server
```

Recommendation:

```text
Use tiny_http for clarity, unless dependency minimalism requires a std::net server.
```

### Slice 3 — composition tests with hash/local apply

Tasks:

```text
1. Net Acquire downloads to a cache/temp destination.
2. HashVerify consumes Acquired<I, LocalMaterial, NetAcquireEvidence>.
3. IdentityPrepare then LocalApply consumes verified material.
```

Tests:

```text
net_acquire_flows_into_hash_verify
net_acquire_flows_into_local_apply_after_verify
net_acquire_failed_status_does_not_create_apply_material
```

Important:

```text
Hash remains a Verify step, not embedded in Acquire.
Apply remains a LocalApply step, not embedded in Acquire.
```

### Slice 4 — error quality and retry decision

First implementation should not add retry yet unless test pressure requires it.

Reason:

```text
Retry changes observable behavior and can hide test failures.
Old retry policy exists but belongs to a later NetAcquirePolicy extension.
```

Minimal first error variants:

```rust
InvalidUrl(String)
UnsupportedUrlScheme(String)
NetworkError(String)
HttpStatus { status: u16, url: String }
DownloadLimitExceeded { max: u64, actual: u64 }
DownloadIo { path: PathBuf, source: io::Error } // or existing PulithError::io
```

Later retry shape:

```rust
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_backoff: Duration,
}
```

Do not implement until non-retry behavior is green.

### Slice 5 — async reqwest mirror

Prerequisite:

```text
sync ureq contract stable
```

Tasks:

```text
1. Ensure reqwest feature includes stream if bytes_stream is used.
2. Define ReqwestAcquire with reqwest::Client resource.
3. Implement AsyncAcquireNode<Chosen<I, RemoteSource>>.
4. Do not spawn or create a runtime inside library code.
5. Stream chunks into tokio or blocking file carefully.
6. Mirror evidence and error semantics from ureq.
```

Tests:

```text
reqwest_acquire_downloads_file_to_local_material
reqwest_acquire_rejects_non_success_status
reqwest_acquire_enforces_max_bytes
```

Open implementation choice:

```text
Use tokio::fs::File + AsyncWriteExt if async file writing is desired.
Or use spawn_blocking for sync tempfile writes.
```

Recommendation:

```text
Keep async slice separate; do not contaminate sync ureq slice with tokio.
```

### Slice 6 — object_store deferred

Do not implement now.

Reason:

```text
object_store introduces backend-specific path semantics and auth/config concerns.
It is not the same as plain URL download.
```

Reopen when:

```text
S3/GCS/Azure/local object source becomes a real Pulith source type.
```

## Code quality constraints

### Typed behavior constraints

```text
No App/Context monolith.
No old Fetcher/FetchReceipt choreography.
No pulith-fetch compatibility module.
No public Workspace/Transaction replacement.
No hidden global HTTP client singleton.
```

### File placement constraints

```text
Always write to same-parent NamedTempFile.
Persist only after full successful response body read.
On status/limit/network error, do not touch final destination.
Final path material must be a regular file.
```

### Network constraints

```text
Accept only http/https.
Use ureq Agent for connection pooling/reuse.
Use timeout_global or request/resource timeout.
Treat non-2xx as error for first slice.
Record status and byte count.
Do not auto-decompress unless feature choice explicitly wants it.
Avoid external network in tests.
```

### Evidence constraints

Evidence must answer:

```text
which URL was used?
where are the bytes now?
what status was observed?
how many bytes were written?
was Content-Length known?
```

Evidence should not try to answer:

```text
whether bytes are trusted by digest
whether archive contents are safe
whether artifact should be installed
```

Those belong to Verify/Prepare/Apply.

## Verification plan for implementation

Each implementation pass must end with a fresh ad-hoc script under:

```text
F:\Stratum\TEMP\hermes-verify-*.py
```

Initial sync ureq verification should run:

```text
cargo fmt --all --check
cargo check -p pulith --no-default-features
cargo check -p pulith --features "sync local net ureq"
cargo test -p pulith --features "sync local net ureq" net::tests::
cargo check --workspace --all-features
cargo test --workspace --all-features
git diff --check -- changed paths
```

Structural markers should include:

```text
url::Url
tempfile::NamedTempFile::new_in
ureq::Agent
body_mut
NetAcquireEvidence
LocalMaterial
MaterialKind::File
```

## Final implementation order recommendation

Execute next as:

```text
1. Slice 0 + Slice 1 together: net module skeleton + typed RemoteUrl/RemoteSource contract.
2. Slice 2: sync ureq downloader with local test server.
3. Slice 3: composition tests with HashVerify and LocalApply.
4. Stop and report.
5. Only then consider retry policy.
6. Only after sync path stabilizes, implement async reqwest.
```

This keeps the next coding step small, testable, and aligned with the existing typed tree.
