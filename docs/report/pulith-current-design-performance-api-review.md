# Pulith Current Design Performance/API Review

## Status

This is a design and code-shape review. No production code was changed in this pass.

Reviewed files:

```text
crates/pulith/src/net.rs
crates/pulith/src/behavior.rs
crates/pulith/src/lib.rs
crates/pulith/src/local.rs
crates/pulith/src/hash.rs
crates/pulith/src/application.rs
crates/pulith/src/error.rs
crates/pulith/Cargo.toml
```

Commands run for current feature surface:

```text
cargo tree -p pulith --features "async net reqwest" --depth 2
cargo tree -p pulith --features "sync local net ureq" --depth 2
cargo check -p pulith --features "async net reqwest"
cargo check -p pulith --features "sync local net ureq"
```

Both checked feature combinations compile.

## Executive summary

The current design is mostly sound for a first typed-tree migration slice:

```text
Typed behavior states are explicit.
Net Acquire stays separate from Verify/Prepare/Apply.
Sync ureq and Tokio-backed reqwest reuse explicit agent/client resources.
Feature axes are mostly orthogonal after introducing runtime-tokio.
Reqwest staging typestate prevents persist-before-close at the type level.
```

Main problems are not raw compute throughput. The bigger issues are API/state hygiene and async blocking boundaries:

```text
1. Too many public fields allow invalid states to be constructed outside constructors.
2. Async reqwest path still does small blocking filesystem operations on the async task.
3. Several public/feature surfaces are premature or dead: SelectFirst, object, compress, fs-extra, json.
4. AsyncAcquireNode currently boxes futures per operation.
5. Net policy headers are untyped Strings and are validated late/backend-specifically.
6. RemoteSource.destination conflates cache/material path with final apply target in naming.
```

Severity ranking:

```text
High: public invariant bypass on RemoteUrl/DigestValue/state nodes
Medium: blocking filesystem work inside async reqwest path
Medium: dead/premature feature flags and SelectFirst API
Medium: no shared resource budget for concurrent downloads
Low: boxed async future allocation, extra tempfile reopen, small clone/string overhead
```

## Performance review

### 1. Network streaming path is reasonable

Sync `ureq`:

```text
response.body_mut().as_reader()
16 KiB stack buffer
write_all into same-parent NamedTempFile
max_bytes checked before writing an excessive chunk
```

Tokio-backed `reqwest`:

```text
response.chunk().await
write_all into tokio::fs::File
max_bytes checked before writing an excessive chunk
StagedDownload<Open> -> finish -> StagedDownload<Closed> -> persist
```

Assessment:

```text
Good enough for current slice.
No full-body buffering.
No Vec accumulation of downloaded data.
No per-chunk heap allocation introduced by Pulith beyond backend chunk objects.
```

### 2. Missing content-length preflight wastes bandwidth on known-too-large downloads

Current code records `content_length` after success status but does not fail early if:

```text
content_length > policy.max_bytes
```

Instead, it starts streaming and fails when a chunk crosses the limit.

Impact:

```text
For large known-size bodies this wastes at least one chunk and sometimes more depending backend chunk sizing.
```

Recommendation:

```text
After status/content_length, if max_bytes exists and content_length exists and content_length > max_bytes, return DownloadLimitExceeded before staging/streaming.
```

Caveat:

```text
Keep streaming limit check anyway because content_length may be absent, wrong, or represent encoded/decoded length depending backend configuration.
```

### 3. Async reqwest path does blocking fs work inside async function

In `acquire_reqwest`:

```text
std::fs::create_dir_all
std::fs::symlink_metadata
tempfile::NamedTempFile::new_in
temp.reopen()
NamedTempFile::persist
```

These happen inside the async operation.

Impact:

```text
Usually small, but on slow/network filesystems or contended directories this can block a Tokio worker thread.
The design says Tokio-backed honestly, so this should either be accepted and documented or moved to spawn_blocking.
```

Recommendation:

```text
Keep as-is for first slice if simplicity wins.
Before high-concurrency downloads, isolate filesystem preflight/temp/persist in blocking sections or make a backend-common staging helper whose async variant explicitly uses spawn_blocking for blocking fs setup/persist.
```

Priority:

```text
Medium before adding concurrency budgets.
High if adding many concurrent reqwest downloads.
```

### 4. Boxed future in AsyncAcquireNode is acceptable but not zero-cost

`AsyncAcquireNode` permits a generic associated future, but `ReqwestAcquire` implements it as:

```rust
Pin<Box<dyn Future<Output = Result<...>> + 'a>>
```

Impact:

```text
One allocation and dynamic poll dispatch per acquire operation.
Negligible relative to network + file IO for real downloads.
Not ideal if AsyncAcquireNode becomes hot for tiny in-memory operations.
```

Recommendation:

