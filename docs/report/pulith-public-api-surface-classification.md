# Pulith Public API Surface Classification

## Status

Design/classification report only. No Rust code changes are authorized by this report.

This report performs Stage 1 from `docs/report/pulith-single-crate-migration-plan.md`: classify current public workflow surfaces against the new single-crate `pulith` spine.

Target spine:

```text
Application -> Acquire -> Prepare -> Apply -> Receipt
```

Classification buckets:

```text
Application field
Acquire implementation
Prepare implementation
Apply implementation
Receipt/Evidence field
Internal helper
Delete
```

Compatibility is not a constraint. Old public APIs should not be preserved as peer surfaces unless they remain the best final vocabulary.

## Classification rule

A type belongs in the public `pulith` API only if it answers a user-level question:

```text
what resource?
from what source?
to what target?
with what operation/requirements/evidence policy?
what happened?
```

A type belongs behind a feature module if it answers an implementation question:

```text
how to acquire material?
how to prepare material?
how to apply material?
how to persist evidence?
```

A type should be deleted if it exists mainly to stitch old crates together.

## Summary by bucket

### Application field

These concepts belong in `pulith::Application`, `Resource`, `Source`, `Target`, `Operation`, `Requirements`, or `EvidencePolicy`.

| Current surface | Current crate | Target | Action |
|---|---|---|---|
| `ResourceId` | `pulith-resource` | `pulith::Resource` | Absorb. |
| `VersionSelector` | `pulith-resource` | `pulith::Resource.version` or `Requirements.version` | Absorb as simpler version intent. |
| `ResolvedVersion` | `pulith-resource` | `Receipt/Evidence` after acquisition or resolution | Do not keep as caller-required state. |
| `ResourceLocator` | `pulith-resource` | `pulith::Source` | Absorb only active variants. |
| `ResolvedLocator` | `pulith-resource` | `Acquire` evidence or selected `Source` | Do not expose as workflow checkpoint. |
| `VerificationRequirement` | `pulith-resource` | `EvidencePolicy` / `Requirements` | Absorb. |
| `TrustMode` | `pulith-resource` | `EvidencePolicy` | Absorb if still needed. |
| `TrustAnchor` | `pulith-resource` | `EvidencePolicy` | Absorb if still needed. |
| `TrustPolicy` | `pulith-resource` | `EvidencePolicy` | Absorb with less ceremony. |
| `TrustDecision` | `pulith-resource` | `Receipt/Evidence` | Do not require caller branching. |
| `ArtifactForm` | `pulith-resource` | `Requirements` or `Prepare` selector | Absorb only if caller truly chooses form. |
| `UnpackPolicy` | `pulith-resource` | `Requirements` / `Prepare` config | Absorb into prepare requirement. |
| `MaterializationSpec` | `pulith-resource` | `Requirements` | Absorb and shrink. |
| `ActivationModel` | `pulith-resource` | `Operation` / `Target` | Absorb into apply semantics. |
| `MutationScope` | `pulith-resource` | `Operation` | Absorb. |
| `ProvenanceRequirement` | `pulith-resource` | `EvidencePolicy` | Absorb. |
| `LifecycleRequirements` | `pulith-resource` | `Requirements` | Absorb. |
| `ResourceBehaviorContract` | `pulith-resource` | `Application` plus `Requirements` | Likely split and delete wrapper. |
| `ResourceSpec` | `pulith-resource` | `Application.resource` | Absorb. |
| `SourceDefinition` | `pulith-source` | `Source` | Absorb active variants. |
| `SourceSet` | `pulith-source` | `Source::Candidates` or `Acquire` config | Absorb only if candidates remain user-visible. |
| `SelectionStrategy` | `pulith-source` | `Requirements` or `Acquire` config | Absorb as acquisition policy if needed. |
| `InstallMode` | `pulith-install` | `Operation.mode` | Absorb. |
| `ConnectivityMode` | `pulith-install` | `Requirements.network` | Absorb. |
| `ActivationSupport` | `pulith-install` | `Requirements` / `Target` capability | Absorb if caller chooses it. |
| `RollbackSupport` | `pulith-install` | `Requirements` / `EvidencePolicy` | Absorb if caller chooses it. |
| `InstallWritableScope` | `pulith-install` | `Operation` or `Target` | Absorb only if still meaningful. |
| `UninstallOptions` | `pulith-install` | future `Operation::Uninstall` | Absorb later, not first slice. |
| `UninstallDisposition` | `pulith-install` | future `Operation::Uninstall` | Absorb later. |

