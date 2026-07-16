# Pulith Behavior-Semantic Migration Plan

## Status

Design/migration plan only. No Rust code changes are authorized by this document.

This plan follows:

```text
docs/report/pulith-unified-atomic-architecture-analysis.md
docs/report/pulith-behavior-morphism-spec.md
```

The migration axis is behavior semantics, not crate boundaries, file layout, or examples.

## Core rule

```text
Migrate behavior-defined semantics first.
Then attach behavior implementations to those semantics.
```

Do not migrate by:

```text
old crate -> new module
old public type -> renamed public type
old example -> new API
old implementation -> copied implementation
```

Instead migrate by:

```text
behavior relation -> semantic state -> required facts -> evidence/laws -> implementation owner
```

## Target behavior graph

Analysis behavior names, not final API names:

```text
Declare -> Offer -> Select -> Acquire -> Verify -> Prepare -> Apply -> Remember -> Inspect -> Repair
                                                       \                         \-> Forget
                                                        \-> Remember(cache only)
```

Important constraints:

```text
Declare does not prove anything.
Offer does not obtain material.
Select does not obtain material.
Acquire does not verify or prepare.
Verify does not change material identity.
Prepare does not mutate target.
Apply is the target mutation boundary.
Remember persists facts but cannot invent lifecycle truth.
Inspect observes but does not mutate.
Repair/Forget are explicit behaviors, not hidden cleanup.
```

## Semantic states to migrate

| State | Incoming behavior | Outgoing behavior | Migration status |
|---|---|---|---|
| Declared intent | `Declare` | `Offer`, future desired-state `Remember` | Migrate first. |
| Offered source set | `Offer` | `Select`, identity `Acquire` | Migrate second. |
| Chosen source | `Select` | `Acquire` | Keep evidence/internal unless user-visible policy needs it. |
| Acquired material | `Acquire` | `Verify`, identity `Prepare` | Evidence-heavy, not caller glue. |
| Verified material | `Verify` | `Prepare` | Policy/evidence boundary. |
| Prepared material | `Prepare` | `Apply`, cache `Remember` | Must carry shape + evidence together. |
| Applied target | `Apply` | `Remember`, `Inspect`, `Repair`, `Forget` | Mutation receipt boundary. |
| Remembered fact | `Remember` | `Inspect`, `Repair`, `Forget` | Optional; not first executable path. |
| Observed state | `Inspect` | `Repair`, `Forget`, `Declare` | Later read-only behavior. |
| Repair plan | `Repair` | `Declare`, `Apply` | Later explicit behavior. |

## Complete old-surface behavior classification

### Declare — user intent to declared intent

Behavior semantics:

```text
Caller says what managed item should exist, where it may come from, where it may be applied, and under what requirements.
```

Migrate as behavior-defined semantics:

| Old surface | Semantic state/facts | Disposition |
|---|---|---|
| `Application` skeleton | Declared intent | Keep as current proof; final name may shorten. |
| `ResourceId` | Managed item identity | Keep semantics; avoid duplicating with fs `Resource`. |
| `ResourceSpec` | Declared item + source + requirements bundle | Split by behavior; do not preserve wrapper. |
| `ResourceBehaviorContract` | Aggregated need/policy axes | Delete wrapper; facts flow into declared intent/Need. |
| `VersionSelector` | Declared version intent | Keep only if behavior needs version selection. |
| `VersionRequirement` / `VersionPreference` / `SelectionPolicy` | Declared/selection policy facts | Internal to version selection until public need exists. |
| `InstallSpec` | Duplicate declaration + apply intent | Delete as public peer; map facts into declared intent + Apply need. |
| `InstallMode` / skeleton `OperationMode` | Operation intent | Collapse to one operation mode. |
| `ConnectivityMode` | Network need | Migrate as requirement if Offer/Acquire uses it. |
| `ActivationModel` / `ActivationTarget` | Target/apply intent | Collapse into Target/Apply semantics. |
| `MutationScope` / `InstallWritableScope` | Apply constraint | Migrate as Need only if behavior law needs it. |
| `LifecycleRequirements` | Replace/rollback/uninstall/repair needs | Split across Apply/Repair/Forget requirements. |
| `ProvenanceRequirement` | Remember/evidence need | Migrate only when Remember is implemented. |
| `Metadata` / `Labels` | Declared annotations or receipt annotations | Keep only as plain metadata; not behavior driver. |

