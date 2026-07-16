# Pulith Static Typed Feature Behavior Design

## Status

Design correction only. This document does not authorize code migration by itself.

This document responds to the current critique:

```text
The current proof uses string matching rather than type semantics.
It puts request, behavior state, and result into App/transition states too broadly.
It exposes public functions instead of aggregating methods and typed conversions.
Cargo features should determine fields and behavior structures at compile time, not only hide modules.
```

## References read

### Cargo features

Rust Cargo reference states that features provide:

```text
conditional compilation and optional dependencies
```

and that features are declared in `[features]` and can enable other features or optional dependencies.

Important Cargo constraints from the reference:

```text
features are additive
features are package-local
feature unification takes the union of enabled dependency features
all features are disabled unless default enables them
--no-default-features disables defaults
```

Design consequence:

```text
Pulith features cannot be subtractive semantic switches.
A feature should add a typed capability, field, implementation, or associated type.
It must not make the same public type mean something different.
```

### Rust conditional compilation

The Rust reference states that configuration options are set statically during crate compilation and that Cargo conventionally sets `feature = "..."` cfg values.

Design consequence:

```text
Feature-gated semantic availability should be expressed with #[cfg(feature = "...")] on fields, variants, impls, and marker types.
Do not merely gate modules while leaving all semantic fields as runtime strings/options.
```

### Associated types and traits

The Rust Book says associated types connect type placeholders with traits, and implementors specify the concrete type.

Design consequence:

```text
Behavior traits should use associated types for Need, Evidence, Material, Prepared, Error, and resource control context.
Do not put every possible request/result field into App.
```

## Diagnosis of current proof code

Current files inspected:

```text
crates/pulith/src/application.rs
crates/pulith/src/behavior.rs
crates/pulith/src/hash.rs
```

### Problem 1 — stringly typed semantics

Current digest request:

```rust
VerifyNeed::Digest { algorithm: String, value: String }
```

Current digest implementation:

```rust
match normalize_algorithm(algorithm).as_str() {
    "blake3" => ...,
    "sha256" => ...,
    other => ...,
}
```

This is runtime string dispatch. It violates the behavior-as-type direction.

Correct direction:

```text
Digest algorithm is type semantics, not a string.
```

Preferred shapes:

```rust
pub struct Blake3;
pub struct Sha256;

pub trait DigestAlgorithm {
    const NAME: &'static str;
    type Output;
}
```

or, when caller must choose at runtime:

```rust
#[cfg(any(feature = "blake3", feature = "sha2"))]
pub enum DigestKind {
    #[cfg(feature = "blake3")]
    Blake3,
    #[cfg(feature = "sha2")]
    Sha256,
}
```

Rule:

```text
Use ZST marker types for static composition.
Use cfg-gated enums only when runtime choice is truly a domain requirement.
Never use free strings for algorithm identity in behavior decisions.
```

### Problem 2 — App carries too much

Current `App` carries:

```rust
item
sources
target
op
need
evidence
```

Then transition states repeat `app`:

```rust
Acquired<M> { app, source, material, evidence }
Verified<M> { app, source, material, evidence }
Prepared<P> { app, source, prepared, evidence }
```

This makes `App` an omnibus request/result/context object.

Correct direction:

```text
App should be declaration/intent only.
Behavior-specific request fields belong to behavior-specific Need associated types.
Behavior results belong to transition states, not to App.
```

Proposed split:

```rust
pub struct Intent<I, S, T, O> {
    pub item: I,
    pub source: S,
    pub target: T,
    pub op: O,
}

pub trait Verify<M> {
    type Need;
    type Evidence;
    type Error;
    fn verify(&self, material: M, need: Self::Need) -> Result<Verified<M, Self::Evidence>, Self::Error>;
}
```

The transition may keep only the facts needed for the next morphism, not the entire original App.

Rule:

```text
Data follows the morphism.
Do not carry the whole request through every state unless the next behavior actually needs it.
```

### Problem 3 — result, behavior, and request are mixed

Current `Verified<M>` contains:

```rust
app
source
material
evidence
```

This conflates:

```text
request: need/policy
behavior result: verified material
evidence: observable proof
original intent: app/source/target
```

Correct direction:

```text
Request is input to a behavior.
Result is the target semantic state.
Evidence is a fact emitted by the behavior.
```

Typed shape:

```rust
pub struct VerifyRequest<A> {
    pub algorithm: A,
    pub expected: DigestValue<A>,
}

pub struct Verified<M, E> {
    pub material: M,
    pub evidence: E,
}
```

No target path, operation mode, or source selection should exist in `Verified` unless Verify semantics require them.

### Problem 4 — public functions expose mechanics

Current `hash.rs` has free functions:

```rust
digest_file
normalize_algorithm
normalize_hex
digest_blake3
digest_sha256
copy_into_hasher
```

Most are private, but the design pressure is still function-first. Behavior should be aggregated as types and methods.

Correct direction:

```text
Behavior implementation is a struct carrying its type semantics and resource controls.
Conversions between states are typed associated conversions, not public helper functions.
```

Preferred shape:

```rust
pub struct HashVerify<A, R = NoSharedResource> {
    resources: R,
    _algorithm: PhantomData<A>,
}

impl<A, R> Verify<FileMaterial> for HashVerify<A, R>
where
    A: DigestAlgorithm,
    R: HashResources,
{
    type Need = DigestNeed<A>;
    type Evidence = DigestEvidence<A>;
    type Error = VerifyError;
}
```

Mechanism helpers remain private implementation details.

## Design philosophy

### 1. Features produce types, not runtime booleans

Bad:

```rust
VerifyNeed::Digest { algorithm: String, value: String }
```

Good static form:

```rust
#[cfg(feature = "blake3")]
pub struct Blake3;

#[cfg(feature = "sha2")]
pub struct Sha256;
```

Cargo features should determine whether these types and impls exist:

```rust
#[cfg(feature = "blake3")]
impl DigestAlgorithm for Blake3 { ... }

#[cfg(feature = "sha2")]
impl DigestAlgorithm for Sha256 { ... }
```

This makes unsupported combinations fail at compile time when statically composed.

### 2. Features gate fields and variants, not only modules

Module-level gating is too coarse:

```rust
#[cfg(feature = "hash")]
pub mod hash;
```

Better semantic gating:

```rust
pub enum VerifyNeed<A = NoVerify> {
    None,
    #[cfg(any(feature = "blake3", feature = "sha2"))]
    Digest(DigestNeed<A>),
}
```

or stricter:

```rust
pub struct NoVerify;
pub struct DigestNeed<A: DigestAlgorithm> {
    pub expected: DigestValue<A>,
}
```

Then `DigestNeed<Blake3>` cannot compile unless `Blake3` exists.

### 3. Prefer ZST marker types for static behavior

Use ZSTs when the behavior choice is known by composition:

```rust
HashVerify<Blake3>
HashVerify<Sha256>
ZipPrepare
TarPrepare
ReqwestAcquire
UreqAcquire
```

ZSTs are ideal here because they represent type-level semantics without runtime storage.

### 4. Use enums only for true runtime choice

Runtime enum is valid only when declared domain behavior includes dynamic selection:

```rust
pub enum DigestKind {
    #[cfg(feature = "blake3")]
    Blake3,
    #[cfg(feature = "sha2")]
    Sha256,
}
```

This is acceptable for config-file driven behavior.

Rule:

```text
ZST first.
Enum only when runtime choice is semantic.
String never for behavior identity.
```

### 5. App is declaration, not execution context

`App` should not become:

```text
request bag + behavior result bag + evidence bag + execution context
```

The public declaration should be narrow:

```rust
Intent<Item, Source, Target, Op>
```

Behavior-specific needs should attach to behaviors:

```rust
AcquireNeed
VerifyNeed<A>
PrepareNeed<K>
ApplyNeed
RememberNeed
```

This prevents `Verify` from knowing about apply target, and prevents `Prepare` from seeing network policy.

### 6. Behavior owns methods and conversions

Do not model the API as:

```text
public free helper functions
manual caller choreography
string/config switching
```

Model it as:

```text
implementation structs + behavior trait impls + typed transition conversion
```

Example direction:

```rust
let verified = HashVerify::<Blake3>::default().verify(acquired, DigestNeed::<Blake3>::new(expected))?;
```

or aggregated:

```rust
engine.verify(acquired, need)?
```

But the important part is:

```text
caller should not assemble low-level fields by hand.
```

### 7. Resource control is type/context, not glue

Resource control should not become `ResourceManager` middleware.

Instead:

```rust
ReqwestAcquire<SharedNet>
HashVerify<Blake3, CpuBudget>
ZipPrepare<TempQuota>
ApplyLocal<ExclusiveTarget>
```

