# Pulith Behavior-First API and Resource Execution Report

## Status

Implemented the behavior-first cleanup plan before retry/budget work.

This slice changes code. It does not add retry yet.

## Implemented changes

### 1. Behavior constructors before privacy

Added crate-owned semantic constructors on behavior states:

```rust
Chosen::from_selected(...)
Acquired::from_acquire(...)
Verified::from_verify(...)
Prepared::from_prepare(...)
Applied::from_apply(...)
Remembered::from_remember(...)
```

These are `pub(crate)` so external callers cannot use them as public bypasses.

Each state also now exposes read-only accessors and ownership-consuming `into_parts`:

```rust
input()
source() / material() / prepared() / receipt()
evidence()
into_parts()
```

The state fields were changed from public to crate-visible:

```rust
pub(crate) input
pub(crate) source/material/prepared/receipt
pub(crate) evidence
```

Rationale:

```text
The type's premise is the behavior that constructs it.
Crate internals can still compose without churn.
External callers can inspect or consume through methods, but cannot directly forge state structs.
```

### 2. SelectFirst is now a real behavior node

Before:

```text
SelectFirst was exported but did not implement SelectNode.
WithSource::select_first constructed Chosen directly.
```

Now:

```rust
impl<I, S> SelectNode<WithSource<I, S>> for SelectFirst
```

and convenience sugar delegates through the behavior:

```rust
impl<I, S> WithSource<I, S> {
    pub fn select_first(self) -> Result<Chosen<I, S>, PulithError> {
        SelectFirst.select_node(self)
    }
}
```

So `Chosen` is now constructed by `Select` behavior, not just a struct wrapper.

### 3. Behavior impls now use semantic constructors

Updated production behavior implementations to call behavior-state constructors:

```text
LocalAcquire       -> Acquired::from_acquire
IdentityVerify     -> Verified::from_verify
IdentityPrepare    -> Prepared::from_prepare
LocalApply         -> Applied::from_apply
MemoryRemember     -> Remembered::from_remember
HashVerify         -> Verified::from_verify
ArchivePrepare     -> Prepared::from_prepare
UreqAcquire        -> Acquired::from_acquire
ReqwestAcquire     -> Acquired::from_acquire
```

This makes the behavior boundary explicit in code.

### 4. RemoteUrl and DigestValue invariants tightened

`RemoteUrl.url` is now private:

```rust
pub struct RemoteUrl {
    url: url::Url,
}
```

Accessors:

```rust
as_str()
as_url()
into_url()
```

This preserves the `RemoteUrl::parse` law:

```text
RemoteUrl means parsed absolute http/https URL.
```

`DigestValue.value` is now private:

```rust
pub struct DigestValue<A> {
    value: String,
    _algorithm: PhantomData<A>,
}
```

Accessors:

```rust
as_str()
into_string()
```

This preserves the normalization law from `DigestValue::new`.

### 5. Resource constructors made explicit

Added sync resource constructors:

```rust
UreqResource::from_agent(agent)
UreqResource::agent()
UreqAcquire::with_resource(resources)
UreqAcquire::resources()
```

Added Tokio-backed reqwest resource constructors:

```rust
ReqwestResource::from_client(client)
ReqwestResource::client()
ReqwestAcquire::with_resource(resources)
ReqwestAcquire::resources()
```

Fields are no longer public:

```rust
UreqResource { agent }
ReqwestResource { client }
UreqAcquire { resources }
ReqwestAcquire { resources }
```

`new()` remains convenience only:

```text
UreqAcquire::new() -> default ureq Agent
ReqwestAcquire::new() -> default reqwest Client
```

Resource acquisition is now explicit for callers who need configured clients/agents.

### 6. Runtime ownership decision preserved

No Tokio runtime was added to library resources.

Correct boundary remains:

```text
Runtime: caller-owned executor context.
ReqwestResource: shared reqwest::Client handle.
Acquire operation: request/response/staged temp/bytes/evidence.
```

Tests may create a runtime. Library acquire code does not.

### 7. Known-size oversize preflight

Added `reject_known_oversize` and applied it to both backends after status/content-length:

```text
if content_length exists and max_bytes exists and content_length > max_bytes:
    fail before staging/streaming
```

Streaming limit checks remain in place because content-length can be absent or unreliable.

## Files changed

```text
crates/pulith/src/behavior.rs
crates/pulith/src/local.rs
crates/pulith/src/hash.rs
crates/pulith/src/archive.rs
crates/pulith/src/net.rs
docs/report/pulith-behavior-first-api-resource-execution-report.md
```

## Verification result

Fresh ad-hoc verification passed:

```text
cargo fmt --all --check
cargo check -p pulith --no-default-features
cargo check -p pulith --features "sync local net ureq hash blake3"
cargo check -p pulith --features "async net reqwest hash blake3"
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::
  8 passed; 0 failed
cargo test -p pulith --features "async net reqwest hash blake3" net::tests::reqwest
  5 passed; 0 failed
cargo test -p pulith --features "sync local hash blake3 sha2"
  9 passed; 0 failed
cargo check --workspace --all-features
cargo test --workspace --all-features
  43 passed; 0 failed
git diff --check -- changed paths
VERIFY-PASS ad-hoc behavior-first API/resource cleanup verification
```

## Verification plan

Fresh ad-hoc verification command set:

```text
cargo fmt --all --check
cargo check -p pulith --no-default-features
cargo check -p pulith --features "sync local net ureq hash blake3"
cargo check -p pulith --features "async net reqwest hash blake3"
cargo test -p pulith --features "sync local net ureq hash blake3" net::tests::
cargo test -p pulith --features "async net reqwest hash blake3" net::tests::reqwest
cargo test -p pulith --features "sync local hash blake3 sha2"
cargo check --workspace --all-features
cargo test --workspace --all-features
git diff --check -- changed paths
```

Structural markers:

```text
impl<I, S> SelectNode<WithSource<I, S>> for SelectFirst
Chosen::from_selected
Acquired::from_acquire
Verified::from_verify
Prepared::from_prepare
Applied::from_apply
Remembered::from_remember
RemoteUrl { url: url::Url }
DigestValue<A> { value: String }
ReqwestResource::from_client
ReqwestAcquire::with_resource
UreqResource::from_agent
UreqAcquire::with_resource
reject_known_oversize
```

## Next recommended slice

After verification, proceed to retry only after deciding whether to:

```text
1. keep state fields pub(crate) for internal composition, or
2. push further to fully private fields and convert internal code to accessors/into_parts.
```

Current slice blocks external forging while minimizing internal churn.
