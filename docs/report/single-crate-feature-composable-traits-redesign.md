# Single-Crate Feature-Gated Composable Traits Redesign

## Status

Design/research report only. No Rust code changes are authorized by this report alone.

This report supersedes the previous `pulith-core` proposal. The corrected direction is:

```text
Do not create another core crate and keep implementation crates around.
Rename the architecture center to one crate: `pulith`.
Use modules, feature flags, and composable traits inside that crate.
Cancel multi-crate aggregation as the public design.
```

Compatibility is not a constraint.

## Research checked

Installed Hermes skills searched/read:

- `architecture-documentation`
- `module-reduction`
- `ponytail`
- `composable-pipeline`
- `cli-tool-design`

No dedicated Rust trait/composition skill was installed locally. I therefore also read official/current Rust material:

- The Rust Book: advanced traits / associated types
- Rust Reference: associated items
- The Rust Book: trait objects
- Cargo Book: features and optional dependencies
- Cargo Book: workspaces
- Rust API Guidelines

Useful Rust design facts applied here:

- Associated types are appropriate when a trait owns a logical output type.
- Trait objects are useful for runtime-selected shared behavior, but should not be the default if static composition works.
- Cargo features are additive and are the normal way to gate optional dependencies and implementation families.
- A workspace is not required to model implementation boundaries; modules plus features can keep one user-facing crate.

## Current correction

The previous `pulith-core` design was still too close to the old mistake:

```text
contract crate + implementation crates
```

That still creates multi-crate aggregation, just with a better center.

The simpler design is:

```text
one crate: pulith
internal modules: behavior, material, evidence, apply, features
optional concrete implementations behind Cargo features
```

No `pulith-core`.
No public `pulith-fetch`, `pulith-archive`, `pulith-store`, `pulith-install` as peer workflow crates.

## Target Cargo shape

Current workspace should eventually collapse toward:

```text
crates/pulith
examples/runtime-manager
```

or even:

```text
pulith
examples/runtime-manager
```

if the repository root becomes the package later.

Initial crate name:

```toml
[package]
name = "pulith"
```

Feature sketch:

```toml
[features]
default = ["local"]
local = []
net = ["dep:reqwest", "dep:tokio"]
archive = ["dep:zip", "dep:tar", "dep:flate2", "dep:xz2", "dep:zstd"]
persist = ["dep:serde_json"]
hash = ["dep:sha2", "dep:blake3", "dep:hex"]
```

Rules:

- Features enable concrete implementations, not public workflow concepts.
- The behavior API exists without net/archive/persist.
- If a feature is disabled, its implementation modules do not compile/export.
- Avoid one feature per old crate name if that preserves old architecture mentally. Use behavior/implementation capability names instead.

## Minimal behavior vocabulary

The public vocabulary should be small:

```text
Application
Resource
Source
Target
Operation
Requirements
EvidencePolicy
Receipt
```

Candidate public call:

```rust
let receipt = pulith::apply(Application {
    resource,
    source,
    target,
    operation,
    requirements,
    evidence,
})?;
```

or explicit composition:

```rust
let receipt = Pulith::new(acquire, prepare, apply)
    .run(Application { ... })?;
```

Do not expose `fetch`, `archive`, `store`, `install`, `state` as mandatory user vocabulary.

## Simpler composable atom model

The previous atom list was too complex:

```text
resolve -> prove -> shape -> remember -> apply -> evidence
```

Reduce it to three composable behaviors:

```text
Acquire -> Prepare -> Apply
```

### Acquire

Question:

```text
Given resource + source + requirements, produce material and acquisition evidence.
```

Trait sketch:

```rust
pub trait Acquire {
    type Material;
    type Evidence;
    type Error;

    fn acquire(&self, app: &Application) -> Result<(Self::Material, Self::Evidence), Self::Error>;
}
```

Examples behind features:

- local path acquisition;
- HTTP acquisition;
- existing material acquisition;
- git acquisition later.

### Prepare

Question:

```text
Given material + acquisition evidence + target/requirements, produce prepared material and preparation evidence.
```

Trait sketch:

```rust
pub trait Prepare<M, E> {
    type Prepared;
    type Evidence;
    type Error;

    fn prepare(
        &self,
        material: M,
        evidence: E,
        app: &Application,
    ) -> Result<(Self::Prepared, Self::Evidence), Self::Error>;
}
```

Examples behind features:

- file passthrough;
- directory passthrough;
- archive extraction;
- wrapper/shim generation later.

### Apply

Question:

```text
Given prepared material + evidence + operation, apply it to target and return receipt.
```

Trait sketch:

```rust
pub trait Apply<P, E> {
    type Receipt;
    type Error;

    fn apply(
        &self,
        prepared: P,
        evidence: E,
        app: Application,
    ) -> Result<Self::Receipt, Self::Error>;
}
```

Examples behind features:

- copy/link into target;
- activate target;
- persist receipt/evidence;
- rollback snapshot when requested.

## Composition without a framework

A small generic runner is enough:

```rust
pub struct Pipeline<A, P, X> {
    acquire: A,
    prepare: P,
    apply: X,
}

impl<A, P, X> Pipeline<A, P, X> {
    pub fn run(&self, app: Application) -> Result<Receipt, Error>
    where
        A: Acquire,
        P: Prepare<A::Material, A::Evidence>,
        X: Apply<P::Prepared, P::Evidence>,
    {
        let (material, acquired) = self.acquire.acquire(&app)?;
        let (prepared, prepared_evidence) = self.prepare.prepare(material, acquired, &app)?;
        self.apply.apply(prepared, prepared_evidence, app)
    }
}
```

This is composable, but not a registry, factory, plugin manager, or orchestrator crate.

The default `pulith::apply(...)` can be a thin convenience over a default feature-enabled `Pipeline`, but the pipeline itself remains explicit and replaceable.

## Static vs dynamic composition

Prefer static composition first:

```rust
Pipeline<LocalAcquire, ArchivePrepare, TargetApply>
```

Use trait objects only when runtime configuration must select behavior dynamically:

```rust
Box<dyn DynAcquire>
```

Do not design trait-object registries first. Rust's generics and associated types give better compile-time contracts for the initial design.

## Module layout inside one crate

Target internal layout:

```text
crates/pulith/src/lib.rs
crates/pulith/src/application.rs
crates/pulith/src/pipeline.rs
crates/pulith/src/evidence.rs
crates/pulith/src/error.rs
crates/pulith/src/local.rs          # feature local, maybe default
crates/pulith/src/net.rs            # feature net
crates/pulith/src/archive.rs        # feature archive
crates/pulith/src/persist.rs        # feature persist
crates/pulith/src/target.rs         # apply-to-target behavior
```

Do not mirror old crate names exactly unless the module has a real behavior role.

Potential public API:

```rust
pub use application::{Application, Resource, Source, Target, Operation, Requirements, EvidencePolicy};
pub use pipeline::{Acquire, Prepare, Apply, Pipeline};
pub use evidence::{Evidence, Receipt};
```

Feature-gated exports:

```rust
#[cfg(feature = "local")]
pub use local::LocalAcquire;

#[cfg(feature = "archive")]
pub use archive::ArchivePrepare;

#[cfg(feature = "persist")]
pub use persist::JsonEvidenceStore;
```

## What gets deleted eventually

Once `pulith` exists and one simple pipeline works, delete/fold old crates in this direction:

```text
pulith-version  -> pulith::application/version module or simpler Version enum/string
pulith-resource -> pulith::application Resource/Requirements
pulith-source   -> pulith::Source / local/net implementation helpers
pulith-fetch    -> pulith::net implementation behind feature
pulith-archive  -> pulith::archive implementation behind feature
pulith-store    -> pulith::persist/evidence implementation behind feature
pulith-state    -> pulith::evidence/lifecycle implementation behind feature
pulith-install  -> pulith::target/apply implementation
pulith-fs       -> internal fs helpers, not public concept
```

No compatibility crates.
No re-export shims.
No old workspace package names preserved unless temporarily needed during one migration slice.

## First implementation slice

The first slice should not port all behavior.

Create one `pulith` crate with:

```text
Application
Resource
Source
Target
Operation
Requirements
EvidencePolicy
Acquire / Prepare / Apply
Pipeline
local path acquire
identity prepare
copy/apply target
receipt
```

This proves the behavior model with no net/archive/persist complexity.

The first vertical behavior can be:

```text
local file/tree source -> prepare as-is -> apply to target -> receipt
```

This is not “install file” as a product focus. It is the smallest proof that the behavior pipeline works.

## First slice acceptance gates

```bash
cargo check -p pulith --all-features
cargo test -p pulith local_application_pipeline
```

Structural gates:

```text
pulith has no dependency on old pulith-* crates.
old crates do not import pulith in first slice unless explicitly migrated.
public API exposes Application + Acquire/Prepare/Apply + Pipeline.
no registry/factory/plugin manager exists.
```

## Design corrections from previous report

Previous report was wrong in three ways:

1. It kept `pulith-core` as a second crate instead of moving to single `pulith`.
2. It listed too many atom traits.
3. It still thought concrete crates might remain as primary implementation packages.

Corrected design:

```text
one crate;
three atom traits;
features for optional implementations;
modules, not workspace crates;
static composition first;
old crates deleted/folded after proof.
```

## Current recommendation

Proceed with a design-first implementation plan for:

```text
create `pulith` crate and prove local Acquire -> Prepare -> Apply pipeline
```

Plan file should be:

```text
docs/report/pulith-single-crate-first-slice-plan.md
```

It should specify:

- exact new crate files;
- minimal public types;
- minimal trait signatures;
- feature flags;
- one local behavior test;
- no old crate migration yet except workspace member addition;
- deletion plan for old crates only after the new behavior spine compiles.

This matches the corrected top-down principle:

```text
behavior first;
composition first;
one crate;
feature-gated implementations;
delete old cross-crate aggregation after the new spine exists.
```
