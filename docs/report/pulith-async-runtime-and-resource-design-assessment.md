# Pulith Async Runtime and Resource-Control Assessment

## Status

This report responds to three design questions before implementing the next async `net Acquire` slice:

```text
1. Besides Tokio, what async runtimes/backends are plausible?
2. Can the async file-handle/persist risk be avoided by design rather than patched after failure?
3. How should Pulith control resources so shared resources stay shared and exclusive resources stay scoped to one operation?
```

No production code is changed in this pass.

## Sources searched/read

Commands:

```bash
cargo info --registry crates-io tokio
cargo info --registry crates-io async-std
cargo info --registry crates-io smol
cargo info --registry crates-io glommio
cargo info --registry crates-io monoio
cargo info --registry crates-io compio
cargo info --registry crates-io reqwest
cargo info --registry crates-io hyper
cargo info --registry crates-io isahc
cargo info --registry crates-io surf
```

Docs/source snippets searched:

```text
tokio docs
async-std docs
smol docs
glommio docs
monoio docs
compio docs
reqwest Client docs/Cargo.toml/source
hyper docs
isahc docs
surf docs
```

## Runtime/backend findings

### Tokio

Findings:

```text
Mature event-driven runtime for network applications.
Feature-gated modules include fs, io-util, net, time, rt, rt-multi-thread.
reqwest depends on Tokio/hyper stack for native async networking.
Reqwest rustls path pulls tokio-rustls/hyper-rustls.
```

Assessment:

```text
Best default for reqwest backend.
Not runtime-agnostic.
Very stable ecosystem and already effectively required by reqwest.
```

Implication for Pulith:

```text
If the backend is reqwest, call it a Tokio-backed reqwest backend explicitly.
Do not pretend it is runtime-neutral.
Feature name `reqwest` is acceptable because it names backend capability, but docs should state it is Tokio-backed.
```

### async-std

Findings:

```text
cargo info says async-std is deprecated in favor of smol.
Docs say async-std has been discontinued; use smol instead.
It provides fs/net/task/time abstractions and tokio compatibility features.
```

Assessment:

```text
Do not choose async-std for a new Pulith backend.
```

Reason:

```text
Deprecated/discontinued signal is enough to reject it unless a user specifically requests async-std compatibility.
```

### smol

Findings:

```text
Small async runtime built by composing smaller crates.
Docs mention tokio-based libraries can be used with async-compat adapters.
Provides async fs/net/executor pieces through smol ecosystem.
```

Assessment:

```text
Plausible runtime-agnostic/small-runtime path, but not a good match for reqwest directly.
```

Implication:

```text
If Pulith wants a smol-native backend, use a smol-compatible HTTP client stack such as surf/h1/http-client, not reqwest.
Treat this as a separate backend family, not an implementation detail under `reqwest`.
```

### glommio

Findings:

```text
Linux io_uring, thread-per-core runtime.
Docs emphasize pinned/thread-per-core scheduling and local executors.
```

Assessment:

```text
Not suitable for Pulith's default portable net Acquire backend.
```

Reason:

```text
Linux/io_uring/thread-per-core constraints conflict with current Windows host and cross-platform baseline.
```

### monoio

Findings:

```text
Thread-per-core runtime using io_uring/epoll/kqueue.
Has optional tokio compatibility features.
```

Assessment:

```text
Interesting for high-performance specialized network/file IO, not a baseline Pulith backend now.
```

Reason:

```text
Different IO traits and buffer ownership model would shape APIs; it should not be hidden behind the current reqwest plan.
```

### compio

Findings:

```text
Completion-based runtime using IOCP/io_uring/polling.
Cross-platform ambition, including Windows IOCP.
Provides own fs/net/runtime modules.
```

Assessment:

```text
Most interesting non-Tokio future candidate if Pulith later wants completion-based IO.
Not the next step.
```

Reason:

```text
It would be a distinct backend and likely a distinct material/staging design. It should not be introduced while net Acquire baseline is still young.
```

### hyper directly

Findings:

```text
Low-level HTTP library, not a convenient high-level client.
Docs recommend reqwest for convenient HTTP client use.
Hyper stack is Tokio-oriented in common native use.
```

Assessment:

```text
Do not use hyper directly for Pulith now.
```

Reason:

```text
It would increase implementation surface without adding behavior value over reqwest.
```

### isahc

Findings:

```text
HTTP client based on curl/libcurl model.
Docs describe sync and runtime-agnostic async API.
Supports timeouts, redirects, HTTP/2, cancellation on drop.
Rust version 1.85.
```

