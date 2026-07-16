# Pulith Crate Prune, Net Acquire, and File I/O Assessment

## Scope

User request:

```text
1. Delete crates whose migrated modules are now implemented in `crates/pulith`.
2. Evaluate net Acquire libraries for quality/performance and prefer a high-quality, high-performance design.
3. Analyze current file interaction behavior quality/performance.
```

## Deletion performed

Deleted:

```text
crates/pulith-archive
```

Reason:

```text
Archive Prepare has been migrated into `crates/pulith/src/archive.rs`:
- Zip Prepare
- Tar<Plain> Prepare
- Tar<Gzip> Prepare
- Tar<Xz> Prepare
- Tar<Zstd> Prepare
- generic ArchiveTree<A> -> LocalApply<_>
```

I did not delete these crates yet:

```text
crates/pulith-fetch
crates/pulith-fs
crates/pulith-resource
crates/pulith-source
crates/pulith-store
crates/pulith-state
crates/pulith-install
crates/pulith-version
```

Reason:

```text
Their behavior has not yet been completely migrated into the new typed `pulith` crate.
Deleting them now would discard still-useful design/test/performance reference code before net Acquire, persistent Remember, and high-quality file Apply are migrated.
```

Current workspace membership already contains only:

```toml
members = ["crates/pulith"]
```

So deleting `pulith-archive` does not change active workspace build membership. It removes the stale migrated crate from the repository tree.

## Net Acquire library evaluation

Inspected current workspace dependency candidates and crates.io metadata:

```text
reqwest 0.13.4
ureq 3.3.0
object_store 0.14.0
```

Current `crates/pulith/Cargo.toml` features:

```toml
net = []
reqwest = ["async", "dep:reqwest", "dep:tokio"]
ureq = ["sync", "dep:ureq"]
object = ["async", "dep:object_store"]
```

### reqwest

Observed crates.io metadata:

```text
reqwest 0.13.4
license MIT OR Apache-2.0
rust-version 1.85.0
higher-level HTTP client
features include stream, rustls, http2, blocking, gzip/deflate/brotli/zstd, http3
```

Quality/performance assessment:

```text
Best async HTTP default.
High ecosystem quality.
Connection pooling and streaming are mature.
Good fit for AsyncAcquireNode.
Supports streaming bodies without buffering whole downloads.
Good long-term backend for concurrent downloads and future retry/rate policy.
```

Design caveats:

```text
Do not expose reqwest request/response types in Pulith public API.
Do not create a universal HttpClient trait unless runtime backend swapping becomes real.
Reuse one reqwest::Client as a shared resource; do not create a new client per request.
Use rustls/default configured feature intentionally.
Keep decompression policy explicit; avoid accidental transparent decompression when byte-for-byte hashes/content length must match wire bytes.
```

Recommendation:

```text
Use `reqwest` for async URL Acquire.
Implement `ReqwestAcquire<R>` with a resource struct holding reqwest::Client, temp root, and policy.
```

### ureq

Observed crates.io metadata:

```text
ureq 3.3.0
license MIT OR Apache-2.0
rust-version 1.85
simple safe HTTP client
features include rustls, gzip, native-tls, socks-proxy
```

Quality/performance assessment:

```text
Good sync HTTP backend.
Small/simple API compared with reqwest async stack.
Appropriate for `sync` feature where callers do not want Tokio.
Should stream response into a file; do not read whole response into memory.
```

Design caveats:

```text
The current workspace config uses `default-features = false, features = ["rustls"]`, which avoids default gzip.
That is good for exact-byte Acquire because transparent decompression can break content-length/checksum semantics.
Need explicit timeout/max-bytes policy.
Need careful status-code handling and partial-file cleanup.
```

Recommendation:

```text
Use `ureq` for sync URL Acquire.
Implement it first because the current typed tree has sync node paths and the slice can stay small.
```

### object_store

Observed crates.io metadata:

```text
object_store 0.14.0
license MIT/Apache-2.0
rust-version 1.85
generic object store interface for S3/GCS/Azure/local files
```

