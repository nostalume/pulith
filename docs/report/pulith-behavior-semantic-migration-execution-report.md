# Pulith Behavior-Semantic Migration Execution Report

## Status

Executed against the active Cargo workspace.

This report follows:

```text
docs/report/pulith-behavior-morphism-spec.md
docs/report/pulith-behavior-semantic-migration-plan.md
```

## Execution decision

The migration plan was executed by making `pulith` the only active workspace crate and moving the active API to behavior-defined semantic states.

This intentionally does not preserve compatibility with old public crates.

Old crate directories are not used by the active workspace after this execution. They can be physically deleted in a later cleanup slice after any desired manual review of uncommitted historical files.

## Active workspace result

`Cargo.toml` now contains:

```text
members = ["crates/pulith"]
```

Verified by `cargo metadata`:

```text
PASS active workspace contains only pulith
```

This demotes old crate-shaped surfaces from active API:

```text
pulith-archive
pulith-fetch
pulith-fs
pulith-install
pulith-resource
pulith-source
pulith-state
pulith-store
pulith-version
examples/pulith-backend-example
examples/runtime-manager
```

## Code execution by phase

### Phase 0 — Freeze behavior contract

Executed.

Accepted docs:

```text
docs/report/pulith-behavior-morphism-spec.md
docs/report/pulith-behavior-semantic-migration-plan.md
```

### Phase 1 — Declare/Offer semantics

Executed in `crates/pulith/src/application.rs` and `crates/pulith/src/behavior.rs`.

Active semantic vocabulary:

```text
App
Item
Source
Target
Op
OpMode
Need
VerifyNeed
PrepareNeed
EvidencePolicy
Declared
Offered
```

Old wrappers are not present in the active `pulith` crate:

```text
ResourceSpec
ResourceBehaviorContract
RequestedResource
ResolvedResource
SourcePlan
SourceSpec
```

### Phase 2 — Select semantics

Executed in `crates/pulith/src/behavior.rs` and `crates/pulith/src/local.rs`.

Active semantic state:

```text
Chosen
```

Active behavior trait:

```text
Select
```

The active path chooses from `Offered` into `Chosen`; it does not fetch or prepare during selection.

### Phase 3 — Acquire/Verify semantics

Executed in `crates/pulith/src/behavior.rs`, `crates/pulith/src/evidence.rs`, and `crates/pulith/src/local.rs`.

Active semantic states:

```text
Acquired<M>
Verified<M>
```

Active behavior traits:

```text
Acquire
Verify
```

Old active glue is absent from the new crate:

```text
FetchSource
FetchReceipt
MultiSourceFetcher
```

Local verification behavior is explicit:

```text
VerifyNeed::NotRequired => identity verification evidence
VerifyNeed::Required => error
VerifyNeed::Digest => error until a digest implementation is attached
```

This avoids pretending verification exists before implementation exists.

### Phase 4 — Prepare semantics

Executed.

Active semantic state:

```text
Prepared<P>
```

Active behavior trait:

```text
Prepare
```

Local implementation validates requested shape:

```text
PrepareNeed::Identity
PrepareNeed::Directory
PrepareNeed::File
```

Old active glue is absent from the new crate:

```text
ArchiveReport
ExtractedTreeRegistration
ExtractRegistration
IntoExtractRegistration
```

### Phase 5 — Apply semantics

Executed.

Active semantic state:

```text
Applied
```

Active behavior trait:

```text
Apply
```

Active operation modes:

```text
Create
Replace
CreateOrReplace
Forget
```

Old active choreography is absent from the new crate:

```text
InstallSpec
InstallInput
IntoInstallInput
InstallFlow<S>
PlannedInstall
StagedInstall
InstalledInstall
ActivatedInstall
```

Apply is the target mutation boundary in the active code.

### Phase 6 — Remember semantics

Executed.

Active semantic state:

```text
Remembered
```

Active behavior trait:

```text
Remember
```

Old active store glue is absent from the new crate:

```text
StoreKey
IntoArtifactRegistration
IntoExtractRegistration
IntoResourceUpsert
StateReady
```

