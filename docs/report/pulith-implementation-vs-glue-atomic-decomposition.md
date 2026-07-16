# Pulith Implementation vs Glue Atomic Decomposition

## Status

Design/decomposition report only. No Rust code changes are authorized by this report.

This report corrects the previous migration posture:

```text
Do not migrate yet.
First distinguish implementation units from glue layers.
Then design implementation as minimal independent atoms.
Only after that decide what to absorb, delete, or rewrite.
```

The existing `pulith` skeleton remains useful as a behavior-spine proof, but it is not a license to start porting old APIs.

## Why this report exists

The previous classification answered:

```text
Where could each old public surface land in the new spine?
```

That is necessary but still too migration-oriented. The next question must be:

```text
What is real implementation?
What is glue between implementations?
What is policy/configuration?
What is evidence/fact?
What should be deleted instead of migrated?
```

If this distinction is skipped, `pulith` will simply recreate the old crate graph inside one crate.

## Terms

### Implementation unit

A minimal unit that performs one irreversible or externally meaningful behavior.

It has:

- one input contract;
- one output product/evidence;
- one side-effect boundary, or no side effects;
- no knowledge of the full workflow;
- no dependency on caller choreography.

Examples:

```text
copy path
create directory
download bytes
select source candidate
extract archive entry safely
compute digest
write receipt atomically
activate symlink
create rollback snapshot
```

### Glue layer

A layer whose main purpose is to convert, wrap, or stitch outputs from one crate into inputs for another.

Typical signs:

- `Into*` conversion traits;
- typestate wrappers used only to force call order;
- tuple protocols;
- repeated caller sequence with no user decision in the middle;
- objects whose fields are mostly other crates' receipts/reports/paths;
- public APIs that exist because crate boundaries exist.

### Policy/configuration

A user or caller decision that changes behavior.

Examples:

```text
offline vs network allowed
replace vs create-only
retain evidence or not
trust/digest requirement
allowed source candidates
activation desired or not
rollback required or not
```

Policy is not implementation and not glue. It belongs in `Application`, `Requirements`, `Operation`, or `EvidencePolicy` only when the user can reasonably choose it.

### Evidence/fact

A record of what happened or what was observed.

Examples:

```text
selected source
download destination
digest value
archive entries extracted
sanitized path decisions
target path written
activation performed
rollback snapshot created
persisted receipt location
```

Evidence must not become a mandatory caller stitching object.

## High-level decomposition

The main behavior can still be described as:

```text
Application -> Acquire -> Prepare -> Apply -> Receipt
```

But the implementation units below it are smaller:

```text
Acquire
  SourceInterpret
  SourceSelect
  MaterialReadOrFetch
  MaterialValidate

Prepare
  MaterialIdentify
  MaterialTransform
  PreparedValidate

Apply
  TargetPlan
  TargetMutate
  ActivationMutate
  RecoverySnapshot

Evidence
  EvidenceBuild
  EvidencePersist
  EvidenceInspect
```

These are not new public traits yet. They are design atoms for judging current code.

## Current surfaces by nature

### Real implementation units

These contain actual behavior worth preserving, but not necessarily as public APIs.

| Area | Current examples | Atomic role | Future shape |
|---|---|---|---|
| Path copy/link/write | `pulith-fs` primitives, current `TargetApply` | `TargetMutate` | internal `target`/`fs` helpers |
| Atomic write/transaction | `pulith-fs::atomic_write`, `Transaction` | `EvidencePersist` / safe mutate | internal helper |
| Directory workspace | `pulith-fs::Workspace` | temp workspace allocation | internal helper |
| HTTP byte acquisition | `Fetcher`, `HttpClient`, `ReqwestClient` | `MaterialReadOrFetch` | `net` feature implementation |
| Candidate fallback | `MultiSourceFetcher`, `SelectionStrategy` | `SourceSelect` | acquire policy + implementation |
| Local source path handling | `LocalSource`, `ResourceLocator::LocalPath`, `Source::LocalPath` | `SourceInterpret` / local acquire | keep minimal in `pulith::Source` and `local` |
| Checksum/digest | digest/hash code, `ValidDigest`, `DigestAlgorithm` | `MaterialValidate` / `EvidenceBuild` | `hash` feature/evidence |
| Signature verification | signature codec types | `MaterialValidate` | optional trust implementation, not first slice |
| Archive format detect/decode | `ArchiveFormat`, `Decoder`, tar/zip sources | `MaterialIdentify` / `MaterialTransform` | `archive` feature implementation |
| Safe path extraction | `SanitizedPath`, `PermissionStrategy`, entry checks | `PreparedValidate` | archive-internal safety atoms |
| Store metadata write | `StoreMetadataRecord`, atomic persistence | `EvidencePersist` | `persist` feature implementation |
| Activation mutation | `SymlinkActivator`, `CopyFileActivator`, shim activators | `ActivationMutate` | target/apply implementations |
| Rollback/backup restore | backup/restore/rollback code | `RecoverySnapshot` / recovery mutate | apply/evidence implementation |
| Inspection/repair analysis | state inspection/repair reports | `EvidenceInspect` | later inspect/repair module |