Quality/performance assessment:

```text
High-quality abstraction for cloud object storage.
Potentially excellent for remote object Acquire where HTTP URL semantics are insufficient.
Not the right first net Acquire slice because it brings a broader semantic model: paths, stores, credentials, range/get options, cloud backend config.
```

Recommendation:

```text
Defer object_store until URL Acquire is typed and stable.
Add it as `ObjectAcquire` only when Pulith has a real object-store source semantic, not as a generic URL downloader replacement.
```

## Proposed high-quality/high-performance Net Acquire design

### Semantic shape

Add a typed source and backend markers:

```rust
pub struct UrlSource { pub url: String }
pub struct Ureq;
pub struct Reqwest;
```

Sync path:

```text
Chosen<I, UrlSource>
  -> Acquired<I, LocalMaterial, NetEvidence<Ureq>>
```

Async path:

```text
Chosen<I, UrlSource>
  -> Acquired<I, LocalMaterial, NetEvidence<Reqwest>>
```

Do not migrate old public names as the main caller vocabulary:

```text
FetchSource
Fetcher
FetchReceipt
BatchFetcher
MultiSourceFetcher
SegmentedFetcher
ConditionalFetcher
```

Those are mechanisms or later specialized behaviors, not first-slice public semantics.

### Resource/policy ownership

Use explicit resources:

```rust
NetAcquire<B, R> { resources: R, _backend: PhantomData<B> }

SyncNetResources {
    temp_root: PathBuf,
    agent: ureq::Agent,
}

AsyncNetResources {
    temp_root: PathBuf,
    client: reqwest::Client,
}
```

Use associated Need:

```rust
NetNeed {
    max_bytes: Option<u64>,
    timeout: Option<Duration>,
    headers: Vec<(String, String)>,
    expected_status: StatusPolicy,
}
```

Evidence:

```rust
NetEvidence<B> {
    source: String,
    path: PathBuf,
    bytes: u64,
    status: u16,
    content_length: Option<u64>,
    etag: Option<String>,
    last_modified: Option<String>,
}
```

### Performance requirements

The first implementation should:

```text
stream response directly to a temp file
avoid full body allocation
count bytes during streaming
enforce max_bytes before exceeding resource policy
reuse client/agent resources
write to a unique temp path, then expose LocalMaterial
avoid transparent decompression unless explicitly requested
```

Avoid initially:

```text
segmented downloads
resume/checkpoint state
batch/multi-source orchestration
conditional caching
progress subsystem
rate limiter
runtime HttpClient trait object
```

Reason:

```text
Those are useful but not the minimal typed Acquire behavior. They should be added as separate typed capabilities after the basic Acquire law is stable.
```

### First implementation slice recommendation

```text
1. Add `net.rs` module gated by `net`.
2. Add `UrlSource`, `NetNeed`, `NetEvidence<B>`, `NetAcquire<B, R>`.
3. Implement sync `NetAcquire<Ureq, SyncNetResources>` for `AcquireNode<Chosen<I, UrlSource>>`.
4. Stream into a temp file under a caller-provided temp root.
5. Return `LocalMaterial { path, kind: File }`.
6. Tests use a local TCP HTTP fixture or tiny test server, not live network.
7. Then implement async `NetAcquire<Reqwest, AsyncNetResources>` with the same evidence semantics.
```

## Current file interaction behavior analysis

### Current active `crates/pulith/src/local.rs`

Current behavior:

```text
LocalAcquire checks path existence and classifies File/Directory.
IdentityPrepare passes through LocalMaterial.
LocalApply<Create/Replace/CreateOrReplace/Forget> mutates target directly.
copy_prepared uses std::fs::copy for files and recursive copy_dir_all for directories.
remove_existing uses remove_dir_all/remove_file before replacement.
MemoryRemember is in-memory evidence only.
```

Quality assessment:

```text
Good typed behavior shape.
Small and readable.
No App/Context monolith.
Works for executable local happy path tests.
But filesystem safety/performance is currently baseline, not production-grade.
```

