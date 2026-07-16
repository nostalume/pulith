# Pulith Core Interface-First Redesign

## Status

Design/plan report only. No Rust code changes are authorized by this report alone.

This report accepts the stronger correction:

```text
The current crate design should be considered cancelled as the architectural authority.
It defines many concrete behaviors, then re-aggregates them across crates.
The next design should create a core library (`pulith-core` or `pulith`) that defines behavior-first abstractions, then let concrete crates implement those behaviors.
```

Compatibility is not a constraint. These crates have no external users that require preserving the current public APIs.

## Problem with current crate design

The current workspace is split into concrete mechanism crates:

```text
pulith-version
pulith-resource
pulith-source
pulith-fetch
pulith-archive
pulith-store
pulith-state
pulith-install
pulith-fs
```

This looks modular, but the user-facing workflow is still assembled by hand across them.

The design failure is:

```text
Concrete behavior crates exist before the abstract workflow contract.
```

So the API forces callers to know and sequence concrete implementation owners:

```text
source planning -> fetch -> fs path -> archive -> store -> install -> state
```

But the top-level behavior is simply:

```text
apply resource from source to target with operation requirements and retained evidence
```

The current crates define too many bottom-up nouns before defining the top-down verbs.

## Redesign thesis

Create one core behavior crate first:

```text
pulith-core
```

or, if this is the primary library name:

```text
pulith
```

This crate owns the vocabulary of behavior:

```text
ResourceApplication
MaterialSource
MaterialHandle
MaterialEvidence
PreparedMaterial
RememberedMaterial
ApplicationTarget
ApplicationOperation
ApplicationRequirements
EvidencePolicy
ApplicationReceipt
```

Concrete crates then become implementation plugins/adapters for those behaviors, not peer public workflow owners.

## Naming choice

### Option A — `pulith-core`

Pros:

- explicit that this crate is the abstract behavior contract;
- current concrete crates can temporarily remain while implementing core traits;
- avoids name collision if a future façade crate wants to be `pulith`.

Cons:

- final user-facing crate may still be another layer;
- `core` can become a dumping ground if not kept trait/request-only.

### Option B — `pulith`

Pros:

- best final user-facing name;
- makes the primary API clear;
- discourages users from depending on concrete mechanism crates directly.

Cons:

- higher migration blast radius;
- if we still need implementation crates, `pulith` becomes both contract and façade unless carefully separated.

### Recommendation

Start with:

```text
pulith-core
```

for the design/implementation migration, then decide whether to rename/re-export as `pulith` after the core API stabilizes.

Do not create both `pulith-core` and `pulith` at the same time.

## Core ownership rule

`pulith-core` owns behavior contracts, not concrete I/O.

Allowed in core:

- request/receipt structs;
- trait definitions;
- lightweight enums for behavior semantics;
- error traits/categories if needed;
- no async runtime choice;
- no filesystem implementation;
- no HTTP client implementation;
- no archive codec implementation;
- no store/state persistence implementation.

Forbidden in core:

- `reqwest`;
- `tokio` runtime construction;
- zip/tar extraction;
- hardlink/copy/symlink implementation;
- concrete JSON persistence;
- store directory layout;
- current multi-crate orchestration glued directly into one god object.

Core is the behavior vocabulary and composition contract.

## Behavior-first core graph

The core workflow is:

```text
ResourceApplication
  -> MaterialResolver
  -> MaterialProver
  -> MaterialShaper
  -> MaterialMemory
  -> ResourceApplier
  -> EvidenceRecorder
```

Main path:

```rust
let receipt = engine.apply(ResourceApplication { ... })?;
```

Atom path:

```rust
let material = resolver.resolve(&request.resource, &request.source)?;
let evidence = prover.prove(&material, &request.requirements)?;
let prepared = shaper.shape(material, evidence, &request.target)?;
let remembered = memory.remember(prepared, &request.evidence)?;
let receipt = applier.apply(remembered, &request.target, &request.operation)?;
```

The main path must be implemented by composing atom traits, not by a hidden monolith.

## Proposed core types

### `ResourceApplication`

```rust
pub struct ResourceApplication {
    pub resource: ResourceSubject,
    pub source: MaterialSource,
    pub target: ApplicationTarget,
    pub operation: ApplicationOperation,
    pub requirements: ApplicationRequirements,
    pub evidence: EvidencePolicy,
}
```

### `ResourceSubject`

```rust
pub struct ResourceSubject {
    pub id: ResourceId,
    pub version: VersionIntent,
    pub behavior: ResourceBehavior,
}
```

