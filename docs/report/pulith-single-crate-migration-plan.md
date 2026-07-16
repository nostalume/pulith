# Pulith Single-Crate Migration Plan

## Status

Implementation plan after the first `pulith` skeleton slice.

The first slice has created the new single user-facing crate:

```text
crates/pulith
```

It intentionally does not depend on existing `pulith-*` crates. The old crates remain only as migration material.

## First slice implemented

New files:

```text
crates/pulith/Cargo.toml
crates/pulith/src/lib.rs
crates/pulith/src/application.rs
crates/pulith/src/pipeline.rs
crates/pulith/src/evidence.rs
crates/pulith/src/error.rs
crates/pulith/src/local.rs
```

Workspace change:

```text
Cargo.toml adds "crates/pulith" as a workspace member.
```

Current `pulith` public API skeleton:

```text
Application
Resource
Source
Target
Operation
OperationMode
Requirements
EvidencePolicy
Receipt
Evidence
Acquire
Prepare
Apply
Pipeline
```

Feature-gated local proof exports:

```text
LocalAcquire
IdentityPrepare
TargetApply
PathMaterial
PreparedPath
LocalEvidence
PreparedEvidence
```

Current behavior proof:

```text
local source -> identity prepare -> target apply -> receipt
```

This is not a product workflow such as “install file”. It is the smallest proof that `Acquire -> Prepare -> Apply` composes inside one crate.

## Current repository dependency analysis

Fresh `cargo metadata --no-deps` after adding `pulith` reports:

```text
pulith: internal_deps=[]
pulith-archive: internal_deps=['pulith-fs']
pulith-fetch: internal_deps=['pulith-fs', 'pulith-resource', 'pulith-source', 'pulith-resource']
pulith-fs: internal_deps=[]
pulith-install: internal_deps=['pulith-archive', 'pulith-fetch', 'pulith-fs', 'pulith-resource', 'pulith-state', 'pulith-store', 'pulith-source']
pulith-resource: internal_deps=['pulith-version']
pulith-source: internal_deps=['pulith-resource']
pulith-state: internal_deps=['pulith-fs', 'pulith-resource', 'pulith-store']
pulith-store: internal_deps=['pulith-archive', 'pulith-fetch', 'pulith-fs', 'pulith-resource']
pulith-version: internal_deps=[]
```

Interpretation:

- `pulith-install` is the current aggregation hotspot.
- `pulith-store` already couples archive/fetch/fs/resource, so it is not a pure store boundary.
- `pulith-fetch` already couples resource/source/fs, so it is not just acquisition mechanics.
- `pulith-state` couples fs/resource/store, so lifecycle evidence is split across state/store/resource.
- `pulith` is currently clean and independent. Keep it that way while absorbing concepts.

## Public surface analysis

Current high-signal public surfaces:

```text
pulith-resource:
  ResourceSpec
  MaterializationSpec
  ActivationModel

pulith-fetch:
  FetchReceipt

pulith-archive:
  ArchiveReport

pulith-store:
  ExtractedTreeRegistration
  IntoArtifactRegistration
  IntoExtractRegistration

pulith-install:
  InstallInput
  InstallSpec
  InstallReceipt
  InstallPlanningRequest
  InstallPlanReport
  InstallMode
  ConnectivityMode

pulith-state:
  ResourceInspectionReport
```

These names prove the design problem: the real user workflow is fragmented into per-crate objects, then recomposed by callers/tests/examples.

The migration must not preserve those surfaces as peers. It should absorb useful semantics into `pulith::Application`, `pulith::Receipt`, feature modules, and atom implementations.

## Target architecture

One crate:

```text
pulith
```

One main behavior chain:

```text
Application -> Acquire -> Prepare -> Apply -> Receipt
```

Optional implementation modules behind features:

```text
local      existing file/tree material
net        remote acquisition
archive    archive shaping/extraction
persist    evidence and receipt retention
hash       digest evidence
```

Avoid feature names that simply mirror old crate identities. Features describe capability families, not legacy packages.