### Acquire implementation

These concepts belong to acquisition behavior. They may become implementation structs under `pulith::local` or `pulith::net`, but not public workflow checkpoints.

| Current surface | Current crate | Target | Action |
|---|---|---|---|
| `HttpAssetSource` | `pulith-source` | `Source::Remote` or net acquire config | Absorb only if needed. |
| `MirrorSource` | `pulith-source` | `Source::Candidates` / net acquire config | Absorb or delete if over-modeled. |
| `LocalSource` | `pulith-source` | `Source::LocalPath` | Already covered by skeleton. |
| `GitSource` | `pulith-source` | future feature module, maybe `git` | Do not add until needed. |
| `RemoteSource` | `pulith-source` | `Source::Remote` | Absorb as enum if active. |
| `SourcePlan<Unplanned>` / `SourceSpec` | `pulith-source` | `Application.source` or `Acquire` config | Delete typestate wrapper unless it removes real branching. |
| `SourcePlan<Planned>` / `PlannedSources` | `pulith-source` | `Acquire` internal selected candidates | Do not keep as user-main API. |
| `ResolvedSourceCandidate` | `pulith-source` | `Acquire` evidence | Internal/evidence, not caller stitching. |
| `SourceAdapter` | `pulith-source` | `Acquire` implementation trait if still needed | Prefer `Acquire`; delete duplicate trait. |
| `PassthroughAdapter` | `pulith-source` | local acquire or identity helper | Delete if redundant with `LocalAcquire`. |
| `Fetcher` | `pulith-fetch` | net `Acquire` implementation | Fold behind `net` feature. |
| `MultiSourceFetcher` | `pulith-fetch` | net `Acquire` implementation | Fold; do not expose as workflow default. |
| `FetchSource` | `pulith-fetch` | `Source` or acquire evidence | Absorb/delete duplication. |
| `FetchReceipt` | `pulith-fetch` | `Evidence::Acquired` | Receipt/evidence, not caller-required object. |
| `FetchOptions` | `pulith-fetch` | `Requirements` or net acquire config | Split: user policy to `Requirements`, mechanics internal. |
| `RetryPolicy` | `pulith-fetch` | net acquire config | Internal unless caller explicitly chooses retry. |
| `FetchPhase` | `pulith-fetch` | progress/evidence detail | Internal/evidence. |
| `BatchFetcher` | `pulith-fetch` | separate batch acquire implementation | Not first migration. |
| `SegmentedFetcher` | `pulith-fetch` | net acquire implementation detail | Internal. |
| `ResumableFetcher` | `pulith-fetch` | net acquire implementation detail | Internal. |
| `ConditionalFetcher` | `pulith-fetch` | net acquire implementation detail | Internal. |
| `HttpClient` | `pulith-fetch` | net adapter trait | Internal leaf trait, not main API. |
| `ReqwestClient` if exported | `pulith-fetch` | net feature implementation | Feature export only if users inject clients. |
| `RemoteMetadata` | `pulith-fetch` | acquire evidence | Evidence/internal. |
| `ConditionalOptions` | `pulith-fetch` | net acquire config | Internal/advanced. |

### Prepare implementation

These concepts belong to material shaping, especially archive extraction. They should become `Prepare` implementations or prepare evidence.