Glue/delete:

```text
Requested / Resolved typestate markers
RequestedResource / ResolvedResource as public phases
ResolvedResourceContext
ResourceBehaviorContract wrapper
InstallSpec wrapper
```

Migration law:

```text
A declared intent must not be accepted as chosen source, acquired material, applied target, or remembered fact.
```

### Offer — declared intent to offered source set

Behavior semantics:

```text
Normalize possible sources from declared intent without choosing or obtaining one.
```

Migrate as behavior-defined semantics:

| Old surface | Semantic state/facts | Disposition |
|---|---|---|
| `ResourceLocator` | Source offer expression | Collapse into Offer semantics; not peer to Source. |
| `ValidUrl` | URL validation fact | Keep as validator; not a behavior state. |
| `SourceDefinition` | Offered source variant | Collapse into offered source set. |
| `RemoteSource` / `LocalSource` | Offer variants | Keep facts; do not preserve duplicate names if final `Source` owns them. |
| `HttpAssetSource` | Single remote offer | Map to offered source facts. |
| `MirrorSource` / `SourcePath` | Mirror offer expansion | Keep only if mirror behavior remains. |
| `GitSource` | Git offer facts | Defer until Git behavior exists. |
| `SourceSet` | Offered source set | Behavior state candidate; simplify. |
| `SourceSpec` / `SourcePlan<Unplanned>` | Declared/offered source wrapper | Delete public typestate; maybe internal during analysis. |

Glue/delete:

```text
SourceSpec alias
SourcePlan<Unplanned> public typestate
PassthroughAdapter if only identity glue
```

Migration law:

```text
Offer may expand source candidates but cannot claim which candidate was used.
```

### Select — offered source set to chosen source

Behavior semantics:

```text
Choose one candidate from an offered set according to policy and availability.
```

Migrate as behavior-defined semantics:

| Old surface | Semantic state/facts | Disposition |
|---|---|---|
| `SelectionStrategy` | Select policy | Keep only policy variants with live behavior. |
| `PlannedSources` / `SourcePlan<Planned>` | Chosen/planned candidate set | Delete public typestate; internal/evidence if needed. |
| `ResolvedSourceCandidate` | Chosen source evidence | Keep as `Chosen source` facts if receipt needs it. |
| `MultiSourceFetcher` selection loop | Select + Acquire mixed behavior | Split semantics; do not preserve wrapper as public API. |
| `SourceAdapter` | Glue around source conversion | Prefer behavior relation; delete if duplicating Offer/Select. |

Glue/delete:

```text
SourcePlan<Planned>
PlannedSources as caller type
SourceAdapter/PassthroughAdapter unless real interchangeable behavior exists
```

Migration law:

```text
Select cannot perform transfer. Race-style Select must still emit which source won.
```

### Acquire — chosen source to acquired material

Behavior semantics:

```text
Obtain bytes/tree/material from the chosen source.
```

Migrate as behavior-defined semantics:

| Old surface | Semantic state/facts | Disposition |
|---|---|---|
| `Fetcher<C>` | Acquire implementation | Keep behavior, not public workflow noun. |
| `HttpClient` / `ReqwestClient` | Transfer mechanism | Internal or injectable leaf only if needed. |
| `FetchOptions` | Mixed policy + mechanics | Split: Need/network policy vs Acquire mechanics. |
| `RetryPolicy` | Acquire mechanic/policy | Internal unless caller truly chooses retry. |
| `FetchPhase` / `Progress` / `ExtendedProgress` | Acquire observation | Evidence/progress optional, not behavior state. |
| `FetchReceipt` | Acquisition evidence | Facts migrate into Evidence; not manual glue. |
| `FetchSource` | Duplicate source/chosen-source vocabulary | Delete as public peer. |
| `RemoteMetadata` / `ConditionalOptions` | Acquire cache/conditional facts | Internal/evidence when net behavior migrates. |
| `BatchFetcher` / `BatchOptions` / `BatchResult` | Batch Acquire | Defer; separate behavior family. |
| `SegmentedFetcher` / `ResumableFetcher` / `ConditionalFetcher` | Acquire implementations | Internal; enable only when behavior needs them. |
| `TokenBucket` / rate metrics | Acquire mechanism | Internal. |
| `CompressionType` / `StreamTransform` codec | Transfer decoding mechanism | Internal; may belong Verify/Prepare depending semantics. |

