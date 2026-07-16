# Pulith Behavior-First API and Resource Plan

## Status

This report optimizes the previous performance/API cleanup plan according to the correction:

```text
A type's premise is behavior construction, not merely wrapping subtypes.
Do not prioritize privacy as a cosmetic wrapper step before establishing behavior.
```

No production code is changed in this pass.

Reviewed current code shape:

```text
crates/pulith/src/net.rs
crates/pulith/src/behavior.rs
crates/pulith/src/lib.rs
docs/report/pulith-current-design-performance-api-review.md
```

Loaded design references:

```text
references/pulith-composable-typed-tree-migration.md
references/pulith-async-runtime-resource-control.md
references/pulith-tokio-backed-reqwest-acquire.md
```

## Corrected premise

The previous plan was too focused on field privacy as the first move. Field privacy is still useful, but it should be a consequence of behavior boundaries, not the starting point.

Correct rule:

```text
First define which behavior constructs a state.
Then make all other construction paths impossible or non-primary.
Only then privatize fields where they protect a behavior law.
```

So the goal is not:

```text
wrap url::Url -> hide field
```

The goal is:

```text
RemoteUrl exists because a ParseRemoteUrl / HttpUrl validation behavior happened.
Chosen exists because Select chose from offered sources.
Acquired exists because Acquire produced material/evidence.
Verified exists because Verify checked a need against material.
Prepared exists because Prepare transformed material under a need.
```

Privacy is the enforcement mechanism after the behavior constructor is named.

## Reframed state design

### Current issue

Current state types are structurally forgeable:

```rust
pub struct Chosen<I, S> {
    pub input: I,
    pub source: S,
}

pub struct Acquired<I, M, E> {
    pub input: I,
    pub material: M,
    pub evidence: E,
}
```

The problem is not only that fields are public. The deeper issue is:

```text
The type name says a behavior happened, but public struct construction can bypass that behavior.
```

### Correct design direction

Each semantic state should have one primary constructor: the behavior that creates it.

```text
WithSource is constructed by Declare/AttachSource.
Chosen is constructed by Select.
Acquired is constructed by Acquire.
Verified is constructed by Verify.
Prepared is constructed by Prepare.
Applied is constructed by Apply.
Remembered is constructed by Remember.
```

The public API should make the behavior path obvious:

```rust
let offered = intent.with_source(source);
let chosen = SelectFirst.select_node(offered)?;
let acquired = UreqAcquire::new().acquire_node(chosen)?;
let verified = HashVerify::<Blake3>::new().verify_node(acquired, need)?;
```

not:

```rust
let acquired = Acquired { input, material, evidence };
```

## Optimized cleanup priority

### Slice A — establish behavior constructors before field privacy

Implement or correct behavior nodes first:

```text
SelectFirst must implement SelectNode<WithSource<I, S>>.
RemoteUrl parsing should be framed as construction behavior or fallible constructor with named semantics.
RemoteSource construction should clarify material path selection.
State constructors should be crate-private or behavior-owned.
```

Concrete code shape:

```rust
impl<I, S> SelectNode<WithSource<I, S>> for SelectFirst {
    type Source = S;
    type Error = PulithError;
    type Output = Chosen<I, S>;

    fn select_node(&self, node: WithSource<I, S>) -> Result<Self::Output, Self::Error> {
        Ok(Chosen::from_selected(node.input, node.source))
    }
}
```

Then expose method sugar through the behavior, not parallel to it:

```rust
impl<I, S> WithSource<I, S> {
    pub fn select_first(self) -> Result<Chosen<I, S>, PulithError> {
        SelectFirst.select_node(self)
    }
}
```

Important: `Chosen::from_selected` should not become a public bypass. It can be `pub(crate)` or test-only.

### Slice B — introduce behavior-owned state constructors

Add crate-private constructors to state wrappers:

```rust
impl<I, S> Chosen<I, S> {
    pub(crate) fn from_selected(input: I, source: S) -> Self { ... }
}

impl<I, M, E> Acquired<I, M, E> {
    pub(crate) fn from_acquire(input: I, material: M, evidence: E) -> Self { ... }
}

impl<I, M, E> Verified<I, M, E> {
    pub(crate) fn from_verify(input: I, material: M, evidence: E) -> Self { ... }
}
```

Behavior impls call these constructors. This makes the intended behavior boundary explicit before field privacy changes.

### Slice C — then privatize only behavior-protecting fields

After behavior constructors are in place, privatize fields that protect laws:

```text
RemoteUrl.url: protects http/https validation law.
DigestValue.value: protects normalization law.
Chosen/Acquired/Verified/Prepared/Applied fields: protect behavior provenance law.
```

Expose accessors for read-only facts:

```rust
impl RemoteUrl {
    pub fn as_url(&self) -> &url::Url;
    pub fn as_str(&self) -> &str;
}

impl<I, S> Chosen<I, S> {
    pub fn input(&self) -> &I;
    pub fn source(&self) -> &S;
    pub fn into_parts(self) -> (I, S); // if composition needs ownership
}
```

This avoids private-field churn before behavior semantics are set.

### Slice D — rename RemoteSource.destination by behavior role

Current name:

```rust
RemoteSource { destination: PathBuf }
```

Behavior role is not final target. It is the path where Acquire materializes a remote source.

Rename later to one of:

```text
material_path
local_material_path
cache_path
```

Preferred:

```rust
pub struct RemoteSource {
    url: RemoteUrl,
    material_path: PathBuf,
    policy: NetAcquirePolicy,
}
```

Why:

```text
Intent target is Apply's target.
RemoteSource material path is Acquire's output location.
Using `destination` for both concepts blurs behavior boundaries.
```

Do this with behavior constructor/accessor changes, not as an isolated rename.

## Reqwest runtime/resource initialization analysis

### Current code fact

Current library code does not create a Tokio runtime inside `ReqwestAcquire`:

```rust
ReqwestAcquire::new()
  -> ReqwestResource::default()
  -> reqwest::Client::new()
```

The runtime creation appears in tests only:

```rust
tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(future)
```

So if the question is:

```text
Should library Acquire create or own a Tokio runtime in the function?
```

Answer:

```text
No. That would not fit the resource-acquisition design.
```

A Tokio runtime is an execution environment, not an Acquire resource. Pulith should require the caller to execute async behavior within an existing runtime.

### Is fixed reqwest client initialization in a function acceptable?

Current `ReqwestResource::default()` calls:

```rust
reqwest::Client::new()
```

This is acceptable as convenience for first slice, but it is not sufficient as the primary resource-acquisition design.

Reason:

```text
Reqwest client owns shared connection pool and transport configuration.
Creating it via a fixed default hides important resource policy:
- pool behavior
- timeout defaults
- redirect policy
- proxy/TLS/system settings
- user agent/default headers
- future retry delay/budget handles
```

Better distinction:

```text
ReqwestAcquire::new() is a convenience constructor.
ReqwestAcquire::with_resource(resource) should be the primary explicit path.
ReqwestResource::from_client(client) should be the resource-acquisition constructor.
```

### Runtime vs resource model

Correct model:

```text
Runtime: caller-owned executor context. Not stored in ReqwestResource. Not created inside acquire_node_async.
ReqwestResource: shared transport/resource handles. Stores reqwest::Client and future budgets/delay providers.
Operation state: request, response, staged download, byte counters, evidence builder.
```

So `ReqwestResource` should not be:

```rust
pub struct ReqwestResource {
    runtime: tokio::runtime::Runtime, // reject
    client: reqwest::Client,
}
```

And `acquire_node_async` should not do:

```rust
tokio::runtime::Runtime::new()?.block_on(...); // reject
```

Correct:

```rust
pub struct ReqwestResource {
    client: reqwest::Client,
    // later: delay, budget, temp quota, etc.
}

impl ReqwestResource {
    pub fn from_client(client: reqwest::Client) -> Self;
    pub fn client(&self) -> &reqwest::Client;
}
```

### Should request timeout remain per function/policy?

Current request timeout is applied per operation:

```rust
if let Some(timeout) = source.policy.timeout {
    request = request.timeout(timeout);
}
```

This is correct.

Reason:

```text
Timeout is operation policy, not shared resource identity.
Different RemoteSource values may have different timeout/max_bytes/header policies.
```

But fixed client construction is separate. Client-level defaults can exist, but operation policy should override/apply at request construction.

## Optimized next plan

### Phase 1 — behavior-constructor cleanup, not privacy-first

Files:

```text
crates/pulith/src/behavior.rs
crates/pulith/src/local.rs
crates/pulith/src/net.rs
crates/pulith/src/hash.rs
crates/pulith/src/lib.rs
```

Tasks:

```text
1. Implement SelectNode for SelectFirst.
2. Route WithSource::select_first through SelectFirst.select_node.
3. Add crate-private behavior constructors for Chosen/Acquired/Verified/Prepared/Applied/Remembered.
4. Update behavior impls to use those constructors.
5. Only after behavior constructors exist, make fields private or plan the exact public accessors.
```

Verification markers:

```text
impl<I, S> SelectNode<WithSource<I, S>> for SelectFirst
Chosen::from_selected
Acquired::from_acquire
Verified::from_verify
Prepared::from_prepare
Applied::from_apply
select_first delegates to SelectFirst.select_node
```

### Phase 2 — resource constructor cleanup

Files:

```text
crates/pulith/src/net.rs
```

Tasks:

```text
1. Add ReqwestResource::from_client(client).
2. Add ReqwestAcquire::with_resource(resources).
3. Add UreqResource::from_agent(agent).
4. Add UreqAcquire::with_resource(resources).
5. Keep new() as convenience only.
6. Do not create runtime inside library code.
```

Potential future shape for retry:

```rust
pub struct ReqwestResource {
    client: reqwest::Client,
    delay: AsyncDelay,
    budget: Option<Arc<NetBudget>>,
}
```

But do not add delay/budget until retry/budget slice.

### Phase 3 — behavior-law privacy/accessors

Only after Phase 1/2:

```text
1. Make RemoteUrl.url private; expose as_url/as_str/into_url if needed.
2. Make DigestValue.value private; expose as_str/into_string if needed.
3. Make state wrapper fields private if tests and behavior impls are converted.
4. Keep evidence fields public if they are pure observations.
```

### Phase 4 — small performance guard

Add content-length preflight:

```text
if content_length exists and max_bytes exists and content_length > max_bytes:
    fail before temp/stage/stream
```

Both ureq and reqwest.

### Phase 5 — retry slice

Proceed with prior retry plan only after behavior/resource cleanup:

```text
NetRetryPolicy
NetAttemptEvidence
parse_retry_after with httpdate
injected sync/async delay providers
backend retry loops
```

## Revised priority order

Old order:

```text
private fields -> cleanup -> retry
```

New order:

```text
behavior constructors -> explicit resource constructors -> privacy/accessors -> small perf guard -> retry
```

This matches the corrected principle:

```text
A type is justified by the behavior that constructs it.
Privacy enforces that behavior law; privacy is not itself the design.
```

## Direct answer on runtime/resource design

```text
- Creating a Tokio runtime inside acquire_node_async would be wrong.
- Current library code does not do that; tests do, which is acceptable.
- ReqwestResource::default() using reqwest::Client::new() is acceptable as a convenience, but should not be the only or primary resource-construction API.
- The resource design should expose from_client/with_resource so callers can provide configured shared clients.
- Operation-level parameters like timeout/max_bytes/headers belong in RemoteSource/NetAcquirePolicy and are correctly applied per request.
```