| Current surface | Current crate | Target | Action |
|---|---|---|---|
| `ArchiveFormat` | `pulith-archive` | archive prepare config/evidence | Fold behind `archive` feature. |
| `TarCompress` | `pulith-archive` | archive implementation detail | Internal. |
| `Decoder` | `pulith-archive` | archive implementation detail | Internal. |
| `ExtractOptions` | `pulith-archive` | prepare config / `Requirements` | Absorb only user-facing safety knobs. |
| `HashStrategy` | `pulith-archive` | `EvidencePolicy` / `hash` feature | Fold, do not separate crate. |
| `PermissionStrategy` | `pulith-archive` | prepare config | Keep only if caller chooses it. |
| `PermissionResolution` | `pulith-archive` | prepare evidence | Evidence/internal. |
| `SanitizedPath` | `pulith-archive` | archive implementation detail | Internal safety type. |
| `Entry` | `pulith-archive` | archive implementation detail | Internal. |
| `EntryKind` | `pulith-archive` | archive implementation detail | Internal or evidence detail. |
| `ArchiveReport` | `pulith-archive` | `Evidence::Prepared` | Keep as evidence shape, not caller stitching object. |
| `EntrySource` | `pulith-archive` | archive implementation trait | Internal; do not duplicate `Prepare`. |
| `PendingEntry` | `pulith-archive` | archive implementation detail | Internal. |
| `Extracted` | `pulith-archive` | `Prepared` material | Fold/rename if useful. |
| `WorkspaceExtraction` | `pulith-archive` | prepare result/evidence | Fold into `Prepared`/evidence if needed. |
| `ZipSource` | `pulith-archive` | archive implementation detail | Internal. |
| `TarSource` | `pulith-archive` | archive implementation detail | Internal. |
| `CompressionType` | `pulith-fetch` codec | prepare/acquire transform detail | Internal unless exposed as source requirement. |
| `StreamTransform` | `pulith-fetch` codec | prepare/acquire helper | Internal. |
| codec decoder types | `pulith-fetch` codec | implementation detail | Internal. |

### Apply implementation

These concepts belong to target mutation and activation. They should implement `Apply`, not dominate the public API.

| Current surface | Current crate | Target | Action |
|---|---|---|---|
| `InstallReady` | `pulith-install` | apply environment/context | Internal builder or delete. |
| `InstallInput` | `pulith-install` | `Prepared` generic input to `Apply` | Delete public enum after prepared material carries shape. |
| `IntoInstallInput` | `pulith-install` | obsolete conversion glue | Delete. |
| `InstallSpec` | `pulith-install` | `Application` | Absorb; do not preserve name unless final vocabulary chooses it. |
| `InstallPlanningRequest` | `pulith-install` | `Requirements` / apply planning internals | Split; mostly internal. |
| `InstallCapabilities` | `pulith-install` | `Requirements` / target capability evidence | Absorb if caller-facing. |
| `InstallWorkflowVariant` | `pulith-install` | internal apply branch | Delete or internal. |
| `InstallPlanLimitation` | `pulith-install` | plan/evidence detail | Receipt/Evidence if user needs diagnostics. |
| `InstallPlanReport` | `pulith-install` | apply plan diagnostics | Receipt/Evidence or internal. |
| `InstallFlow<S>` typestate | `pulith-install` | internal apply sequence | Internal; not main path. |
| `PlannedInstall` | `pulith-install` | internal apply plan | Internal. |
| `StagedInstall` | `pulith-install` | internal apply state | Internal. |
| `InstalledInstall` | `pulith-install` | internal apply state | Internal. |
| `ActivatedInstall` | `pulith-install` | internal apply state | Internal. |
| `ActivationTarget` | `pulith-install` | `Target` / `Operation.activation` | Absorb. |
| `ActivationRequest` | `pulith-install` | apply implementation input | Internal. |
| `Activator` | `pulith-install` | apply implementation trait | Internal or feature-local advanced trait. |
| `SymlinkActivator` | `pulith-install` | apply implementation | Feature implementation. |
| `CopyFileActivator` | `pulith-install` | apply implementation | Feature implementation; skeleton already has simple copy. |
| `ShimLinkActivator` | `pulith-install` | apply implementation | Internal/feature implementation. |
| `ShimCopyActivator` | `pulith-install` | apply implementation | Internal/feature implementation. |
| `ShimCommand` | `pulith-install` | apply implementation detail | Internal. |
| `InstalledShimResolver` | `pulith-install` | apply implementation detail | Internal. |
| `BackupReceipt` | `pulith-install` | receipt/evidence | Fold into `Receipt` evidence. |
| `RestoreReceipt` | `pulith-install` | receipt/evidence | Fold into `Receipt` evidence. |
| `RollbackReceipt` | `pulith-install` | receipt/evidence | Fold into `Receipt` evidence. |
| `UninstallReceipt` | `pulith-install` | receipt/evidence | Fold into `Receipt` evidence. |
| `ActivationReceipt` | `pulith-install` | receipt/evidence | Fold into `Receipt` evidence. |
| `InstallReceipt` | `pulith-install` | `Receipt` | Absorb. |
| `LifecycleOperationReceipt` | `pulith-install` | `Receipt` / evidence event | Absorb. |
| `LifecycleOperationPhase` | `pulith-install` | receipt/evidence enum | Absorb if useful. |
| `LifecycleOperationDetails` | `pulith-install` | receipt/evidence detail | Absorb if useful. |

