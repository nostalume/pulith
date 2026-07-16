# Pulith Net Acquire Next-Step Preparation

## Status

This is a planning/research pass after the completed sync `ureq` baseline.

No production code is changed in this pass. The next implementation should be chosen deliberately because `net Acquire` now has a working typed sync path:

```text
Chosen<I, RemoteSource>
  -> UreqAcquire
  -> Acquired<I, LocalMaterial, NetAcquireEvidence>
```

Current verified baseline:

```text
RemoteUrl / RemoteSource typed contract
sync UreqAcquire
same-parent staged file download
non-2xx rejection
max_bytes rejection before persist
HashVerify composition
LocalApply composition
```

## Current implementation assessment

Files reviewed:

```text
crates/pulith/src/net.rs
crates/pulith/Cargo.toml
docs/report/pulith-net-acquire-sync-ureq-execution-report.md
docs/report/pulith-net-acquire-execution-detail-plan.md
```

Current feature surface:

```toml
net = ["local", "dep:url"]
ureq = ["net", "sync", "dep:ureq"]
reqwest = ["net", "async", "dep:reqwest", "dep:tokio"]
object = ["net", "async", "dep:object_store"]
```

Current important code facts:

```text
RemoteUrl accepts only http/https.
NetAcquirePolicy currently has timeout/max_bytes/headers.
UreqAcquire uses request-level http_status_as_error(false).
UreqAcquire streams response.body_mut().as_reader() into NamedTempFile::new_in(parent).
UreqAcquire returns LocalMaterial::File and NetAcquireEvidence.
Tests use a local std::net HTTP server, no external network.
```

Known deliberate omissions:

```text
async reqwest backend
retry policy
range/resume
mirror/multi-source race
object_store backends
progress callbacks
bandwidth throttling
```

## Additional knowledge searched/read

Commands run:

```bash
cargo info --registry crates-io reqwest
cargo info --registry crates-io tokio
cargo info --registry crates-io futures-util
cargo info --registry crates-io httpmock
cargo info --registry crates-io wiremock
```

Docs/source searched:

```text
reqwest 0.13.4 source/docs for Response::status/content_length/chunk/bytes_stream/error_for_status and RequestBuilder::timeout/send
tokio 1.52.3 source/docs for fs::File and AsyncWriteExt
futures-util StreamExt docs
MDN Range header docs
MDN Retry-After header docs
httpmock / wiremock crate metadata
```

### reqwest findings

`cargo info reqwest`:

```text
version: 0.13.4
rust-version: 1.85.0
stream feature = [tokio/fs, dep:futures-util, dep:tokio-util, dep:wasm-streams]
rustls feature available
workspace currently disables defaults and enables rustls
```

Source/docs findings:

```text
Response::status() is available.
Response::content_length() is available.
Response::chunk().await is available without requiring the stream feature.
Response::bytes_stream() exists but requires the stream feature and futures-util.
RequestBuilder::timeout(Duration) applies request-level timeout.
RequestBuilder::send().await returns Response.
Response::error_for_status() exists, but using it would hide status mapping/evidence.
```

Quality implication:

```text
For the first async backend, prefer Response::chunk().await instead of bytes_stream().
This avoids adding futures-util or reqwest stream feature immediately.
Keep manual status check so Pulith owns HttpStatus evidence/error mapping.
Use request.timeout(policy.timeout) to mirror ureq policy semantics.
```

### tokio findings

`cargo info tokio`:

```text
fs feature exposes tokio::fs::File.
io-util exposes AsyncWriteExt.
net/time are not in the current workspace feature set unless explicitly enabled elsewhere.
```

Docs/source findings:

```text
tokio::fs::File supports async write through AsyncWriteExt::write_all and flush.
tokio fs uses blocking operations behind the scenes and requires the fs feature.
Async file writes should be flushed before file handle drop/persist.
```

Quality implication:

```text
If implementing ReqwestAcquire with async file writes, make tokio workspace features explicit:
rt-multi-thread, fs, io-util, net, time.
Use tokio::fs::File::from_std(temp.reopen()?) or an equivalent temp-path strategy.
Drop/flush async file handle before NamedTempFile::persist.
Do not create a tokio runtime inside the library.
```

### HTTP Range findings

MDN Range docs:

```text
Range can request partial resource bytes.
206 Partial Content means range was served.
416 Range Not Satisfiable means invalid range.
A server may ignore Range and return 200 with the full resource.
```

Quality implication:

```text
Range/resume cannot be a small flag on current Acquire.
It needs explicit resume state, expected existing partial size, and handling of 200-vs-206 semantics.
Defer until the plain async backend and retry semantics are stable.
```

### Retry-After findings