This may replace or absorb current `RequestedResource` / `ResolvedResource` split if that split only forces caller branching.

### `MaterialSource`

```rust
pub enum MaterialSource {
    LocalPath(PathBuf),
    RemoteUrl(Url),
    CandidateSet(CandidateSet),
    ExistingMaterial(MaterialHandle),
}
```

Important: `MaterialSource` is not `pulith-source` vocabulary. It is the core request vocabulary. Concrete source-planning crates may implement conversion into it or resolution from it.

### `ApplicationTarget`

```rust
pub struct ApplicationTarget {
    pub root: PathBuf,
    pub activation: Option<ActivationTarget>,
}
```

### `ApplicationOperation`

```rust
pub struct ApplicationOperation {
    pub mode: ApplicationMode,
    pub rollback: RollbackRequirement,
}
```

### `ApplicationRequirements`

```rust
pub struct ApplicationRequirements {
    pub connectivity: ConnectivityRequirement,
    pub writable_scope: WritableScopeRequirement,
    pub activation: ActivationRequirement,
    pub integrity: IntegrityRequirement,
}
```

### `EvidencePolicy`

```rust
pub struct EvidencePolicy {
    pub retain_material: RetainMaterial,
    pub retain_provenance: bool,
    pub retain_lifecycle: bool,
    pub inspectable: bool,
    pub rollbackable: bool,
    pub repairable: bool,
}
```

### `ResourceApplicationReceipt`

```rust
pub struct ResourceApplicationReceipt {
    pub resource: ResourceId,
    pub target: ApplicationTargetReceipt,
    pub operation: OperationReceipt,
    pub evidence: ApplicationEvidence,
}
```

## Proposed core traits

### `MaterialResolver`

```rust
pub trait MaterialResolver {
    type Error;

    fn resolve(&self, request: MaterialResolveRequest) -> Result<MaterialHandle, Self::Error>;
}
```

Responsibility:

```text
resource + source -> material handle
```

Concrete implementers may use local paths, HTTP, git, caches, mirrors, etc.

### `MaterialProver`

```rust
pub trait MaterialProver {
    type Error;

    fn prove(&self, request: MaterialProveRequest) -> Result<MaterialEvidence, Self::Error>;
}
```

Responsibility:

```text
material handle + requirements -> evidence
```

Concrete implementers may compute digests, verify signatures, check sizes, detect shape, etc.

### `MaterialShaper`

```rust
pub trait MaterialShaper {
    type Error;

    fn shape(&self, request: MaterialShapeRequest) -> Result<PreparedMaterial, Self::Error>;
}
```

Responsibility:

```text
material handle + evidence + target shape -> prepared material
```

Concrete implementers may pass through files, extract archives, copy trees, generate wrappers, etc.

### `MaterialMemory`

```rust
pub trait MaterialMemory {
    type Error;

    fn remember(&self, request: RememberMaterialRequest) -> Result<RememberedMaterial, Self::Error>;
}
```

Responsibility:

```text
prepared material + evidence policy -> remembered material/evidence
```

Concrete implementers may store artifacts, write metadata, update lifecycle records, or operate in no-store mode.

### `ResourceApplier`

```rust
pub trait ResourceApplier {
    type Error;

    fn apply(&self, request: ApplyMaterialRequest) -> Result<ApplicationReceipt, Self::Error>;
}
```

Responsibility:

```text
remembered/prepared material + target + operation -> applied resource
```

Concrete implementers may stage, commit, activate, replace, rollback, update state.

### `ResourceApplicationEngine`

```rust
pub trait ResourceApplicationEngine {
    type Error;

    fn apply(&self, request: ResourceApplication) -> Result<ResourceApplicationReceipt, Self::Error>;
}
```

This trait is only composition of the atoms. It should not become the only implementation path.

## How current crates map after redesign

Current crates stop being architecture authority. They become implementation providers.

| Current crate | Future role |
| --- | --- |
| `pulith-resource` | candidate source of `ResourceSubject` fields, possibly folded into `pulith-core` |
| `pulith-version` | candidate `VersionIntent` implementation, likely fold into core unless independently valuable |
| `pulith-source` | possible `MaterialResolver` planning implementation, not primary vocabulary owner |
| `pulith-fetch` | concrete `MaterialResolver` implementation for remote/local byte acquisition |
| `pulith-archive` | concrete `MaterialShaper` implementation for archive-to-tree shaping |
| `pulith-store` | concrete `MaterialMemory` implementation |
| `pulith-state` | concrete evidence/lifecycle persistence implementation |
| `pulith-install` | concrete `ResourceApplier` implementation |
| `pulith-fs` | internal utility implementation, not a public workflow concept |