## Migration principle

For each old crate, choose one of three outcomes:

```text
absorb value types into pulith application/evidence modules
absorb implementation into a feature module
delete obsolete glue/shims
```

Do not create compatibility crates. Do not add re-export shims. Do not leave old public APIs as parallel normal paths.

## Stage 0 — skeleton lock-in

Already implemented.

Acceptance:

```bash
cargo check -p pulith --all-features
cargo test -p pulith local_application_pipeline
```

Structural gate:

```text
pulith has no dependency on old pulith-* crates.
```

## Stage 1 — classify old public APIs against the new spine

Write a source table that maps every public workflow type to one bucket:

```text
Application field
Acquire implementation
Prepare implementation
Apply implementation
Receipt/Evidence field
Internal helper
Delete
```

Initial classification:

| Old surface | New bucket | Note |
|---|---|---|
| `ResourceSpec` | `Application.resource` | Fold id/version/material hints into `Resource`/`Requirements`. |
| `MaterializationSpec` | `Requirements` or `Prepare` selector | Keep only if caller truly chooses material shape. |
| `ActivationModel` | `Operation` / `Target` | Activation is an apply concern. |
| `FetchReceipt` | `Evidence` from `Acquire` | Should not be a mandatory caller stitching object. |
| `ArchiveReport` | `Evidence` from `Prepare` | Should not require manual root/report pairing. |
| `ExtractedTreeRegistration` | transitional evidence glue | Delete after `Prepare -> Apply` carries prepared material. |
| `InstallInput` | `Prepared` input to `Apply` | Avoid caller-visible enum if pipeline selects it. |
| `InstallSpec` | `Application` + `Operation` | Collapse into top-level request. |
| `InstallReceipt` | `Receipt` | Merge lifecycle/evidence result. |
| `ResourceInspectionReport` | `Receipt`/persisted evidence query | Keep as inspect view, not main application path. |

Deliverable:

```text
docs/report/pulith-public-api-surface-classification.md
```

No code migration in this stage.

## Stage 2 — absorb resource/version/source into `Application`

Move only vocabulary, not behavior.

Target files:

```text
crates/pulith/src/application.rs
```

Likely additions:

```text
ResourceId
VersionRequirement / VersionIntent
Source variants for local path and candidate source
TargetKind or Target path semantics
Operation details for create/replace/activate
Requirements for offline/network/trust/material shape
```

Rules:

- Do not import `pulith-resource`, `pulith-version`, or `pulith-source` into `pulith`.
- Copy/reshape only the needed concepts after classification.
- Delete compatibility intent from the new API.
- Keep constructors ergonomic and small.

Verification:

```bash
cargo check -p pulith --all-features
cargo test -p pulith
```

## Stage 3 — migrate local/file behavior first

The current skeleton has a minimal local implementation. Expand it only enough to replace current local-path workflow tests.

Target module:

```text
crates/pulith/src/local.rs
```

Absorb from old crates only when directly needed:

```text
pulith-fs primitives -> internal helpers or stdlib replacement
pulith-resource local path locator -> Source::LocalPath
pulith-install local target application -> TargetApply behavior
```

Do not port archive/net/store/state yet.

Goal:

```text
one local example/test imports only pulith
```

Verification:

```bash
cargo test -p pulith local_application_pipeline
```

Migration gate:

```bash
grep -R "pulith_\(resource\|source\|install\|fs\)" examples/runtime-manager crates/pulith/tests || true
```

The first migrated example/test should not need old crates.

## Stage 4 — absorb archive as `Prepare`

Target module:

```text
crates/pulith/src/archive.rs
```

Feature:

```text
archive
```

Old concepts:

```text
ArchiveReport -> Prepare evidence
ExtractOptions -> Prepare config or Requirements field
extract_from_reader -> ArchivePrepare implementation
```

Rules:

- Archive is not a public workflow crate.
- Archive extraction is a `Prepare` implementation.
- Root + report must travel as one prepared/evidence product; no caller tuple protocol.

