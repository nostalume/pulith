# Pulith Tokio-backed ReqwestAcquire Execution Report

## Status

Completed the async `net Acquire` slice as a Tokio-backed reqwest backend.

The implementation is deliberately named by backend/runtime facts:

```text
net behavior: Acquire RemoteSource into LocalMaterial
async runtime family: runtime-tokio feature
HTTP backend: reqwest feature
```

The code does not claim runtime neutrality for reqwest.

## Implemented path

```text
Chosen<I, RemoteSource>
  -> ReqwestAcquire<ReqwestResource>
  -> Acquired<I, LocalMaterial, NetAcquireEvidence>
```

`ReqwestAcquire` implements:

```rust
AsyncAcquireNode<Chosen<I, RemoteSource>>
```

with output:

```text
LocalMaterial::File
NetAcquireEvidence { url, final_path, status, bytes, content_length }
```

## Cargo feature design

Updated `crates/pulith/Cargo.toml`:

```toml
sync = []
async = []
runtime-tokio = ["async", "dep:tokio"]
local = ["dep:same-file", "dep:tempfile", "dep:walkdir"]
net = ["local", "dep:url"]
reqwest = ["net", "runtime-tokio", "dep:reqwest"]
ureq = ["net", "sync", "dep:ureq"]
object = ["net", "async", "dep:object_store"]
```

This keeps axes separate:

```text
`net` names behavior family.
`async` names modality.
`runtime-tokio` names runtime capability.
`reqwest` names HTTP backend and depends on runtime-tokio.
`ureq` names sync HTTP backend and depends on sync.
```

Updated workspace `tokio` dependency features:

```toml
tokio = { version = "1", features = ["fs", "io-util", "net", "rt", "rt-multi-thread", "time"] }
```

Reason:

```text
fs/io-util are needed for staged async file writes.
net/time/rt are needed by runtime-backed async HTTP tests/operation.
rt-multi-thread remains available for users; tests use current-thread runtime builder.
```

## New public backend types

Exported behind `reqwest` feature:

```rust
ReqwestResource
ReqwestAcquire
```

Shape:

```rust
pub struct ReqwestResource {
    pub client: reqwest::Client,
}

pub struct ReqwestAcquire<R = ReqwestResource> {
    pub resources: R,
}
```

Resource-control decision:

```text
ReqwestResource owns reusable shared resource handles only.
ReqwestAcquire is reusable and cloneable.
Operation-local state is not stored in the resource.
```

## Shared vs exclusive resources

Shared:

```text
reqwest::Client
connection pool inside client
TLS/proxy/DNS config inside client
future budget/semaphore handles if added later
```

Exclusive per operation:

```text
response body
StagedDownload<Open>
async file writer
byte counter
evidence builder
StagedDownload<Closed>
```

This prevents one Acquire operation from monopolizing the backend object.

## Design-level risk avoidance

Implemented private staging typestate:

```rust
StagedDownload<Open>
StagedDownload<Closed>
```

Only open stage can write:

```rust
StagedDownload<Open>::write_chunk(...).await
StagedDownload<Open>::finish().await -> StagedDownload<Closed>
```

Only closed stage can persist:

```rust
StagedDownload<Closed>::persist(...)
```

The ordering law is encoded by type shape:

```text
write chunks -> flush/drop writer in finish -> persist closed temp file
```

This avoids the Windows handle/persist risk by construction. The reqwest backend cannot persist while its async writer is still live because `persist` is unavailable on `StagedDownload<Open>`.

## Reqwest behavior

The async backend does:

```text
resolve destination parent
create parent directory
reject unsafe existing destination
clone shared reqwest::Client
build GET request
apply policy headers
apply request timeout
send request
manual status check
capture content_length
stream response with Response::chunk().await
write chunks into same-parent staged file
check max_bytes before writing excessive chunk
finish stage, closing writer
persist closed stage to final destination
return LocalMaterial::File + NetAcquireEvidence
```

Manual status check is intentional:

```text
Response::error_for_status() is not used because Pulith owns status/evidence mapping.
```

`Response::chunk().await` is intentional:

```text
No reqwest stream feature.
No futures-util dependency added for this slice.
```

## Tests added

Local loopback only; no external network.

Reqwest tests:

```text
reqwest_acquire_downloads_file_to_local_material
reqwest_acquire_rejects_non_success_status_without_touching_destination
reqwest_acquire_enforces_max_bytes_before_persist
reqwest_acquire_flows_into_hash_verify
reqwest_acquire_flows_into_local_apply_after_verify
```

Existing ureq tests remain feature-gated so `async net reqwest` can test without enabling `ureq`.

Test runtime:

```text
#[test] + tokio::runtime::Builder::new_current_thread().enable_all().build().block_on(...)
```

No `tokio/macros` feature was added.

## Files changed

```text
Cargo.toml
crates/pulith/Cargo.toml
crates/pulith/src/lib.rs
crates/pulith/src/net.rs
docs/report/pulith-reqwest-tokio-backed-acquire-execution-report.md
```

## Deferred

Still not implemented:

```text
runtime-neutral IsahcAcquire
smol-native backend
compio backend
retry policy
range/resume
mirror/multi-source
object_store acquire
bandwidth limiter
network semaphore/budget API
```

These remain separate slices. In particular, runtime-neutral HTTP should be a separate backend such as `isahc`, not hidden under `reqwest`.

## Verification plan

A fresh ad-hoc script should verify:

```text
cargo fmt --all --check
cargo check -p pulith --no-default-features
cargo check -p pulith --features "runtime-tokio"
cargo check -p pulith --features "async net reqwest"
cargo test -p pulith --features "async net reqwest" net::tests::
cargo test -p pulith --features "async net reqwest hash blake3" net::tests::reqwest
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::
cargo check --workspace --all-features
cargo test --workspace --all-features
git diff --check -- changed paths
```