Remember currently records retained evidence in memory. Persistent storage is not faked.

### Phase 7 — Inspect/Repair/Forget semantics

Executed.

Active semantic states:

```text
Observed
RepairPlan
Forgotten
```

Active behavior traits:

```text
Inspect
Repair
Forget
```

Inspect observes target presence without mutation.

Repair produces a plan and does not mutate.

Forget explicitly removes target state and emits forget evidence.

### Phase 8 — Implementation attachment

Executed for the local implementation path.

Active local implementation:

```text
LocalEngine
```

`LocalEngine` implements:

```text
Declare
Offer
Select
Acquire
Verify
Prepare
Apply
Remember
Inspect
Repair
Forget
```

`LocalEngine::run` executes:

```text
Declare -> Offer -> Select -> Acquire -> Verify -> Prepare -> Apply -> Remember
```

Inspect/Repair/Forget remain explicit post-apply behaviors.

## Wheel-repetition/deletion decisions

Deleted from the active `pulith` crate:

```text
crates/pulith/src/pipeline.rs
```

Reason:

```text
The previous Acquire -> Prepare -> Apply pipeline was an implementation-shaped shortcut.
It duplicated the behavior graph after the DDD behavior-morphism migration introduced explicit Declare/Offer/Select/Verify/Remember/Inspect/Repair/Forget relations.
```

Not physically deleted yet:

```text
old pulith-* crate directories
old example directories
```

Reason:

```text
They are no longer active workspace members, but the repository already had many uncommitted edits/deletes in those historical directories. Physical deletion should be a separate cleanup slice so it is easy to audit and does not erase historical work accidentally.
```

## Active absence checks

Search in `crates/pulith` found no active references to retired glue names:

```text
ResourceSpec
SourcePlan
FetchReceipt
ArchiveReport
ExtractedTreeRegistration
InstallFlow
InstallInput
StoreKey
IntoResourceUpsert
RequestedResource
ResolvedResource
```

## Verification run

Commands executed:

```text
cargo fmt --all --check
cargo check --workspace --all-features
cargo test --workspace --all-features
```

Result:

```text
Finished `dev` profile
Finished `test` profile
running 5 tests
5 passed; 0 failed
Doc-tests pulith: 0 passed; 0 failed
```

Additional checks:

```text
cargo metadata --no-deps --format-version 1
PASS active workspace contains only pulith
```

```text
git diff --check -- Cargo.toml Cargo.lock crates/pulith docs/report/pulith-behavior-semantic-migration-plan.md
exit 0
```

Note:

```text
git diff --check emitted a CRLF warning for Cargo.lock only; it did not fail.
```

## Current active API summary

Public semantic states/relations exported by `pulith`:

```text
App
Item
Source
Target
Op
OpMode
Need
VerifyNeed
PrepareNeed
EvidencePolicy
Declared
Offered
Chosen
Acquired
Verified
Prepared
Applied
Remembered
Observed
RepairPlan
Forgotten
Declare
Offer
Select
Acquire
Verify
Prepare
Apply
Remember
Inspect
Repair
Forget
Receipt
Evidence
EvidenceEvent
EvidenceKind
EvidenceDetail
LocalEngine
```

## Tests added/exercised

Active behavior tests:

```text
local_engine_runs_behavior_graph_for_file
local_engine_keeps_inspect_and_forget_explicit
local_engine_rejects_unavailable_verify_behavior
local_engine_rejects_wrong_prepare_shape
create_and_replace_modes_are_apply_laws
```

Coverage intent:

```text
behavior graph order
explicit Remember evidence
explicit Inspect/Forget behavior
Verify cannot be silently skipped when required
Prepare shape laws are enforced
Apply Create/Replace laws are enforced
```

## Remaining cleanup after this execution

The plan is executed for the active workspace. Remaining cleanup is repository hygiene, not active API migration:

```text
physically remove or archive old demoted pulith-* crate directories
physically remove or rewrite old examples against the new pulith API
prune old workspace dependencies if no historical crate needs them
update README/docs that still describe the old multi-crate workspace
```

Those are intentionally separated because they are destructive and touch many already-modified historical files.