Glue/delete:

```text
FetchReceipt as input to store/archive
FetchSource as duplicate source type
MultiSourceFetcher if it bundles Select+Acquire as public workflow
```

Migration law:

```text
Acquire evidence can be observed by Receipt, but callers must not have to pass it manually into Prepare/Remember.
```

### Verify — acquired material to verified material

Behavior semantics:

```text
Check acquired material against integrity/trust requirements.
```

Migrate as behavior-defined semantics:

| Old surface | Semantic state/facts | Disposition |
|---|---|---|
| `DigestAlgorithm` | Verification policy/evidence | Keep behind Verify semantics. |
| `ValidDigest` | Verification fact | Evidence; not independent workflow object. |
| `VerificationRequirement` | Verify need | Migrate only if Verify behavior uses it. |
| `TrustMode` / `TrustAnchor` / `TrustPolicy` | Trust policy | Shrink; do not port wholesale. |
| `TrustDecision` | Verification evidence | Receipt evidence. |
| `SignatureAlgorithm` / `Signature` / `PublicKey` / `SignatureConfig` | Verify policy/mechanism | Optional trust implementation. |
| `SignatureVerifier` / `SignatureManager` | Verify mechanism | Internal/injectable only if multiple real implementations. |
| `VersionKind` / `CalVer` / `Partial` | Version parse facts | May support Declare/Select; not Verify unless material version proof exists. |

Glue/delete:

```text
Fetch-local checksum wrappers that duplicate canonical Verify semantics
trust wrappers that are not behavior-selected
```

Migration law:

```text
If Need requires verification, Apply cannot consume unverified material.
Verify never changes material identity.
```

### Prepare — verified material to prepared material

Behavior semantics:

```text
Shape material into something Apply can consume.
```

Migrate as behavior-defined semantics:

| Old surface | Semantic state/facts | Disposition |
|---|---|---|
| `ArtifactForm` | Desired/observed material shape | Need or evidence depending behavior. |
| `UnpackPolicy` / `MaterializationSpec` | Preparation need | Split user policy from implementation choice. |
| `ArchiveFormat` / `TarCompress` / `Decoder` | Prepare mechanism/evidence | Internal/evidence. |
| `ExtractOptions` | Mixed policy + mechanism | Split; only safety policy may surface. |
| `HashStrategy` | Verify/Prepare evidence option | Prefer Verify; only keep if archive entry hashes matter. |
| `PermissionStrategy` / `PermissionResolution` | Prepare safety policy/evidence | Keep if law needs permission preservation. |
| `SanitizedPath` | Safety invariant | Internal evidence detail. |
| `Entry` / `EntryKind` | Preparation evidence | Keep as evidence details if needed. |
| `ArchiveReport` | Preparation evidence | Facts migrate; not caller glue. |
| `EntrySource` / `PendingEntry` | Prepare implementation | Internal. |
| `Extracted` | Prepared material candidate | Keep only if it owns invariant. |
| `WorkspaceExtraction` | Prepared material + evidence wrapper | Collapse if duplicate. |
| `ExtractedTreeRegistration` / `ExtractRegistration` | Prepare->Remember glue | Delete. |

Glue/delete:

```text
ArchiveReport + root tuple protocols
ExtractedTreeRegistration
ExtractRegistration
IntoExtractRegistration
WorkspaceExtraction if just root+report duplicate
```

Migration law:

```text
Prepare output must carry material shape and required evidence together; caller must not pair root/report manually.
```

### Apply — prepared material to applied target

Behavior semantics:

```text
Perform target mutation and activation/rollback facts under operation policy.
```

Migrate as behavior-defined semantics:

| Old surface | Semantic state/facts | Disposition |
|---|---|---|
| `InstallInput` | Prepared material adapter | Delete public enum; Apply consumes behavior-defined Prepared. |
| `IntoInstallInput` | Conversion glue | Delete. |
| `InstallReady` | Apply context with state coupling | Internal or delete; not public behavior input. |
| `InstallCapabilities` | Apply constraints/capability evidence | Migrate only if Need/diagnostics require. |
| `InstallPlanningRequest` / `InstallPlanReport` | Apply planning diagnostics | Internal/evidence; not required public step. |
| `InstallPlanLimitation` | Apply diagnostic evidence | Keep as evidence if useful. |
| `InstallWorkflowVariant` | Old workflow branch label | Delete/internal unless diagnostics need it. |
| `InstallFlow<S>` / `PlannedInstall` / `StagedInstall` / `InstalledInstall` / `ActivatedInstall` | Public choreography | Delete public typestate; internal sequence only. |
| `StagingArea` | Apply mechanism | Internal. |
| `ActivationRequest` / `Activator` | Activation mechanism | Internal; trait only if interchangeable implementations remain. |
| `SymlinkActivator` / `CopyFileActivator` / `Shim*Activator` | Apply implementations | Internal/feature behavior. |
| `ShimCommand` / `InstalledShimResolver` | Activation mechanics | Internal. |
| `InstallReceipt` | Apply evidence/core receipt | Migrate facts into Receipt/Evidence. |
| `ActivationReceipt` | Apply evidence | Migrate as Evidence. |
| `RollbackReceipt` | Repair/Apply failure evidence | Migrate under Repair/Apply. |
| `LifecycleOperationReceipt` / details/phase | Event evidence | Collapse into Receipt/Evidence if useful. |

Glue/delete:

```text
InstallInput
IntoInstallInput
InstallSpec as peer request
InstallFlow<S> public typestate
manual stage -> commit -> activate -> finish as caller path
```

Migration law:

```text
Apply is the mutation boundary and cannot choose sources or invent acquisition/preparation evidence.
```

### Remember — receipt/applied target to remembered fact

Behavior semantics:

```text
Persist evidence/facts for future inspection, repair, retention, or cache.
```

Migrate as behavior-defined semantics:

| Old surface | Semantic state/facts | Disposition |
|---|---|---|
| `StoreRoots` / `StoreReady` | Remember context/mechanism | Internal until explicit memory API exists. |
| `StoreKey` / `KeyDerivation` | Remember mechanism identity | Internal; not caller input. |
| `StoreProvenance` | Remembered evidence summary | Facts migrate into Evidence. |
| `StoreMetadataRecord` | Remembered fact | Persisted evidence shape; internal/public only for inspect API. |
| `StoredKind` | Remember mechanism discriminator | Internal. |
| `StoredArtifact` / `ExtractedArtifact` | Remembered material refs | Evidence/internal; not Apply input. |
| `ArtifactRegistration` / `ExtractRegistration` | Registration bags | Delete or internal. |
| `IntoArtifactRegistration` / `IntoExtractRegistration` | Conversion glue | Delete. |
| `PruneReport` / `MetadataPrunePlan` | Forget/Inspect evidence | Later retention behavior. |
| `StateReady` / `ResourceRecordPatch` | Remember lifecycle implementation | Internal; cannot be main operation input. |
| `ResourceRecord` / `ResourceLifecycle` / `ActivationRecord` | Remembered facts | Keep facts under Remember/Inspect semantics. |

Glue/delete:

```text
IntoArtifactRegistration
IntoExtractRegistration
ArtifactRegistration as public bag
ExtractRegistration as public bag
StoreKey as caller object
StateReady as caller apply dependency
```

Migration law:

```text
Remember cannot create lifecycle truth that Apply did not produce.
Memory cannot be more authoritative than its evidence.
```

### Inspect — remembered/applied facts to observed state

Behavior semantics:

```text
Observe remembered facts and optional live target state without mutating.
```

Migrate as behavior-defined semantics:

| Old surface | Semantic state/facts | Disposition |
|---|---|---|
| `StateSnapshot` / `ResourceStateSnapshot` | Observed/remembered snapshots | Inspect evidence. |
| `InspectionSeverity` / `InspectionCategory` | Inspection classification | Keep if reports stay useful. |
| `ResourceInspectionFinding` / `Summary` / `Report` | Observed state report | Keep as Report semantics, not Apply path. |
| `ActivationOwnershipReport` / entries/conflicts | Observed state report | Inspect semantics. |
| `OwnershipSeverity` / `OwnershipReason` | Inspection evidence | Keep if laws require. |
| `LockFile` / `LockedResource` | Remembered fact snapshot | Inspect/Remember, not Apply input. |
| `LockDiff` / `LockResourceChange` | Observed difference | Inspect report. |
| `StateAnalysisIndex` | Inspect mechanism | Internal. |
| `StoreKeyReference` / protected/removable metadata types | Inspect/Forget retention facts | Later. |

Glue/delete:

```text
Inspect reports used as mutation inputs without explicit Repair/Forget behavior
state/store split records that duplicate receipt facts without law
```

Migration law:

```text
Inspect must distinguish missing evidence from negative evidence.
Inspect cannot mutate.
```

### Repair — observed state to repair plan or repair apply

Behavior semantics:

```text
Turn observed inconsistency into explicit repair intent or explicit mutation.
```

Migrate as behavior-defined semantics:

| Old surface | Semantic state/facts | Disposition |
|---|---|---|
| `ResourceRepairPlan` | Repair plan | Keep as Repair state. |
| `ResourceRepairAction` | Repair action | Keep if plan remains useful. |
| `BackupReceipt` | Recovery evidence | Repair/Apply evidence. |
| `RestoreReceipt` | Repair evidence | Repair evidence. |
| `RollbackReceipt` | Apply failure/Repair evidence | Repair evidence. |
| backup/restore helpers | Repair implementations | Internal. |

Glue/delete:

```text
Rollback hidden inside non-repair flows without evidence
repair plans that directly mutate without Apply/Receipt semantics
```

Migration law:

```text
Repair is a new behavior, not a hidden side effect of Inspect.
```

### Forget — remembered/applied state to absence/removal evidence

Behavior semantics:

```text
Explicitly remove target/facts or make absence durable.
```

Migrate as behavior-defined semantics:

| Old surface | Semantic state/facts | Disposition |
|---|---|---|
| `UninstallOptions` | Forget policy | Keep semantics if uninstall remains. |
| `UninstallDisposition` | Forget policy axis | Simplify if possible. |
| `UninstallReceipt` | Forget evidence | Receipt/Evidence. |
| `StoreRetentionPolicy` | Forget/retention policy | Later Remember/Forget semantics. |
| `StoreRetentionPlan` / `ReasonedStoreRetentionPlan` / `OwnershipRetentionPlan` | Forget plan/evidence | Later. |
| prune functions/reports | Forget implementations/evidence | Internal/later. |

Glue/delete:

```text
Forget operations that erase evidence without receipt when retention requires it
retention types exposed before Remember/Inspect laws stabilize
```

Migration law:

```text
Forget is explicit and must not leave remembered lifecycle facts claiming active state.
```

## Cross-cutting helper/mechanism classification

These should not drive migration order.

| Surface family | Behavior support | Disposition |
|---|---|---|
| `pulith-fs::Workspace` / `Transaction` | Apply/Remember mechanisms | Internal. |
| `copy_dir_all`, `replace_dir`, `hardlink_or_copy`, `atomic_symlink`, `atomic_write` | Apply/Remember mechanisms | Keep behavior, not public semantic layer. |
| `PermissionMode` | Prepare/Apply mechanism/policy | Keep only if law needs it. |
| `Resource<'a>` / `Content` in `pulith-fs` | Mechanism | Rename/delete later to avoid semantic conflict. |
| `AlignedBuf` | Mechanism/perf | Delete unless used by behavior implementation. |
| error enums / `Result` aliases | Local error surfaces | Collapse only when implementation migrates. |
| `PerformanceReport`, memory/throughput/timer types | Acquire diagnostics | Optional diagnostics; not behavior state. |

## Migration phases

### Phase 0 — Freeze behavior contract

Goal:

```text
Accept behavior morphisms and semantic states as the migration axis.
```

Deliverables:

```text
docs/report/pulith-behavior-morphism-spec.md accepted
docs/report/pulith-behavior-semantic-migration-plan.md accepted
```