MDN Retry-After docs establish use around throttling/unavailable statuses such as:

```text
429 Too Many Requests
503 Service Unavailable
```

Quality implication:

```text
Retry policy should eventually parse Retry-After, but the first retry slice should be conservative.
Start with transport errors and selected 5xx/429 statuses, bounded attempts, and test-injected sleep provider.
Do not sleep in tests.
```

### HTTP mock crate findings

`httpmock`:

```text
rich HTTP mock library
rust-version: 1.88.0
```

`wiremock`:

```text
async HTTP mocking library
common for reqwest tests
```

Quality implication:

```text
Do not add httpmock now because its rust-version is 1.88 while Pulith currently compiles with Rust 1.85-era dependencies.
wiremock may be useful later, but the current std::net local server is sufficient and keeps dependencies low.
Continue using local std::net loopback server for next tests unless async test setup becomes too noisy.
```

## Next-step recommendation

Recommended next implementation:

```text
Async ReqwestAcquire parity slice
```

Reason:

```text
1. The sync ureq baseline is already green and typed.
2. The feature surface already reserves reqwest as the async backend.
3. Implementing reqwest now proves the sync/async trait split before adding retry/resume complexity.
4. Retry should be backend-common; doing it after both backends exist avoids encoding ureq-only behavior.
5. Range/resume requires more state and should wait.
```

Do not implement `object_store` next.

Reason:

```text
object_store has backend-specific path/auth semantics and is not plain HTTP URL Acquire.
It should only happen after URL Acquire semantics are stable.
```

## Concrete next implementation plan

### Slice A — Cargo feature cleanup for async backend

Files:

```text
Cargo.toml
crates/pulith/Cargo.toml
```

Tasks:

```text
1. Ensure workspace tokio features explicitly include rt-multi-thread, fs, io-util, net, time.
2. Keep reqwest default-features=false + rustls.
3. Do not enable reqwest stream yet if using Response::chunk().await.
4. Do not add futures-util for the first async slice.
```

Potential `Cargo.toml` direction:

```toml
tokio = { version = "1", features = ["rt-multi-thread", "fs", "io-util", "net", "time"] }
reqwest = { version = "0.13", default-features = false, features = ["rustls"] }
```

Acceptance:

```bash
cargo check -p pulith --features "async net reqwest"
```

### Slice B — Add async resource and marker types

File:

```text
crates/pulith/src/net.rs
```

Types:

```rust
#[cfg(feature = "reqwest")]
pub struct ReqwestResource {
    pub client: reqwest::Client,
}

#[cfg(feature = "reqwest")]
pub struct ReqwestAcquire<R = ReqwestResource> {
    pub resources: R,
}
```

Exports:

```rust
#[cfg(feature = "reqwest")]
pub use net::{ReqwestAcquire, ReqwestResource};
```

Rules:

```text
Reuse reqwest::Client.
No runtime creation inside library code.
No compatibility with old pulith-fetch Fetcher.
```

### Slice C — Implement AsyncAcquireNode parity

Trait target:

```rust
impl<I> AsyncAcquireNode<Chosen<I, RemoteSource>> for ReqwestAcquire<ReqwestResource>
```

Suggested associated future shape:

```rust
type Future<'a> = Pin<Box<dyn Future<Output = Result<Self::Output, Self::Error>> + 'a>>
where
    Self: 'a,
    Chosen<I, RemoteSource>: 'a;
```

Implementation flow:

```text
1. Move Chosen<I, RemoteSource> into async block.
2. Resolve destination parent with existing helper or shared helper.
3. create_dir_all destination parent using std or tokio; keep path errors explicit.
4. reject existing destination symlink/non-file using existing sync metadata helper.
5. Build reqwest GET request from source.url.as_str().
6. Add headers from NetAcquirePolicy.
7. Apply request.timeout(timeout) if set.
8. send().await.
9. Capture status and content_length.
10. Reject non-success status before temp file persist.
11. Create same-parent NamedTempFile.
12. Reopen temp as std::fs::File and convert to tokio::fs::File::from_std, or use a carefully scoped blocking write path.
13. Loop over response.chunk().await.
14. Enforce max_bytes before writing excessive chunk.
15. write_all chunk and accumulate bytes.
16. flush and drop async file handle.
17. persist NamedTempFile to destination.
18. Return Acquired<I, LocalMaterial, NetAcquireEvidence>.
```

Open implementation detail:

```text
The NamedTempFile + tokio::fs::File handle must be scoped so Windows can persist/rename after the async file handle is flushed and dropped.
```

Recommended pattern:

```rust
let mut temp = tempfile::NamedTempFile::new_in(&parent)?;
{
    let std_file = temp.reopen()?;
    let mut file = tokio::fs::File::from_std(std_file);
    // write chunks
    file.flush().await?;
}
temp.persist(&destination)?;
```