These should be decomposed into small modules/functions before being lifted into `pulith`.

### Glue layers

These should not be ported as-is.

| Current surface | Why it is glue | Action |
|---|---|---|
| `IntoArtifactRegistration` | Converts many caller shapes into store registration because store is a separate workflow checkpoint. | Delete after direct evidence path exists. |
| `IntoExtractRegistration` | Same, plus tuple/workflow stitching. | Delete. |
| `ExtractRegistration` | Intermediate construction bag. | Internal/delete. |
| `ExtractedTreeRegistration` | Good transitional name, but still exists to bridge archive report/root/fetch into store. | Delete after `Prepare` output carries prepared tree/evidence. |
| `IntoInstallInput` | Converts prepared artifacts into install enum. | Delete. |
| `InstallInput` public enum | Forces caller to select old prepared material representation. | Replace with typed `Prepared` associated type. |
| `IntoResourceUpsert` | Converts into state record update. | Delete; evidence persistence owns shape. |
| `RequestedResource` / `ResolvedResource` split | Exposes an intermediate resolution phase to caller. | Replace with Application + acquisition evidence unless proven necessary. |
| `SourcePlan<Unplanned/Planned>` | Typestate planning surface around source selection. | Internal or delete; use only if it prevents real misuse inside implementation. |
| `InstallFlow<S>` typestate | Exposes stage/commit/activate choreography. | Internal apply sequence only. |
| `ResolvedResourceContext` | Aggregates resolved facts for other crates. | Replace with evidence facts. |
| `StoreKey` as caller object | Caller reconstructs persistence identity. | Internal persist/evidence key. |
| `FetchReceipt` as caller object | Caller passes download evidence into archive/store manually. | Acquire evidence, not caller glue. |
| `ArchiveReport` as caller object | Caller pairs report with path/root manually. | Prepare evidence, not caller glue. |

### Policy/configuration surfaces

These should be reduced before implementation migration.

| Current surface | Real policy? | Target |
|---|---|---|
| `InstallMode` | yes | `Operation.mode` |
| `ConnectivityMode` | yes | `Requirements.network` |
| `TrustPolicy` | yes, but large | shrink into `EvidencePolicy` only when trust implementation migrates |
| `VerificationRequirement` | yes | `EvidencePolicy` / `Requirements` |
| `MaterializationSpec` | partly | split: user material preference vs internal prepare choice |
| `ActivationModel` | yes | `Operation.activation` or `Target` capability |
| `RollbackSupport` | yes | `Requirements.rollback` or `EvidencePolicy` |
| `FetchOptions` | mixed | split into user policy and net implementation knobs |
| `ExtractOptions` | mixed | split into user safety policy and archive implementation knobs |
| `StoreRetentionPolicy` | yes, but not main path | later persist/inspect policy |

Policy should not be copied wholesale. Each field must answer: can the user decide this at the top-level task?

### Evidence/fact surfaces

These facts should survive, but usually in new shapes.

| Current surface | Fact type | Target |
|---|---|---|
| `FetchReceipt` | acquisition fact | `Evidence::Acquired` |
| `ArchiveReport` | preparation fact | `Evidence::Prepared` |
| `StoreProvenance` | memory/persistence fact | `Evidence::Persisted` |
| `InstallReceipt` | apply fact | `Receipt` / `Evidence::Applied` |
| `ActivationReceipt` | activation fact | `Evidence::Activated` |
| `BackupReceipt` / `RollbackReceipt` / `RestoreReceipt` | recovery facts | `Evidence::Recovery` |
| `ResourceInspectionReport` | query fact | inspect API later |
| `LockFile` / `LockDiff` | persisted evidence snapshot/diff | persist/inspect later |

The rule is:

```text
evidence may be rich, but it must not be required as manual glue for the next step.
```

## Minimal implementation atom design

### Atom 1 — SourceInterpret

Question:

```text
What material reference did the user provide?
```

Input:

```text
Application.source
```

Output:

```text
SourceRef
```

Examples:

```text
local path
remote URL
candidate set
```