No code.

Verification:

```text
Docs classify every old public surface by behavior semantics.
Docs explicitly reject file/module/example-driven migration.
```

### Phase 1 — Declare/Offer semantics

Goal:

```text
Replace old request/source vocabulary with behavior-defined Declared intent and Offered source semantics.
```

Migrate semantics:

```text
ResourceId
VersionSelector only if needed
ResourceLocator facts
SourceDefinition facts
SourceSet facts
network/trust/material needs only as behavior constraints
```

Delete/demote:

```text
ResourceSpec wrapper
ResourceBehaviorContract
Requested/Resolved public phases
SourcePlan<Unplanned>
SourceSpec
PassthroughAdapter if redundant
```

Exit criteria:

```text
One canonical declaration vocabulary.
One canonical offered-source vocabulary.
No chosen/acquired/prepared/apply semantics mixed into declaration.
```

### Phase 2 — Select semantics

Goal:

```text
Represent chosen-source semantics without exposing planning typestate.
```

Migrate semantics:

```text
SelectionStrategy if live
ResolvedSourceCandidate facts as chosen-source evidence
mirror/fallback/race selection facts
```

Delete/demote:

```text
SourcePlan<Planned>
PlannedSources public type
SourceAdapter if it only converts old source shapes
MultiSourceFetcher selection wrapper as public API
```

Exit criteria:

```text
Selection is a behavior relation from offered source set to chosen source.
Selection cannot obtain material.
```

### Phase 3 — Acquire/Verify semantics

Goal:

```text
Migrate acquisition and verification facts without retaining FetchReceipt as caller glue.
```

Migrate semantics:

```text
FetchReceipt facts -> acquisition evidence
FetchOptions split into Need vs mechanics
DigestAlgorithm / ValidDigest / TrustDecision -> verification evidence
Signature facts if needed
```

Delete/demote:

```text
FetchSource
FetchReceipt as manual input to archive/store
Fetcher/MultiSourceFetcher as main workflow objects
batch/resume/segment/conditional fetch public surfaces until behavior needs them
```

Exit criteria:

```text
Acquired material and verified material are distinct semantic states.
Apply cannot consume unverified material when Need requires Verify.
```

### Phase 4 — Prepare semantics

Goal:

```text
Migrate material shaping and archive evidence without root/report tuple glue.
```

Migrate semantics:

```text
ArchiveFormat / ArchiveReport facts
Entry / EntryKind facts
Extracted / WorkspaceExtraction only if they express Prepared material invariant
ExtractOptions safety policy
SanitizedPath / PermissionResolution evidence
```

Delete/demote:

```text
ExtractedTreeRegistration
ExtractRegistration
IntoExtractRegistration
ArchiveReport as caller-required input to Remember/Apply
EntrySource public trait unless real behavior relation remains
```

Exit criteria:

```text
Prepared material carries enough evidence for Apply/Receipt.
Caller never pairs archive report and root manually.
```

### Phase 5 — Apply semantics

Goal:

```text
Migrate target mutation semantics and delete public install choreography.
```

Migrate semantics:

```text
InstallMode/OperationMode into one operation mode
Activation facts into target/apply evidence
InstallReceipt / ActivationReceipt / RollbackReceipt facts into Receipt/Evidence
Apply diagnostics from InstallPlanReport if still useful
```

Delete/demote:

```text
InstallSpec
InstallInput
IntoInstallInput
InstallFlow<S>
Planned/Staged/Installed/Activated public types
ActivationRequest as public input
Activator public trait unless injection is required
```

Exit criteria:

```text
Apply is the only target mutation boundary.
Stage/commit/activate may exist internally but not as required caller choreography.
```

### Phase 6 — Remember semantics

Goal:

```text
Migrate persistence as fact retention, not as install/source/archive glue.
```

Migrate semantics:

```text
StoreMetadataRecord facts
StoreProvenance facts
ResourceRecord / ResourceLifecycle / ActivationRecord facts
Store roots/schema only as memory implementation details
```

Delete/demote:

```text
IntoArtifactRegistration
IntoExtractRegistration
ArtifactRegistration public bag
ExtractRegistration public bag
StoreKey as caller object
StoredArtifact/ExtractedArtifact as Apply input
StateReady as Apply dependency
```