Assessment:

```text
Best non-Tokio async HTTP candidate if runtime neutrality is a hard requirement.
```

Tradeoff:

```text
Pros: runtime-agnostic async story; sync+async in one backend; mature curl behavior.
Cons: brings curl/static-curl surface; different TLS/cert/proxy behavior from reqwest/ureq rustls path; heavier/non-pure-Rust feel.
```

Recommendation:

```text
Do not replace reqwest with isahc now.
Keep `isahc` as a possible future backend feature if the user wants runtime-neutral async HTTP.
```

### surf

Findings:

```text
Async HTTP framework built around async-std by default with optional backends.
Docs mention async-std base and optional hyper client.
```

Assessment:

```text
Not recommended as the next Pulith backend.
```

Reason:

```text
It pulls in async-std-era design by default and abstracts HTTP backend behind another framework. Pulith already has its own behavior abstraction; adding surf would duplicate abstraction.
```

## Runtime recommendation

### Short answer

For the immediate `ReqwestAcquire` slice:

```text
Use Tokio, because reqwest is Tokio/hyper-based on native targets.
```

But the design should phrase this precisely:

```text
ReqwestAcquire is the Tokio-backed async HTTP backend.
```

Do not claim:

```text
Pulith async Acquire is runtime-agnostic.
```

### Backend matrix

Recommended backend posture:

```text
sync ureq        -> current default sync backend, pure Rust, blocking
async reqwest    -> next backend, Tokio-backed, high ecosystem fit
async isahc      -> future optional backend if runtime-neutral async becomes a requirement
smol/surf        -> defer; only if smol-native ecosystem matters
glommio/monoio   -> reject for default; specialized Linux/thread-per-core runtimes
compio           -> future research candidate for completion-based cross-platform IO
hyper direct     -> reject for now; lower-level than needed
async-std        -> reject; discontinued in favor of smol
```

## Design-level mitigation for async temp/persist risk

The earlier risk was:

```text
On Windows, persist/rename may fail if an async file handle is still open.
```

A patch-level mitigation would be:

```text
try flush/drop, if persist fails patch around it.
```

Better design:

```text
Make it impossible to call persist while the writer is open.
```

### Proposed typestate model

Introduce private staging states in `net.rs` or a private helper module:

```rust
struct DownloadStage<Open> { ... }
struct DownloadStage<Closed> { ... }
```

Only `DownloadStage<Closed>` has:

```rust
fn persist(self, destination: &Path) -> Result<PathBuf, PulithError>
```

Only `DownloadStage<Open>` has:

```rust
async fn write_chunk(&mut self, chunk: &[u8]) -> Result<(), PulithError>
async fn finish(self) -> Result<DownloadStage<Closed>, PulithError>
```

`finish(self)` consumes the open stage, flushes, drops the async writer, and returns the closed stage.

This prevents:

```text
persist while writer is still live
leaking writer handle across persist
ad-hoc drop ordering in backend code
```

### Better variant: backend-independent staged file boundary

The current `UreqAcquire` has direct `NamedTempFile` logic. The next async backend should not duplicate ad-hoc staging. Instead:

```text
Net backend owns network response.
Private StagedDownload owns temp file lifecycle.
Backend writes bytes through a narrow sink interface.
Only closed StagedDownload can persist.
```

Possible private API shape:

```rust
struct StagedDownload {
    temp: tempfile::NamedTempFile,
    bytes: u64,
}

struct SyncStageWriter<'a> { ... }
struct AsyncStageWriter { ... }
```

However, avoid overengineering. The minimal robust design for the reqwest slice:

```rust
let mut stage = AsyncDownloadStage::new_in(&parent)?;
while let Some(chunk) = response.chunk().await? {
    stage.write_chunk(&chunk, max_bytes).await?;
}
let closed = stage.finish().await?;
closed.persist(&destination)?;
```

`AsyncDownloadStage` is private and exists only to encode the ordering law.

### Alternative: acquire body, not file

A stronger semantic redesign would be:

```text
ReqwestAcquire -> Acquired<I, RemoteBody, NetAcquireEvidence>
RemoteBodyPrepare -> Prepared<I, LocalMaterial, ...>
LocalApply -> Applied
```

That would avoid mixing network streaming and file staging in Acquire. But it changes the current accepted contract and delays practical value.

Recommendation:

```text
Do not change public material shape now.
Keep Acquired<I, LocalMaterial, NetAcquireEvidence>.
Use a private typestate StagedDownload to enforce safe file lifecycle.
```

## Resource management model

Pulith should make a hard distinction:

```text
shared resources are reusable capabilities
exclusive resources are operation-owned capabilities
```

### Shared resources

Shared resources should live in backend resource structs and be borrowed/cloned cheaply:

```rust
pub struct UreqResource {
    pub agent: ureq::Agent,
}

pub struct ReqwestResource {
    pub client: reqwest::Client,
    pub budget: Option<Arc<NetBudget>>,
}
```

Shared candidates:

```text
HTTP client / connection pool
TLS/proxy/DNS config
network concurrency semaphore
retry sleep provider / clock
bandwidth limiter
temp quota / disk budget
metrics sink
```

Rules:

```text
No hidden global singleton.
No per-request client creation in hot path.
No resource ownership in RemoteSource; RemoteSource is request facts, not execution resource.
Clone handles only when the underlying resource is shared by design.
```

### Exclusive resources

Exclusive resources must be created per Acquire operation and consumed/dropped before the operation completes:

```text
response body stream
destination mutation right
temp file / staged download
temp writer handle
byte counter
evidence builder
retry attempt local state
```

Rules:

```text
Do not store exclusive state in ReqwestResource/UreqResource.
Do not share a staging file across operations.
Do not hold a target-path mutation permit after persist.
Do not carry open file handles into evidence.
```

### RAII budget/permit design

For future resource control, use RAII permits:

```rust
struct NetPermit { ... }       // one active network transfer slot
struct TempPermit { ... }      // temp byte/disk quota reservation
struct TargetPermit { ... }    // per-target mutation lock if later needed
```

Acquire flow:

```text
borrow shared resource
acquire shared-budget permit
create exclusive staging
stream/write
close exclusive staging
persist
release permits by drop
return evidence
```

This gives controlled sharing without turning every operation into global mutable state.

### Avoiding accidental exclusivity

Bad design:

```rust
struct ReqwestAcquire {
    client: reqwest::Client,
    current_destination: PathBuf,
    temp_file: Option<NamedTempFile>,
}
```

Problem:

```text
ReqwestAcquire becomes single-operation stateful and cannot be safely shared.
```

Good design:

```rust
struct ReqwestAcquire<R> { resources: R }
struct ReqwestResource { client: reqwest::Client, budget: Option<Arc<NetBudget>> }
```

Operation state stays in local variables/private typestate values.

## Revised next-step recommendation

The previous plan said “implement ReqwestAcquire next.” That remains true if the user accepts Tokio-backed async.

But after runtime research, the more precise plan is:

```text
1. Keep reqwest backend as Tokio-backed async backend.
2. Before implementation, add private StagedDownload typestate to avoid async handle/persist risk by construction.
3. Do not introduce runtime-neutral claims.
4. Defer isahc as a future runtime-neutral backend option.
5. Keep smol/compio/monoio/glommio out of the baseline until a concrete requirement appears.
```

## Concrete implementation plan update

### Slice 1 — staging design before reqwest

Add private helper:

```text
StagedDownload<Open>
StagedDownload<Closed>
```

Acceptance:

```text
Only Closed can persist.
Open finish consumes/drops writer before persist.
Unit test proves oversized write does not create final target.
```

### Slice 2 — reqwest resource

Add:

```text
ReqwestResource { client: reqwest::Client }
ReqwestAcquire<R = ReqwestResource>
```

No per-request client construction.

### Slice 3 — AsyncAcquireNode

Implement:

```text
AsyncAcquireNode<Chosen<I, RemoteSource>> for ReqwestAcquire<ReqwestResource>
```

Use:

```text
request.timeout(timeout)
response.status()
response.content_length()
response.chunk().await
StagedDownload typestate
```

### Slice 4 — tests

Local loopback server. No external network.

Required tests:

```text
reqwest_acquire_downloads_file_to_local_material
reqwest_acquire_rejects_non_success_status_without_touching_destination
reqwest_acquire_enforces_max_bytes_before_persist
reqwest_stage_cannot_persist_until_finished   # compile-time law via API shape plus behavior test
reqwest_acquire_flows_into_hash_verify
reqwest_acquire_flows_into_local_apply_after_verify
```

## Final recommendation

Proceed with async reqwest only if we explicitly accept:

```text
ReqwestAcquire is Tokio-backed.
```

If runtime neutrality is a hard requirement instead, switch the next research/implementation target to:

```text
IsahcAcquire
```

My recommendation for Pulith now:

```text
Implement Tokio-backed ReqwestAcquire next, but first factor private StagedDownload typestate so file-handle/persist correctness is enforced by design, not by patching failed Windows behavior.
```
