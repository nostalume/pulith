# Pulith Composable Tree Behavior Design

## Status

Design correction only. This document supersedes the monolithic `App` proof direction. It does not authorize code migration by itself.

Current correction:

```text
Delete App-style monolith design.
Build a compositional, inductive tree of typed behavior nodes.
Use methods, associated types, and typed conversions to construct behavior structure.
Cargo features expose concrete typed nodes/fields/impls at compile time.
```

## Why App must disappear

The current proof shape uses:

```rust
App { item, sources, target, op, need, evidence }
```

and then carries `App` through states:

```rust
Declared { app, evidence }
Chosen { app, source, evidence }
Acquired<M> { app, source, material, evidence }
Verified<M> { app, source, material, evidence }
Prepared<P> { app, source, prepared, evidence }
```

This is a monolith because it centralizes:

```text
intent
source candidates
operation mode
behavior needs
behavior evidence
execution context
future results
```

A monolith prevents induction: every behavior can see fields it should not know, so the type system cannot prove behavior orthogonality.

Replacement rule:

```text
No universal App.
No universal Need.
No universal Evidence bag as behavior input.
No transition state carries all prior context by default.
```

## Design target: typed behavior tree

Pulith should be a tree of typed behavior nodes.

A node is:

```text
input subtree + behavior morphism + output subtree + evidence subtree
```

It is not:

```text
a global request object passed through every stage
```

Tree example:

```text
Plan
├── Declare<Item>
├── Source
│   ├── Offer<LocalPath>
│   └── Select<First>
├── Acquire<LocalPath>
│   └── Material<File>
├── Verify<Hash<Blake3>>
│   ├── Need<Digest<Blake3>>
│   └── Evidence<Digest<Blake3>>
├── Prepare<Identity>
│   └── Prepared<File>
├── Apply<LocalTarget>
│   └── Receipt<Created>
└── Remember<Memory>
    └── Fact<Receipt>
```

The tree is inductive because each node is built from already-typed child nodes.

## Core shape

### Intent is a leaf, not a monolith

Use a small declaration leaf:

```rust
pub struct Intent<I, T, O> {
    item: I,
    target: T,
    op: O,
}
```

If source belongs to declaration, model it as a child node rather than a field bag:

```rust
pub struct WithSource<I, S> {
    intent: I,
    source: S,
}
```

Do not add `need` or `evidence` to `Intent`.

### Behavior nodes are typed wrappers

```rust
pub struct Offered<I, O> {
    input: I,
    offers: O,
}

pub struct Chosen<I, S> {
    input: I,
    source: S,
}

pub struct Acquired<I, M, E> {
    input: I,
    material: M,
    evidence: E,
}

pub struct Verified<I, M, E> {
    input: I,
    material: M,
    evidence: E,
}

pub struct Prepared<I, P, E> {
    input: I,
    prepared: P,
    evidence: E,
}
```

The `input` is the previous subtree, not a universal `App`.

This preserves history structurally while keeping each node narrow.

## Behavior traits with associated types

Each behavior owns its request, evidence, output, and error.

```rust
pub trait Verify<M> {
    type Need;
    type Evidence;
    type Error;

    fn verify(
        &self,
        material: M,
        need: Self::Need,
    ) -> Result<Verified<(), M, Self::Evidence>, Self::Error>;
}
```

For tree preservation:

```rust
pub trait VerifyNode<N> {
    type Need;
    type Material;
    type Evidence;
    type Error;
    type Output;

    fn verify_node(&self, node: N, need: Self::Need) -> Result<Self::Output, Self::Error>;
}
```

Example associated output:

```rust
type Output = Verified<N, Self::Material, Self::Evidence>;
```

Rule:

```text
The behavior trait chooses output shape through associated types.
The caller does not assemble result fields manually.
```

## Typed conversion, not public helper functions

Composition should use `From`, `TryFrom`, or dedicated methods when conversion has semantic meaning.

Examples:

```rust
impl<I, S> Offered<I, Vec<S>> {
    pub fn select_first(self) -> Result<Chosen<I, S>, SelectError> { ... }
}

impl<I, M, E> Acquired<I, M, E> {
    pub fn into_material(self) -> M { ... }
}
```

Behavior-specific conversion:

```rust
impl<I, M, A> TryFrom<(Acquired<I, M, AcquireEvidence>, DigestNeed<A>)>
    for Verified<I, M, DigestEvidence<A>>
where
    A: DigestAlgorithm,
{
    type Error = VerifyError;
}
```

But avoid generic `Into*` shim protocols used only for compatibility.

Allowed conversions:

```text
semantic narrowing
state transition
proof attachment
resource ownership transfer
```

Forbidden conversions:

```text
compatibility tuple conversion
caller glue conversion
old crate adapter conversion without semantic narrowing
```

## Feature-gated tree nodes

Cargo features should control which typed nodes can exist.

```rust
#[cfg(feature = "blake3")]
pub struct Blake3;

#[cfg(feature = "sha2")]
pub struct Sha256;

pub struct Digest<A> {
    _algorithm: PhantomData<A>,
}

#[cfg(feature = "blake3")]
pub type Blake3Digest = Digest<Blake3>;

#[cfg(feature = "sha2")]
pub type Sha256Digest = Digest<Sha256>;
```

Feature impact:

```text
without blake3: Blake3 type does not exist
without sha2: Sha256 type does not exist
without zip: ZipPrepare type does not exist
without reqwest: ReqwestAcquire type does not exist
```

This is compile-time availability, not runtime rejection.

## Hash Verify target shape

Current proof:

```rust
VerifyNeed::Digest { algorithm: String, value: String }
HashVerify.verify(acquired) // reads app.need.verify
```

Target:

```rust
#[cfg(feature = "blake3")]
pub struct Blake3;

#[cfg(feature = "sha2")]
pub struct Sha256;

pub struct DigestValue<A> {
    value: String,
    _algorithm: PhantomData<A>,
}

pub struct DigestNeed<A> {
    expected: DigestValue<A>,
}

pub struct DigestEvidence<A> {
    expected: DigestValue<A>,
    observed: DigestValue<A>,
}

pub struct HashVerify<A, R = NoHashResource> {
    resources: R,
    _algorithm: PhantomData<A>,
}
```

Static composition:

```rust
HashVerify::<Blake3>::default().verify_node(acquired, DigestNeed::<Blake3>::new(expected))
```

No algorithm string exists on the static path.

Runtime config adapter, if needed later:

```rust
pub enum AnyDigest {
    #[cfg(feature = "blake3")]
    Blake3(DigestNeed<Blake3>),
    #[cfg(feature = "sha2")]
    Sha256(DigestNeed<Sha256>),
}
```

Runtime adapter is explicitly a boundary object. It is not the core behavior.

## Apply target and operation as typed nodes

Instead of:

```rust
OpMode::Create | Replace | CreateOrReplace | Forget
```

prefer typed operation nodes where static composition matters:

```rust
pub struct Create;
pub struct Replace;
pub struct CreateOrReplace;
pub struct Forget;

pub struct Target<T, O> {
    location: T,
    op: PhantomData<O>,
}
```

or cfg-gated/runtime enum only if the operation is config-driven.

This allows:

```text
Apply<Create>
Apply<Replace>
Forget
```

to carry distinct laws without relying on runtime branching.

## Source as typed tree

Instead of:

```rust
Source::LocalPath(PathBuf)
Source::Url(String)
Source::Git { ... }
```

static source composition can use:

```rust
pub struct LocalPath(PathBuf);

#[cfg(feature = "reqwest")]
pub struct HttpUrl(Url);

#[cfg(feature = "git")]
pub struct GitSource { ... }
```

Then:

```rust
LocalAcquire: Acquire<LocalPath>
ReqwestAcquire: AsyncAcquire<HttpUrl>
```

Again, runtime enum is allowed only at config boundary:

```rust
pub enum AnySource {
    Local(LocalPath),
    #[cfg(feature = "reqwest")]
    Http(HttpUrl),
}
```

## Resource control as tree annotation

Resource control is not middleware.

It is a type parameter or associated context on nodes:

```rust
HashVerify<Blake3, NoHashResource>
HashVerify<Blake3, CpuBudget>
ReqwestAcquire<SharedClient>
ZipPrepare<TempQuota>
ApplyLocal<ExclusiveTarget>
```

Rule:

```text
Resource control annotates implementation nodes.
It does not become a global App field.
It does not become a universal ResourceManager.
```

## Public API aggregation

The public API should expose small aggregate constructors/methods, not raw field assembly.