Implementation notes:

- no network;
- no filesystem mutation;
- may validate shape only;
- should not decide final selection among mirrors except for trivial single-source cases.

Old code source:

```text
ResourceLocator
SourceDefinition
LocalSource
RemoteSource
ValidUrl
```

### Atom 2 — SourceSelect

Question:

```text
Which candidate source should be attempted first / next?
```

Input:

```text
SourceRef + Requirements
```

Output:

```text
SelectedSource
```

Implementation notes:

- no download;
- no archive knowledge;
- fallback order and mirror selection live here;
- can emit selection evidence.

Old code source:

```text
SelectionStrategy
SourceSet
PlannedSources
ResolvedSourceCandidate
MultiSourceFetcher source loop pieces
```

### Atom 3 — MaterialReadOrFetch

Question:

```text
How do we obtain bytes/tree/material from the selected source?
```

Input:

```text
SelectedSource
```

Output:

```text
MaterialHandle + acquisition evidence
```

Implementation notes:

- local path may return a handle without copying;
- remote source may download to a temp/cache path;
- retries/resume/rate limiting are internal net details;
- no archive extraction.

Old code source:

```text
Fetcher
HttpClient
ReqwestClient
FetchOptions mechanics
ConditionalFetcher
ResumableFetcher
SegmentedFetcher
```

### Atom 4 — MaterialValidate

Question:

```text
Does the acquired material satisfy trust/integrity requirements?
```

Input:

```text
MaterialHandle + EvidencePolicy
```

Output:

```text
ValidatedMaterial + validation evidence
```

Implementation notes:

- digest/signature checks live here;
- no target mutation;
- no archive shape conversion unless validation requires reading container metadata.

Old code source:

```text
ValidDigest
DigestAlgorithm
VerificationRequirement
signature verifier types
checksum code
```

### Atom 5 — MaterialIdentify

Question:

```text
What shape is this material?
```

Input:

```text
ValidatedMaterial
```

Output:

```text
MaterialShape
```

Examples:

```text
file
directory
archive
executable
shim spec
```

Implementation notes:

- format detection only;
- no extraction;
- no target mutation.

Old code source:

```text
ArtifactForm
ArchiveFormat
Decoder detection
```

### Atom 6 — MaterialTransform

Question:

```text
How do we transform material into something apply can consume?
```

Input:

```text
ValidatedMaterial + MaterialShape + Requirements
```

Output:

```text
PreparedMaterial + preparation evidence
```

Examples:

```text
identity file
identity directory
archive extracted tree
```

Implementation notes:

- archive extraction lives here;
- safe path handling is inside the archive transform;
- output must carry root/path and report together, not as caller tuple glue.

Old code source:

```text
extract_from_reader
ExtractOptions
ArchiveReport
WorkspaceExtraction
Extracted
SanitizedPath
PermissionStrategy
```

### Atom 7 — TargetPlan

Question:

```text
What target mutation is required?
```

Input:

```text
PreparedMaterial + Target + Operation + Requirements
```

Output:

```text
TargetPlan
```

Implementation notes:

- may decide copy/link/symlink strategy;
- no mutation yet;
- creates a plan only if planning is truly useful;
- if no independent consumer exists, inline this into TargetMutate.

Old code source:

```text
InstallPlanningRequest
InstallPlanReport
InstallCapabilities
InstallMode
ActivationSupport
RollbackSupport
```

### Atom 8 — TargetMutate

Question:

```text
How is prepared material written to target?
```

Input:

```text
PreparedMaterial + TargetPlan or Operation
```

Output:

```text
TargetMutationEvidence
```

Implementation notes:

- filesystem writes/link/copy live here;
- may use atomic helpers;
- no activation if activation is a separate side effect.

Old code source:

```text
pulith-fs hardlink/copy/replace_dir
InstallFlow stage/commit internals
CopyFileActivator pieces
current TargetApply skeleton
```

### Atom 9 — ActivationMutate

Question:

```text
Does the applied target need an activation side effect?
```

Input:

```text
TargetMutationEvidence + Operation.activation
```

Output:

```text
ActivationEvidence
```

Implementation notes:

- symlink/shim activation lives here;
- optional: no activation is a valid no-op implementation;
- avoid exposing `Activator` as a user-main trait until runtime injection is required.

Old code source:

```text
ActivationTarget
ActivationRequest
Activator
SymlinkActivator
ShimLinkActivator
ShimCopyActivator
ActivationReceipt
```

### Atom 10 — RecoverySnapshot

Question:

```text
What must be captured before mutation so rollback/repair is possible?
```