```text
Do not optimize this now unless API ergonomics demand it.
If later optimizing, add backend-specific inherent async methods or revisit trait shape when stable language features make unnamed associated futures cleaner.
```

### 5. Extra temp-file reopen is a small cost

Reqwest staging creates a `NamedTempFile` then reopens it for Tokio:

```rust
tempfile::NamedTempFile::new_in(parent)
temp.reopen()
tokio::fs::File::from_std(...)
```

Impact:

```text
One extra open syscall and two handles during the write phase.
The typestate finish drops the Tokio writer before persist, so the safety property is still good.
Cost is negligible compared with network transfer.
```

Recommendation:

```text
Keep unless profiling shows high small-file throughput matters.
Do not sacrifice the typestate staging law for this micro-optimization.
```

### 6. Local directory apply is intentionally copy-heavy

`LocalApply` copies directory trees into a staged directory and then renames/replaces.

Impact:

```text
O(total bytes + entries) copy cost.
Good safety baseline, not optimized for same-filesystem directory moves/hardlinks.
```

Assessment:

```text
This is consistent with the prior design preference: safe default copy-only path, no hardlink optimization, no symlink preservation.
```

Do not optimize this unless the user explicitly asks for faster local apply policies.

## State design review

### 1. Public fields break invariants

Examples:

```rust
pub struct RemoteUrl {
    pub url: url::Url,
}

pub struct DigestValue<A> {
    pub value: String,
    _algorithm: PhantomData<A>,
}

pub struct RemoteSource {
    pub url: RemoteUrl,
    pub destination: PathBuf,
    pub policy: NetAcquirePolicy,
}
```

Problem:

```text
RemoteUrl::parse enforces http/https, but external callers can construct RemoteUrl { url } directly with file:// or other schemes.
DigestValue::new normalizes hex, but callers can mutate `value` after construction.
RemoteSource::new sets default policy, but all fields remain mutable from outside the module.
```

Recommendation:

```text
Make invariant-bearing fields private and add accessors/builders.
Keep evidence/result structs public if they are just facts.
```

Suggested direction:

```rust
pub struct RemoteUrl {
    url: url::Url,
}

impl RemoteUrl {
    pub fn parse(...) -> Result<Self, PulithError>;
    pub fn as_str(&self) -> &str;
    pub fn as_url(&self) -> &url::Url;
}
```

Priority:

```text
High before calling the public API stable.
```

### 2. Public typed-tree nodes are forgeable

Current state wrappers expose fields:

```rust
pub struct Acquired<I, M, E> { pub input: I, pub material: M, pub evidence: E }
pub struct Verified<I, M, E> { pub input: I, pub material: M, pub evidence: E }
pub struct Prepared<I, P, E> { pub input: I, pub prepared: P, pub evidence: E }
```

Problem:

```text
Callers can bypass behavior transitions and construct Verified/Prepared by hand.
That weakens the typed-tree law: the type name implies a behavior occurred, but public fields allow forgery.
```

Tradeoff:

```text
Public fields are convenient during migration and tests.
Private fields make composition more ceremony-heavy.
```

Recommendation:

```text
For stable API, make semantic state wrappers opaque or at least provide constructors only for leaf/test/internal paths.
If public construction remains intentionally allowed, rename these as data carriers rather than proof states; but that would weaken the DDD model.
```

Priority:

```text
High design issue, especially before external users depend on the API.
```

### 3. EvidenceChain grows type complexity

`EvidenceChain<A, B>` nests after every behavior:

```text
EvidenceChain<EvidenceChain<NetAcquireEvidence, DigestEvidence>, ApplyEvidence>
```

Runtime cost:

```text
Low: it is just nested values.
```

Compile/API cost:

```text
Potentially high for long pipelines: large generic types, noisy signatures, harder docs.
```

Recommendation:

```text
Keep for now; it preserves typed provenance.
If API becomes too noisy, introduce named receipt/evidence aliases per common path or an evidence-list abstraction behind a type alias, not a dynamic registry.
```

### 4. RemoteSource.destination naming is ambiguous

Current `RemoteSource` has:

```rust
pub destination: PathBuf
```

In composition tests it is often a cache/material path, while `Intent<..., LocalTarget>` holds the final apply target.

Problem:

```text
`destination` reads like final installation target, but Acquire should produce local material, not final placement.
```

Recommendation:

```text
Rename later to something like material_path, cache_path, or local_material_path.
Avoid doing this in the same slice as retry unless API churn is acceptable.
```

### 5. LocalAcquire follows symlinks when classifying material

`LocalAcquire` uses:

```rust
path.exists()
path.is_dir()
```

These follow symlinks.

Risk:

```text
LocalMaterial.kind can describe a symlink target instead of the path entry itself.
Later HashVerify and LocalApply reject symlink/special entries, so many bad cases are caught downstream, but the Acquired state itself may be misleading.
```

Recommendation:

```text
Use symlink_metadata in LocalAcquire too, and reject symlink/special entries at acquisition if the current safe-default policy is to reject symlinks.
```

Priority:

```text
Medium; not specifically part of net, but it aligns Acquire state with later safety policy.
```

## Public API exposure review

### 1. Dead public SelectFirst

`SelectFirst` is exported but not used in implementation:

```rust
pub struct SelectFirst;
impl<I, S> WithSource<I, S> { pub fn select_first(...) ... }
```

Problem:

```text
SelectFirst looks like a behavior node but does not implement SelectNode.
It is public vocabulary without behavior.
```

Recommendation:

```text
Either implement SelectNode<WithSource<I, S>> for SelectFirst or delete/unexport SelectFirst and keep only the method.
```

Priority:

```text
Medium; easy cleanup.
```

### 2. Premature feature flags with no code surface

`crates/pulith/Cargo.toml` includes:

```toml
object = ["net", "async", "dep:object_store"]
compress = ["dep:async-compression"]
fs-extra = ["dep:fs_extra"]
json = ["dep:serde", "dep:serde_json"]
```

Search found no active Rust code using these dependencies in `crates/pulith/src`.

Problem:

```text
These features compile optional dependencies without exposing typed behavior.
They are placeholders, not implementation capabilities.
This contradicts the feature rule: features enable concrete implementation families, not future intentions.
```

Recommendation:

```text
Delete dormant features until the corresponding typed behavior exists, or add real cfg-gated types immediately when implementing them.
Keep object_store for a future ObjectAcquire slice, not as a dead feature.
```

Priority:

```text
Medium-high for public API cleanliness.
```

### 3. Resource fields are public

Examples:

```rust
pub struct UreqResource { pub agent: ureq::Agent }
pub struct ReqwestResource { pub client: reqwest::Client }
pub struct UreqAcquire<R> { pub resources: R }
pub struct ReqwestAcquire<R> { pub resources: R }
```

Assessment:

```text
This is acceptable for explicit resource sharing in early design.
It lets callers inject configured clients/agents.
```

Concern:

```text
As delay providers, budgets, or semaphores are added, public fields make invariants harder to maintain.
```

Recommendation:

```text
Keep resource injection public or builder-based, but prefer constructors/accessors before adding retry/budget fields.
```

### 4. Header API validates too late

Current policy:

```rust
pub headers: Vec<(String, String)>
```

Problem:

```text
Invalid header names/values are accepted into policy and fail later in backend-specific ways.
It also permits accidental duplicate semantic headers without a clear policy.
```

Recommendation:

```text
For retry slice or before API stabilization, add validation at NetAcquirePolicy::header or switch to typed header types if an `http` dependency is acceptable.
Do not expose backend-specific reqwest/ureq header maps in the common policy.
```

## Dependency/feature performance

### Reqwest feature graph

`cargo tree -p pulith --features "async net reqwest" --depth 2` shows reqwest pulls:

```text
hyper
hyper-rustls
hyper-util
tokio
tower
tower-http
rustls
url
```

Assessment:

```text
Expected for reqwest/hyper stack.
Feature naming honestly says runtime-tokio and reqwest.
```

Concern:

```text
`net = ["local", "dep:url"]` means enabling reqwest also brings local dependencies same-file/tempfile/walkdir.
This is currently intentional because net output is LocalMaterial::File, but it couples net to local material.
```

Recommendation:

```text
Keep for now.
If object_store or memory material arrives, split material-local from net protocol later.
```

### Ureq feature graph

`cargo tree -p pulith --features "sync local net ureq" --depth 2` is smaller and expected:

```text
ureq
rustls
webpki-roots
url
same-file/tempfile/walkdir
```

No immediate issue.

## Recommended cleanup order

Before implementing retry, I would do a small API cleanup slice:

```text
1. Make invariant-bearing fields private:
   - RemoteUrl.url
   - DigestValue.value
   - possibly RemoteSource fields via accessors/builders

2. Delete or implement SelectFirst.

3. Delete dormant feature flags with no implementation:
   - object
   - compress
   - fs-extra
   - json
   unless the next slice immediately implements them.

4. Add content_length > max_bytes preflight in both ureq and reqwest.

5. Decide whether async reqwest fs preflight/persist stays documented-blocking or moves to spawn_blocking before adding concurrency budgets.
```

Then implement retry:

```text
NetRetryPolicy
NetAttemptEvidence
parse_retry_after with httpdate
sync/async injected delay providers
ureq retry loop
reqwest retry loop
```

## Final judgment

The computational design is not over-engineered and does not currently have obvious algorithmic bottlenecks for first-slice downloads. The performance-sensitive streaming path is bounded-memory and staged correctly.

The main design debt is API permeability: public fields and dormant features let callers observe or construct states that the typed-tree model is trying to make meaningful. Tightening those before adding retry/budget will make the next layer simpler and safer.