Verification:

```bash
cargo check -p pulith --features archive
cargo test -p pulith archive_prepare_pipeline
```

Deletion candidate after migration:

```text
ExtractedTreeRegistration
```

## Stage 5 — absorb remote acquisition as `Acquire`

Target module:

```text
crates/pulith/src/net.rs
```

Feature:

```text
net
```

Old concepts:

```text
Fetcher / MultiSourceFetcher -> Acquire implementation
FetchReceipt -> Acquire evidence
FetchOptions -> Requirements or NetAcquire config
PlannedSources / SelectionStrategy -> Source or Acquire config
```

Rules:

- Source selection is part of acquisition behavior, not a separate public workflow layer.
- `FetchReceipt` becomes evidence, not a caller stitching object.
- Keep runtime async decision explicit; do not hide a tokio runtime bridge in the main API unless that is the chosen sync API contract.

Verification:

```bash
cargo check -p pulith --features net
```

## Stage 6 — absorb evidence persistence/state as receipt/evidence modules

Target modules:

```text
crates/pulith/src/evidence.rs
crates/pulith/src/persist.rs
```

Feature:

```text
persist
```

Old concepts:

```text
StoreReady / StoreKey / StoreProvenance -> persistence implementation details
StateReady / ResourceRecord / inspection reports -> evidence query views
```

Rules:

- Caller should not manually construct store keys during the main workflow.
- Main `Receipt` should contain enough evidence refs for inspect/rollback/repair.
- Store/state internals may exist, but not as required public workflow crates.

Verification:

```bash
cargo check -p pulith --features persist
```

## Stage 7 — absorb apply/install lifecycle

Target modules:

```text
crates/pulith/src/target.rs
crates/pulith/src/evidence.rs
```

Old concepts:

```text
InstallSpec -> Application
InstallInput -> Prepared material input
PlannedInstall -> internal apply plan
InstallReceipt / LifecycleOperationReceipt -> Receipt
ActivationRequest / Activator -> Apply implementation detail or Target capability
```

Rules:

- `stage -> commit -> activate -> finish` can remain internally if needed, but the common path is `Pipeline::run` / `pulith::apply`.
- Keep advanced atom access if caller needs manual composition.
- No public compatibility shim named `InstallSpec` unless classification proves it is the correct final word, which currently it is not.

Verification:

```bash
cargo check -p pulith --all-features
cargo test -p pulith
```

## Stage 8 — migrate examples and delete old crates

Migrate examples/tests in small groups:

```text
examples/runtime-manager
examples/pulith-backend-example
workspace pipeline tests
benches only after behavior path is stable
```

For each old crate:

1. migrate all importers;
2. run focused check/test;
3. remove workspace member and crate directory;
4. grep for old crate names;
5. do not leave re-export compatibility crates.

Deletion order should follow dependency leaf/root shape:

```text
pulith-version / pulith-resource / pulith-source first after Application absorbs vocabulary
pulith-fs when local/target helpers are internal
pulith-archive after archive feature is live
pulith-fetch after net feature is live
pulith-store / pulith-state after persist/evidence is live
pulith-install last, because it is current aggregation hotspot
```

## Verification cadence

Use focused checks while migrating:

```bash
cargo check -p pulith --all-features
cargo test -p pulith <focused_test>
```

At meaningful milestones only:

```bash
cargo check --workspace --all-features
```

Do not run full suites after every small edit.

## Stop conditions

Pause and redesign if:

- `pulith` needs to depend on old `pulith-*` crates;
- more than three atom traits are needed before a concrete failure of `Acquire/Prepare/Apply`;
- a registry/factory/plugin manager appears;
- an old public type is preserved only for compatibility;
- feature names start matching old crate names rather than capabilities.

## Next concrete task

Write:

```text
docs/report/pulith-public-api-surface-classification.md
```

Then implement Stage 2 by moving only the smallest `Resource` / `Source` / `Target` / `Operation` vocabulary needed by the next local-only migrated example.