Main correctness gaps:

```text
CreateOrReplace and Replace remove the destination before successful copy, so failure can leave target absent or partial.
File copy is not atomic: target file may be partially written if copy fails.
Directory copy is not transactional.
No staging directory resource.
No rollback/backup receipt.
No symlink policy on local directory copy.
No same-file/source-under-target/target-under-source guards.
No permission preservation policy.
No fsync/durability policy.
No hardlink-or-copy optimization.
No temp-file unique path strategy.
```

Main performance gaps:

```text
Directory copy is serial recursive std::fs copy.
No hardlink fast path for large local files.
No copy-only threshold or size hint.
No workspace/report reuse from old pulith-fs.
No adaptive behavior for same-device vs cross-device placement.
No streaming buffer tuning beyond std::fs::copy defaults.
```

### Old `pulith-fs` quality/performance reference

`pulith-fs` contains useful implementation ideas:

```text
Workspace staging root
path sanitization for relative workspace paths
atomic_write / atomic_read primitives
hardlink_or_copy with cross-device fallback
replace_dir with Windows retry behavior
stage_file_by_size / stage_file_with_size_hint
WorkspaceReport counting files/directories/bytes
```

High-value pieces to migrate into `crates/pulith` before deleting `pulith-fs`:

```text
1. Workspace-like staging resource as private Apply resource.
2. hardlink-or-copy fast path for local file Apply.
3. Atomic file write/rename for file targets.
4. Replace directory via staging + rename, not remove-then-copy.
5. Relative path sanitizer for staging internals.
6. Windows retry behavior around remove/rename if keeping Windows support strong.
```

What not to migrate as public choreography:

```text
Workspace as a required caller protocol unless the typed Apply resource truly needs it.
Transaction as a universal workflow object.
WorkspaceReport as top-level result bag detached from ApplyEvidence.
```

## File Apply redesign recommendation

Before deleting `pulith-fs`, implement one focused local Apply hardening slice:

```text
LocalApply<O, R = DirectFs>
```

Resource:

```rust
LocalFsResources {
    staging_root: PathBuf,
    hardlink_threshold_bytes: u64,
    overwrite: OverwritePolicy,
}
```

Behavior:

```text
Prepared<Intent<Item, LocalTarget, Create>, LocalPrepared, E>
  -> staged copy/link -> atomic rename when possible -> Applied<...>
```

Evidence should record:

```text
target path
bytes copied/linked if known
files/directories count if directory
placement strategy: copy | hardlink | staged_rename
```

Stop conditions:

```text
Create never overwrites.
Replace does not remove target until staged replacement is ready.
CreateOrReplace is atomic at file granularity and best-effort atomic at directory granularity.
Directory copy rejects or explicitly handles symlinks.
Feature matrix and failure-path tests pass.
```

## Deletion plan for remaining old crates

Do not delete all old crates blindly. Delete only when the behavior is migrated and structurally guarded:

```text
pulith-archive — deleted now; archive Prepare migrated.
pulith-fs — delete after LocalApply hardening migrates staging/atomic/hardlink behavior.
pulith-fetch — delete after URL Acquire sync+async migrate enough network behavior.
pulith-source/resource/version — delete or fold after source/resource/version semantics are represented in `application.rs` or a small typed source module.
pulith-store/state/install — delete after persistent Remember / Inspect / Repair / lifecycle Apply semantics migrate.
```

## Recommended next execution order

High-quality/high-performance path:

```text
1. Harden LocalApply/file interaction using selected pulith-fs ideas.
2. Delete pulith-fs after verification.
3. Implement sync URL Acquire with ureq, streaming to temp file.
4. Implement async URL Acquire with reqwest, reusing reqwest::Client.
5. Delete pulith-fetch after net Acquire verification.
```

Reason:

```text
Net Acquire needs a high-quality local file landing/staging boundary. Strengthening LocalApply/local file staging first avoids designing net downloads around weak direct writes.
```