Input:

```text
Target + Operation + Requirements
```

Output:

```text
RecoveryEvidence
```

Implementation notes:

- only runs if rollback/replace policy requires it;
- separate from target mutation;
- evidence feeds receipt.

Old code source:

```text
BackupReceipt
RollbackReceipt
RestoreReceipt
LifecycleRequirements
RollbackSupport
```

### Atom 11 — EvidenceBuild

Question:

```text
How do atom facts become one receipt?
```

Input:

```text
acquire evidence + prepare evidence + apply evidence + recovery evidence
```

Output:

```text
Receipt
```

Implementation notes:

- pure aggregation of facts;
- no IO;
- must not become a registry or global context bag.

Old code source:

```text
FetchReceipt
ArchiveReport
StoreProvenance
InstallReceipt
LifecycleOperationReceipt
```

### Atom 12 — EvidencePersist

Question:

```text
Where/how is receipt evidence retained?
```

Input:

```text
Receipt + EvidencePolicy
```

Output:

```text
PersistedEvidenceRef
```

Implementation notes:

- optional feature;
- may use atomic write and store roots internally;
- caller should not construct store keys.

Old code source:

```text
StoreReady
StoreRoots
StoreKey
StoreMetadataRecord
StateReady
LockFile
```

### Atom 13 — EvidenceInspect

Question:

```text
How do we query retained evidence for inspect/rollback/repair?
```

Input:

```text
PersistedEvidenceRef or resource identity
```

Output:

```text
InspectionReport / RepairPlan
```

Implementation notes:

- not part of first apply path;
- should remain separate from mutation;
- can be added after persistence is clear.

Old code source:

```text
ResourceInspectionReport
ResourceRepairPlan
ActivationOwnershipReport
StoreRetentionPlan
```

## Minimal composition levels

The atoms above should not all become public traits. Use three levels:

### Level 1 — public behavior spine

```text
Acquire
Prepare
Apply
Pipeline
```

This stays small.

### Level 2 — implementation atoms

```text
SourceInterpret
SourceSelect
MaterialReadOrFetch
MaterialValidate
MaterialIdentify
MaterialTransform
TargetMutate
ActivationMutate
RecoverySnapshot
EvidenceBuild
EvidencePersist
EvidenceInspect
```

These are module-level internal units or private traits/functions until multiple implementations require a trait.

### Level 3 — mechanism helpers

```text
copy_dir_all
atomic_write
sanitize_path
decode_zip_entry
retry_delay
hash_stream
write_json
```

These are plain functions. No public trait unless there are real interchangeable implementations.

## Important design correction

Do not map every atom to a trait.

The current public traits are enough:

```text
Acquire
Prepare
Apply
```

Most implementation atoms should begin as plain functions or small structs inside feature modules. Promote to a trait only when two real implementations must be composed interchangeably.

## Candidate module layout after decomposition

```text
crates/pulith/src/application.rs
crates/pulith/src/pipeline.rs
crates/pulith/src/evidence.rs
crates/pulith/src/error.rs

crates/pulith/src/local.rs          # local SourceInterpret + MaterialReadOrFetch
crates/pulith/src/target.rs         # TargetMutate + ActivationMutate basics
crates/pulith/src/archive.rs        # MaterialIdentify + MaterialTransform for archives
crates/pulith/src/net.rs            # SourceSelect + MaterialReadOrFetch for remote
crates/pulith/src/hash.rs           # MaterialValidate digest
crates/pulith/src/persist.rs        # EvidencePersist + EvidenceInspect basics

crates/pulith/src/fs.rs             # private mechanism helpers if needed
```

Do not create module names just because old crates exist. Create them only around implementation atoms.

## Decomposition of existing crates

### `pulith-resource`

Implementation units:

```text
ResourceId parse/validate
ValidUrl parse/validate
VersionSelector -> selection policy conversion
Digest validation
TrustDecision evaluation
```

Glue/policy mix:

```text
ResourceSpec combines too many axes.
ResourceBehaviorContract aggregates materialization/activation/mutation/provenance/lifecycle.
Requested/Resolved typestate exposes workflow phase.
```

Minimal design:

```text
Keep tiny validators if needed.
Split ResourceSpec into Application.resource + Source + Requirements.
Do not keep ResourceBehaviorContract wrapper.
Do not keep Requested/Resolved as public phases.
```

### `pulith-source`

Implementation units:

```text
source definition normalization
candidate ordering
source adapter conversion
```

Glue:

```text
SourcePlan<Unplanned/Planned>
PlannedSources as fetch input
PassthroughAdapter if it only copies through
```