### Receipt / Evidence field

These concepts should survive as facts, not as mandatory workflow steps.

| Current surface | Current crate | Target | Action |
|---|---|---|---|
| `ValidDigest` | `pulith-resource` | `Evidence::Digest` | Absorb behind `hash` feature if needed. |
| `DigestAlgorithm` | `pulith-resource` | `Evidence::Digest` / `EvidencePolicy` | Absorb. |
| `Metadata` / `Labels` | `pulith-resource` | `Resource` / `Receipt` metadata | Absorb as plain maps or delay. |
| `ArtifactDescriptor` | `pulith-resource` | `Receipt` / prepared material metadata | Absorb only if used. |
| `Progress` | `pulith-archive` | progress callback/evidence | Internal until needed. |
| `Progress` / `ExtendedProgress` / `ProgressSnapshot` | `pulith-fetch` | progress surface | Internal/advanced, not first API. |
| `PerformanceReport` / perf structs | `pulith-fetch` | diagnostics | Internal or feature diagnostics. |
| `StoreProvenance` | `pulith-store` | `Evidence` | Absorb as evidence facts. |
| `StoreMetadataRecord` | `pulith-store` | persisted receipt/evidence | Fold into persist module. |
| `StoredArtifact` | `pulith-store` | remembered/prepared material evidence | Fold into receipt/persist. |
| `ExtractedArtifact` | `pulith-store` | prepared material evidence | Fold into prepare/apply path. |
| `ResourceInspectionReport` | `pulith-state` | inspect query result | Keep as future inspect view, not main apply path. |
| `ResourceInspectionFinding` | `pulith-state` | inspect evidence | Absorb later. |
| `ResourceInspectionSummary` | `pulith-state` | inspect evidence | Absorb later. |
| `ResourceRepairPlan` | `pulith-state` | repair operation plan | Future operation/evidence, not first migration. |
| `ResourceRepairAction` | `pulith-state` | repair operation detail | Future operation/evidence. |
| `ActivationOwnershipReport` | `pulith-state` | inspect/evidence query | Future inspect module. |
| `Ownership*` types | `pulith-state` | inspect/evidence details | Fold only when inspect migrates. |
| `LockFile` | `pulith-state::lock` | persisted evidence snapshot | Fold into persist/evidence if still needed. |
| `LockedResource` | `pulith-state::lock` | persisted resource evidence | Fold. |
| `LockDiff` / `LockResourceChange` | `pulith-state::lock` | evidence diff diagnostics | Fold into inspect/persist if needed. |

### Internal helper

These are useful mechanics but should not be public workflow nouns.

| Current surface | Current crate | Target | Action |
|---|---|---|---|
| `pulith-fs::Workspace` | `pulith-fs` | internal temp/workspace helper | Fold into internal module if needed. |
| `WorkspaceReport` | `pulith-fs` | diagnostics/evidence if needed | Internal. |
| `Transaction` | `pulith-fs` | internal atomic mutation helper | Fold into apply/persist internals. |
| `PermissionMode` | `pulith-fs` | internal or prepare/apply config | Avoid public unless user chooses it. |
| `hardlink` primitives/options | `pulith-fs` | apply implementation detail | Internal. |
| `replace_dir` primitives/options | `pulith-fs` | apply implementation detail | Internal. |
| `atomic_write` / read helpers | `pulith-fs` | persist/apply internals | Internal. |
| `AlignedBuf` | `pulith-fs` | internal perf helper | Delete unless proven used. |
| `Resource<'a>` / `Content` in `pulith-fs` | `pulith-fs` | possible internal helper | Rename if retained; conflicts with `pulith::Resource`. |
| `Signature*` types | `pulith-fetch` codec | optional trust/hash implementation | Internal behind feature until policy requires it. |
| `TokenBucket`, throttling types | `pulith-fetch` | net internal | Internal. |
| batch/resume/segment/checkpoint types | `pulith-fetch` | net internal | Internal unless explicit advanced API later. |
| archive path sanitization helpers | `pulith-archive` | archive internal | Internal safety helpers. |
| `StoreRoots` | `pulith-store` | persist module config | Internal/advanced. |
| `StoreReady` | `pulith-store` | persist/apply context | Internal. |
| `StoreKey` | `pulith-store` | persisted evidence key | Internal; caller should not reconstruct. |
| `KeyDerivation` | `pulith-store` | persist internal strategy | Internal. |
| `StateReady` | `pulith-state` | persist/inspect context | Internal. |
| `StateAnalysisIndex` | `pulith-state` | inspect internal index | Internal. |