Some of these crates may be deleted/folded once their implementation role is small enough.

## Important reduction implication

After `pulith-core` exists, the old public APIs should not be preserved as peers.

Migration should deliberately delete or demote old surfaces:

```text
RequestedResource / ResolvedResource split if it complicates main-task request
SourceSpec / PlannedSources as public workflow requirement if core MaterialSource covers it
FetchReceipt as caller-stitching object unless it is inside ApplicationEvidence
ArchiveReport as caller-stitching object unless it is inside MaterialEvidence
StoreKey as manual caller requirement unless evidence policy explicitly chooses material retention identity
InstallSpec / PlannedInstall type-state chain as the only common path
Into* conversion traits that exist only to smooth current cross-crate glue
```

The concrete implementation may keep internal equivalents, but the main public API should not require the caller to pass through them.

## What the first implementation slice should be

Do **not** fold all crates immediately.

First code slice should be minimal and structural:

```text
Create crates/pulith-core with behavior contracts only.
```

No concrete fetch/archive/store/install logic in the first slice.

Initial files:

```text
crates/pulith-core/Cargo.toml
crates/pulith-core/src/lib.rs
crates/pulith-core/src/application.rs
crates/pulith-core/src/material.rs
crates/pulith-core/src/evidence.rs
crates/pulith-core/src/operation.rs
crates/pulith-core/src/traits.rs
```

Dependencies should be minimal:

```text
serde
thiserror? only if needed
url? only if MaterialSource::RemoteUrl is concrete
```

Prefer avoiding `url` in the first slice if it makes source too concrete. A string/newtype may be enough until source handling is designed.

## First slice acceptance test

The first slice is successful if it compiles and expresses the workflow without using any concrete implementation crate.

Expected check:

```bash
cargo check -p pulith-core --all-features
```

Structural checks:

```text
pulith-core must not depend on pulith-fetch/pulith-archive/pulith-store/pulith-install/pulith-state/pulith-fs.
pulith-core exposes ResourceApplication and atom traits.
No old crate imports core yet unless explicitly part of the slice.
```

## Second slice

After `pulith-core` exists, choose one implementation adapter to prove the contract.

Do not choose by behavior type like archive first. Choose the smallest proof of abstraction:

```text
local existing tree/file -> apply to target -> evidence receipt
```

This avoids fetch/archive format details and tests the behavior spine.

Likely adapter owner:

```text
pulith-install implements ResourceApplier for a core PreparedMaterial/RememberedMaterial shape
```

or a temporary example adapter under `examples/` if core traits need another design pass.

## Deletion strategy after proof

Once one adapter proves the core contracts, start deleting old architecture authority in this order:

1. Demote old crate docs from architecture authority to implementation provider docs.
2. Move resource/version request vocabulary into core if caller-facing.
3. Move install request/receipt vocabulary into core if caller-facing.
4. Replace cross-crate `Into*` glue with core request/receipt conversion only where needed.
5. Delete old public helpers that merely expose implementation sequencing.
6. Fold crates whose remaining role is only a tiny implementation detail.

No compatibility shims.

## Open decisions

1. Should final user-facing crate be named `pulith` while the contract crate is temporarily `pulith-core`?
2. Should `ResourceSubject` reuse current `ResourceId`/version types or define new core-owned types first?
3. Should traits be sync in the first slice, async, or generic over future? Recommendation: sync traits first unless a concrete implementer requires async; avoid choosing runtime in core.
4. Should material/source/evidence types own paths directly? Recommendation: yes for local material handles, but keep source transport abstract.
5. Should state/rollback/repair evidence be in core immediately? Recommendation: define the evidence policy and receipt envelope now; keep concrete state persistence out.

## Current recommendation

Accept the user's proposal.

Next work should be:

```text
1. Treat the current crate architecture as cancelled for public API design.
2. Create `pulith-core` as a behavior-contract crate.
3. Define `ResourceApplication` and atom traits before touching concrete fetch/archive/store/install implementations.
4. Keep concrete crates as implementation providers only.
5. After one adapter proves the contract, delete/demote old public cross-crate workflow APIs with no compatibility layer.
```

This is a better top-down route because it defines behavior first, then lets concrete libraries implement behavior, instead of defining concrete crates first and trying to recover a humane workflow from their aggregation.