If `reopen()` + persist has Windows handle issues, fallback:

```text
Use a temp path from NamedTempFile, write through std file inside spawn_blocking batches, then persist after all handles are dropped.
Do not silently switch to direct final writes.
```

### Slice D — Async tests with local loopback server

Do not add external mock dependencies yet.

Test runtime options:

```text
Use #[test] plus tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(...)
```

This avoids needing `tokio/macros`.

Minimum async tests:

```text
reqwest_acquire_downloads_file_to_local_material
reqwest_acquire_rejects_non_success_status_without_touching_destination
reqwest_acquire_enforces_max_bytes_before_persist
reqwest_acquire_flows_into_hash_verify
reqwest_acquire_flows_into_local_apply_after_verify
```

Test server:

```text
Reuse the current std::net serve_once helper if possible.
It already works for reqwest because reqwest can talk to loopback HTTP.
```

### Slice E — Shared helper cleanup, but only if needed

Possible helper extraction:

```text
destination_parent
reject_existing_unsafe_destination
build evidence
```

Constraint:

```text
Do not over-abstract into Client trait/factory/registry.
Only share tiny file/path helpers if duplication appears in reqwest implementation.
```

## Risk analysis

### Risk 1 — Tokio feature gaps

Symptoms:

```text
tokio::fs missing
tokio::io::AsyncWriteExt missing
runtime has no net/time driver
```

Mitigation:

```text
Explicit workspace tokio features: rt-multi-thread, fs, io-util, net, time.
Use regular tests with runtime builder rather than tokio macros.
```

### Risk 2 — Reqwest body API choice

Options:

```text
Response::chunk().await: simpler, no reqwest stream feature/futures-util.
Response::bytes_stream(): more stream-generic, needs reqwest stream + futures-util.
```

Decision:

```text
Use chunk() first.
```

### Risk 3 — Windows temp persist with async file handle

Symptoms:

```text
persist/rename fails because async file handle still open.
```

Mitigation:

```text
Scope async file handle tightly.
Flush and drop before persist.
Run tests on current Windows host.
```

### Risk 4 — Error parity with ureq

Risk:

```text
Reqwest errors may differ from ureq errors.
```

Mitigation:

```text
Map backend transport errors to PulithError::NetworkError.
Map HTTP status manually to PulithError::HttpStatus.
Keep evidence identical.
```

### Risk 5 — Accidental old fetch reintroduction

Forbidden:

```text
Fetcher
FetchReceipt
progress phase API
resume checkpoint API
multi-source planner/race
Workspace/Transaction
```

Mitigation:

```text
Structural grep markers in ad-hoc verification should ensure new code exposes ReqwestAcquire/ReqwestResource only and does not reference old glue names.
```

## Verification plan for next implementation

Fresh ad-hoc script under:

```text
F:\Stratum\TEMP\hermes-verify-*.py
```

Commands:

```text
cargo fmt --all --check
cargo check -p pulith --no-default-features
cargo check -p pulith --features "sync local net ureq"
cargo check -p pulith --features "async net reqwest"
cargo test -p pulith --features "async net reqwest hash blake3" net::tests::reqwest
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::
cargo test -p pulith --features "async net reqwest hash blake3" net::tests::
cargo check --workspace --all-features
cargo test --workspace --all-features
git diff --check -- changed paths
```

Structural markers:

```text
pub struct ReqwestResource
pub struct ReqwestAcquire
impl<I> AsyncAcquireNode<Chosen<I, RemoteSource>> for ReqwestAcquire<ReqwestResource>
reqwest::Client
request.timeout(timeout)
response.chunk().await
tokio::fs::File::from_std
tokio::io::AsyncWriteExt
NetAcquireEvidence
MaterialKind::File
reqwest_acquire_downloads_file_to_local_material
reqwest_acquire_flows_into_hash_verify
reqwest_acquire_flows_into_local_apply_after_verify
```

## Recommended execution order

```text
1. Add tokio feature readiness if needed.
2. Add ReqwestResource / ReqwestAcquire exports.
3. Implement AsyncAcquireNode using response.chunk().await.
4. Add local loopback async tests.
5. Verify feature matrix and workspace all-features.
6. Report.
```

## Deferred after async reqwest

After async reqwest passes, the next likely sequence is:

```text
1. Backend-common RetryPolicy with test-injected sleep provider.
2. Header/status evidence extensions if needed.
3. Range/resume design with explicit 200/206/416 semantics.
4. Multi-source/mirror selection as Select/Offer behavior, not inside Acquire.
5. object_store only when non-HTTP object source semantics are required.
```