Exit criteria:

```text
Remember stores only facts produced by prior behavior.
Remember cannot invent lifecycle truth.
```

### Phase 7 — Inspect/Repair/Forget semantics

Goal:

```text
Migrate read-only observation, explicit repair, and explicit deletion behaviors.
```

Migrate semantics:

```text
ResourceInspectionReport/Finding/Summary
ActivationOwnershipReport
LockFile/LockDiff as remembered/observed facts
ResourceRepairPlan/Action
Backup/Restore/Rollback facts
UninstallOptions/Receipt
Retention plans/policies
```

Delete/demote:

```text
mutation hidden inside Inspect
Forget hidden inside Apply cleanup
retention public types before Remember/Inspect laws are stable
state/store duplicate records not grounded in Receipt evidence
```

Exit criteria:

```text
Inspect observes only.
Repair and Forget are explicit behaviors with receipts/evidence.
Missing evidence is explicit.
```

### Phase 8 — Implementation attachment

Goal:

```text
Only after semantic migration, attach concrete implementations to behavior-defined states.
```

Order:

```text
local acquire implementation
identity verify/prepare implementation
basic apply implementation
optional remember implementation
archive prepare implementation
net acquire implementation
inspect/repair/forget implementations
```

Rules:

```text
No compatibility aliases.
No old crate-shaped modules just because old crates existed.
No example-defined behavior.
No new peer type if semantics already exist.
```

Exit criteria:

```text
Active examples/tests use behavior-defined API.
Old public glue imports are absent.
Old crates can be deleted when active importers are gone.
```

## Structural deletion inventory

Delete or demote before claiming migration complete:

```text
Requested / Resolved typestate markers as public phases
RequestedResource / ResolvedResource public split
ResolvedResourceContext
ResourceBehaviorContract
SourcePlan<S> public typestate
SourceSpec
PlannedSources
SourceAdapter/PassthroughAdapter if redundant
FetchSource
FetchReceipt as manual glue object
ArchiveReport as manual glue object
ExtractedTreeRegistration
ExtractRegistration
IntoArtifactRegistration
IntoExtractRegistration
InstallSpec
InstallInput
IntoInstallInput
InstallFlow<S>
PlannedInstall / StagedInstall / InstalledInstall / ActivatedInstall public types
StoreKey as caller object
IntoResourceUpsert
StateReady as apply dependency
```

## Required acceptance checks before code

Before implementation begins, this plan is accepted only if:

```text
each public surface has a behavior relation or delete/demote disposition
each migrated semantic state has incoming/outgoing behavior laws
no phase is organized by old crate name alone
file/module layout remains explicitly out of scope
implementation is last
```

## First code slice after acceptance

Implementation migration is further constrained by:

```text
docs/report/pulith-implementation-library-error-orthogonality-plan.md
```

Before any concrete implementation is migrated, search mature Cargo crates for that mechanism. If mature crates already implement download, archive extraction, decompression, hashing, filesystem staging, locking, or persistence, the old Pulith implementation is marked delete/adapter-only unless it owns Pulith behavior evidence.

Concrete migration also requires:

```text
behavior-specific error categories instead of one large enum
feature-gated implementation families
orthogonal implementation semantics
accurate behavior/state/evidence mapping for every implementation
```

After this design is accepted, the first code slice should be semantic, not implementation-heavy:

```text
Declare/Offer vocabulary only.
```

It should not yet migrate net/archive/store/install implementations.

Expected code goal later:

```text
One canonical declaration type.
One canonical offered-source semantic shape.
No Requested/Resolved public split in the new path.
No SourcePlan public typestate in the new path.
No example-defined behavior.
```

## Verification plan for eventual code phases

Each implementation phase must include:

```text
focused behavior tests for the behavior relation
absence checks for retired glue imports/types
cargo check for touched crates
cargo test for focused behavior suites
ad-hoc hermes-verify script only if external stale-verification guard requires it
```

But this document itself requires only doc verification:

```text
git diff --check -- docs/report/pulith-behavior-semantic-migration-plan.md
marker check for behavior relations, phases, deletion inventory, and non-goals
```