Good:

```rust
let source = LocalPath::new(path)?;
let acquired = LocalAcquire.acquire(source)?;
let need = DigestNeed::<Blake3>::new(expected)?;
let verified = HashVerify::<Blake3>::default().verify_node(acquired, need)?;
```

Better once composition matures:

```rust
let tree = Intent::new(item, target)
    .with_source(LocalPath::new(path)?)
    .acquire_with(LocalAcquire)
    .verify_with(HashVerify::<Blake3>::default(), DigestNeed::new(expected)?)
    .prepare_with(IdentityPrepare)
    .apply_with(LocalApply::<Create>::new());
```

This is a typed tree builder. Each method consumes a typed node and returns a narrower next node.

## Do not recreate monolith under another name

Forbidden replacements:

```text
Context
Runtime
Session
Plan
Request
Run
```

if they merely gather all fields.

Allowed aggregate:

```text
Pipeline / Tree / Program
```

only if it is structurally generic:

```rust
pub struct Chain<A, B> {
    left: A,
    right: B,
}
```

and not a field bag.

## Inductive tree laws

Each node must satisfy:

```text
1. It owns only its semantic output.
2. It references previous context through typed input subtree, not a global App.
3. It has associated Need/Evidence/Error where behavior-specific.
4. It cannot observe fields outside its input type.
5. Feature-disabled node types do not exist.
6. Runtime enums appear only at external config boundaries.
7. Free helper functions remain private mechanism.
```

## Migration plan from current proof

### Stage 1 — Freeze current App as deprecated proof

Do not expand current `App`.

Mark conceptually:

```text
App = temporary proof object
```

### Stage 2 — Introduce typed leaves

Add:

```text
Intent<Item, Target, Op>
LocalPath
FileMaterial
LocalTarget
Create / Replace / Forget ZST operations
```

### Stage 3 — Introduce typed digest semantics

Add:

```text
Blake3 / Sha256 ZSTs under features
DigestValue<A>
DigestNeed<A>
DigestEvidence<A>
HashVerify<A, R>
```

### Stage 4 — Rewrite Verify trait shape

Move from:

```rust
fn verify(&self, acquired: Acquired<M>) -> Result<Verified<M>, Error>
```

to associated need/evidence output:

```rust
fn verify_node(&self, node: N, need: Self::Need) -> Result<Self::Output, Self::Error>
```

### Stage 5 — Remove stringly path

Delete:

```text
VerifyNeed::Digest { algorithm: String, value: String }
normalize_algorithm
UnsupportedDigestAlgorithm for static Verify
```

Keep runtime adapter only if required by config ingestion.

### Stage 6 — Remove App from transition states

Change:

```rust
Acquired<M> { app, source, material, evidence }
```

to:

```rust
Acquired<I, M, E> { input: I, material: M, evidence: E }
```

### Stage 7 — Rebuild local proof as typed tree

Only after the above, rebuild the local proof around typed methods.

## Acceptance criteria

Future code implementation must prove:

```text
No public App monolith remains in the main path.
No Digest behavior branch matches strings.
Feature-gated ZSTs/types disappear when feature is disabled.
Behavior traits own associated Need/Evidence types.
Transition states form an inductive tree with input subtree, output value, evidence.
Public construction uses methods/conversions, not raw field bags.
```

Verification commands:

```text
cargo check -p pulith --no-default-features
cargo check -p pulith --features 'sync local'
cargo check -p pulith --features 'sync local hash blake3'
cargo check -p pulith --features 'sync local hash sha2'
cargo check -p pulith --features 'sync local hash blake3 sha2'
cargo test -p pulith --features 'sync local hash blake3 sha2'
```

Structural marker checks:

```text
no `pub struct App` in final main path
no `algorithm: String` in digest need
no `normalize_algorithm` in static hash path
`pub struct Blake3` is cfg-gated by feature blake3
`pub struct Sha256` is cfg-gated by feature sha2
`trait Verify` has associated Need and Evidence
```

## Summary

```text
Delete App monolith.
Represent behavior as inductive typed tree.
Use ZSTs for static semantic choices.
Use cfg-gated enums only at runtime config boundaries.
Use associated types for behavior-specific Need/Evidence/Error/Output.
Use methods and typed conversions to grow the tree.
Feature-gated types/fields/impls define compile-time capability.
```