The control belongs to the implementation type, while the behavior law remains unchanged.

## Proposed next design shape

### Static digest semantics

```rust
pub trait DigestAlgorithm {
    const NAME: &'static str;
    fn digest_file(path: &Path) -> Result<String, VerifyError>;
}

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
```

### Typed Verify implementation

```rust
pub struct HashVerify<A, R = NoHashResource> {
    resources: R,
    _algorithm: PhantomData<A>,
}

impl<A, R> Verify<FileMaterial> for HashVerify<A, R>
where
    A: DigestAlgorithm,
    R: HashResource,
{
    type Need = DigestNeed<A>;
    type Evidence = DigestEvidence<A>;
    type Error = VerifyError;
}
```

This removes:

```text
algorithm String
normalize_algorithm runtime match
UnsupportedDigestAlgorithm for static path
```

If runtime config wants to choose digest algorithm, add a separate cfg-gated runtime adapter:

```rust
pub enum AnyHashVerify {
    #[cfg(feature = "blake3")]
    Blake3(HashVerify<Blake3>),
    #[cfg(feature = "sha2")]
    Sha256(HashVerify<Sha256>),
}
```

This adapter is explicitly runtime selection, not the core behavior.

### App split

Current:

```rust
App { item, sources, target, op, need, evidence }
```

Target:

```rust
Intent { item, source, target, op }
```

Then behavior states carry only what they own:

```rust
Chosen<S> { source: S }
Acquired<M, E> { material: M, evidence: E }
Verified<M, E> { material: M, evidence: E }
Prepared<P, E> { prepared: P, evidence: E }
Applied<R> { receipt: R }
```

No transition should copy `App` by default.

### Feature-gated behavior structures

```rust
#[cfg(feature = "blake3")]
pub type Blake3Verify = HashVerify<Blake3>;

#[cfg(feature = "sha2")]
pub type Sha256Verify = HashVerify<Sha256>;
```

Feature controls which concrete behavior structures exist.

The behavior trait remains always defined.

## Migration correction plan

### Step 1 — Design-only correction accepted here

This document supersedes the stringly hash design in the previous implementation slice.

### Step 2 — Replace string digest with typed digest

Change:

```rust
VerifyNeed::Digest { algorithm: String, value: String }
```

to either:

```rust
DigestNeed<Blake3>
DigestNeed<Sha256>
```

or a cfg-gated `DigestKind` adapter only for runtime config.

### Step 3 — Split App into intent plus behavior needs

Do not keep `need` and `evidence` in App as universal fields.

### Step 4 — Move hash behavior to typed implementation structs

Change:

```rust
HashVerify.verify(acquired) // reads algorithm string from app.need.verify
```

to:

```rust
HashVerify::<Blake3>::default().verify(acquired, DigestNeed::<Blake3>::new(expected))
```

or equivalent associated-type design.

### Step 5 — Keep public surface aggregated

Expose:

```rust
HashVerify<Blake3>
DigestNeed<Blake3>
DigestEvidence<Blake3>
```

Do not expose helper functions or caller-assembled internals.

### Step 6 — Verify feature behavior

Required checks for the future code correction:

```text
cargo check -p pulith --no-default-features
cargo check -p pulith --features 'sync local hash blake3'
cargo check -p pulith --features 'sync local hash sha2'
cargo check -p pulith --features 'sync local hash blake3 sha2'
cargo test -p pulith --features 'sync local hash blake3 sha2'
```

Compile-fail or structural tests should prove:

```text
Blake3 type does not exist without feature blake3.
Sha256 type does not exist without feature sha2.
HashVerify<Blake3> cannot be constructed without blake3.
String algorithm matching is absent from static path.
```

## Current implementation disposition

Current hash implementation status:

```text
accepted as a proof only
not accepted as final design
must be replaced by static typed digest semantics
```

Current `App` status:

```text
accepted as transitional proof only
must shrink to intent/declaration
behavior-specific requests/results must move to behavior types/states
```

Current sync/async split status:

```text
usable directionally
but should be refined so behavior traits own associated Need/Evidence types
```

## Summary rule

```text
Cargo feature -> cfg-gated type/field/variant/impl availability.
Behavior identity -> ZST marker or cfg-gated enum.
Behavior request -> behavior-associated Need type.
Behavior result -> transition state.
Behavior proof -> evidence associated with that behavior.
App -> declaration only.
Mechanism helpers -> private.
```