### Delete

These are glue, typestate ceremony, or obsolete public surfaces that should not be recreated in `pulith`.

| Current surface | Current crate | Reason |
|---|---|---|
| `IntoArtifactRegistration` | `pulith-store` | Conversion glue around old store API. |
| `IntoExtractRegistration` | `pulith-store` | Conversion glue; already moving away from tuple protocols. |
| `ExtractRegistration` | `pulith-store` | Store-internal construction detail after `Prepare -> Apply` carries prepared material. |
| `ExtractedTreeRegistration` | `pulith-store` | Transitional improvement, delete after direct pipeline exists. |
| `IntoResourceUpsert` | `pulith-state` | Conversion glue around old state API. |
| `Requested` / `Resolved` typestate markers | `pulith-resource` | Preserve only if classification proves typestate removes real bugs; otherwise delete. |
| `RequestedResource` / `ResolvedResource` split | `pulith-resource` | Likely old caller branching surface; replace with `Application` plus evidence. |
| `ResolvedResourceContext` | `pulith-resource` | Old aggregation context; fold facts into receipt/evidence. |
| `SourcePlan<Unplanned>` / `SourcePlan<Planned>` typestate | `pulith-source` | Over-modeled planning surface unless a concrete branch requires it. |
| `InstallFlow<S>` typestate markers | `pulith-install` | Common caller path should not stage manually. |
| `InstallInput` public enum | `pulith-install` | Prepared material should be typed through `Prepare` and `Apply`, not caller-selected enum. |
| `IntoInstallInput` | `pulith-install` | Conversion glue. |
| compatibility aliases/reexports | all old crates | No downstream compatibility requirement. |

## Stage 2 minimal migration target

The next code slice should not port all classified types. It should only make `pulith::Application` expressive enough for the next local-only migrated example.

Recommended additions to `crates/pulith/src/application.rs`:

```text
Resource.id stays String for now; do not import ResourceId yet.
Resource.version stays Option<String> for now; delay version semantics.
Source stays LocalPath for now; add Remote only when net acquire migrates.
OperationMode can grow from CreateOrReplace to CreateOnly/ReplaceExisting if needed by local tests.
Requirements can add network/trust/material-shape only when a migrated caller needs it.
EvidencePolicy retain bool is enough for current skeleton; do not port TrustPolicy yet.
```

So Stage 2 code should be even smaller than this classification table suggests.

## Migration order by deletion value

1. Keep `pulith` independent.
2. Migrate one local-path example/test to import only `pulith`.
3. Delete or stop using old local-path public surfaces in that migrated path.
4. Fold minimal `pulith-resource`/`pulith-source` vocabulary only when needed.
5. Fold archive as `Prepare`.
6. Fold net acquisition as `Acquire`.
7. Fold persist/state as `Receipt`/evidence storage.
8. Fold install lifecycle as `Apply`.
9. Remove old crates after all active importers are gone.

## Do-not-port list for the next slice

Do not port these yet:

```text
TrustPolicy
SourcePlan typestate
FetchReceipt shape
ArchiveReport shape
StoreKey
InstallSpec
InstallFlow typestate
State inspection/repair reports
Signature verification
Batch/segmented/resumable fetch
```

They may contain useful semantics later, but porting them now would recreate the old API mass inside `pulith`.

## Verification checklist for this classification

This report is healthy if:

- every old public workflow surface has a target bucket;
- the next code slice is smaller than the full table;
- old conversion traits and typestate workflow surfaces are marked for deletion or internal use;
- `pulith` remains independent from old crates;
- feature modules are described by capability, not old crate names.