Minimal design:

```text
SourceInterpret and SourceSelect atoms.
Keep candidate selection only if multiple sources are actually active.
Do not expose planning typestate.
```

### `pulith-fetch`

Implementation units:

```text
HTTP client stream/head
download to destination
conditional/resume/segment mechanics
retry/backoff/rate limit
checksum/signature validation
progress reporting
```

Glue:

```text
Fetcher methods accepting ResolvedResource/PlannedSources because other crates require those shapes.
FetchReceipt passed manually into store/archive.
```

Minimal design:

```text
net Acquire implementation composed from SourceSelect + MaterialReadOrFetch + MaterialValidate.
FetchReceipt facts become acquisition evidence.
Complex fetch variants stay internal until a caller-level requirement demands them.
```

### `pulith-archive`

Implementation units:

```text
format detection
decoder construction
entry iteration
path sanitization
permission handling
extract tree construction
```

Glue:

```text
ArchiveReport must be paired manually with root path and fetch receipt.
WorkspaceExtraction duplicates root/report shape unless it owns the whole prepared product.
```

Minimal design:

```text
archive Prepare implementation: ValidatedMaterial -> PreparedTree + PrepareEvidence.
Root and report travel together.
Entry safety stays internal.
```

### `pulith-store`

Implementation units:

```text
artifact persistence
metadata write
key derivation
prune planning
```

Glue:

```text
IntoArtifactRegistration
IntoExtractRegistration
ExtractRegistration
ExtractedTreeRegistration
manual StoreKey reconstruction
```

Minimal design:

```text
EvidencePersist atom.
Store keys are internal evidence refs.
Registration conversion traits deleted.
```

### `pulith-state`

Implementation units:

```text
resource record persistence
state snapshot
inspection finding construction
repair planning
ownership/retention analysis
lock diff
```

Glue:

```text
IntoResourceUpsert
state upsert from resource/store shapes
state/store split forcing duplicate records
```

Minimal design:

```text
EvidenceInspect and repair planning later.
Do not pull state into first apply path.
Persisted receipt should be the source of inspect facts where possible.
```

### `pulith-install`

Implementation units:

```text
target staging
commit/replace
activation mutation
backup/restore/rollback
uninstall mutation
```

Glue:

```text
InstallInput
IntoInstallInput
InstallSpec
InstallPlanningRequest
InstallFlow<S>
manual stage/commit/activate/finish public choreography
```

Minimal design:

```text
Apply implementation composed from TargetPlan/TargetMutate/ActivationMutate/RecoverySnapshot/EvidenceBuild.
Keep public path as Apply, not InstallFlow typestate.
```

### `pulith-fs`

Implementation units:

```text
atomic write
hardlink/copy fallback
replace directory
symlink/junction handling
workspace temp dirs
permission helpers
```

Glue:

```text
public fs crate as workflow concept
Resource<'a> naming conflict with domain Resource
```

Minimal design:

```text
private/internal mechanism helpers inside pulith.
Expose only if a real user-level filesystem utility product exists, which current top-down goal does not require.
```

### `pulith-version`

Implementation units:

```text
semver/calver/partial parse
requirement matching
candidate preference selection
```

Glue:

```text
separate crate boundary for version before Application vocabulary stabilizes
```

Minimal design:

```text
Keep version parser/selector as internal `application` or `version` module only when Application needs richer version semantics.
Do not port before a migrated path requires it.
```

## What to do before any migration

1. Choose one old crate area.
2. Split its surfaces into:

```text
implementation atoms
policy fields
evidence facts
glue/delete
```

3. For each implementation atom, write:

```text
input
output
side effects
errors
whether it needs a trait or just a function
```

4. Only then implement or move code.

## Next report / design artifact

Before more code, write a narrower design report for the first implementation family:

```text
docs/report/pulith-local-target-atomic-design.md
```

Scope:

```text
SourceInterpret(LocalPath)
MaterialReadOrFetch(local path)
MaterialIdentify(file/dir)
MaterialTransform(identity)
TargetMutate(copy/replace)
EvidenceBuild(receipt)
```

This report should decide whether current `local.rs` should stay as one file or split into:

```text
local.rs
target.rs
fs.rs
```

Do not migrate examples yet.

## Verification checklist for this report

This report is healthy if:

- it does not propose direct old API migration;
- it separates implementation from glue;
- it marks conversion traits and typestate workflow surfaces as glue;
- it decomposes implementation into atoms smaller than `Acquire/Prepare/Apply`;
- it explicitly says most atoms should start as functions/small structs, not traits;
- it names the next design artifact before code.
