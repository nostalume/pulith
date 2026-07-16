# Pulith Unified Atomic Architecture Analysis

## Status

Design/analysis only. No Rust code changes are authorized by this document.

This document supersedes the working notes in:

```text
docs/report/pulith-public-api-surface-classification.md
docs/report/pulith-implementation-vs-glue-atomic-decomposition.md
```

Those reports remain historical evidence. This document is the current unified analysis contract.

## User correction incorporated

The next step is not direct migration.

The design must first:

1. distinguish abstraction levels;
2. define behavior from DDD behavior boundaries, not from examples;
3. treat behaviors as morphisms: a behavior is understood by what other behaviors can compose with it;
4. migrate behavior-defined semantic distinctions before assigning behavior to semantic types;
5. split real implementation from glue;
6. avoid semantic duplicate types;
7. keep semantic concepts separate from low-level implementation mechanisms;
8. keep names short, readable, and stable.

The key failure mode to avoid:

```text
single crate that internally recreates old fetch/archive/store/install/state API mass
```

The goal is instead:

```text
small semantic API
plain implementation atoms
rich evidence only where useful
no caller-visible glue
```

## Behavior-first DDD rule

Current priority:

```text
Do not focus on bottom-level design.
Do not focus on file organization.
Do not start from examples.
Define behavior boundaries first.
```

DDD here means:

```text
a behavior is a domain morphism between semantic states
```

It should be defined by:

- what semantic state it accepts;
- what semantic state it produces;
- what adjacent behaviors can compose with it;
- what laws/invariants composition must preserve;
- what evidence it must expose after composition.

It should not be defined by:

- one demo path;
- one file layout;
- one implementation mechanism;
- one crate's existing public object graph;
- a convenience wrapper around old steps.

The useful Yoneda-style intuition:

```text
We understand a behavior by all valid ways other behaviors can observe/compose with it.
```

So a Pulith behavior is not primarily a struct with fields. It is a relation among neighboring behaviors.

Example:

```text
Acquire is defined by what Prepare can validly consume from it,
what Need can restrict about it,
and what Evidence can observe after it.
```

Not by:

```text
an HTTP fetch example
a local path example
a FetchReceipt struct
a pulith-fetch crate boundary
```

## Migration order correction

The correct order is:

```text
1. behavior specifications
2. semantic distinctions required by those behaviors
3. migration of old semantics into the behavior vocabulary
4. only then: behavior implementations owned by those semantics
```

Not:

```text
old type -> new type
old crate -> new module
old implementation -> new implementation
```

This means:

```text
First migrate the different semantics specified by behavior.
Then attach behavior to semantic types.
```

For example, do not begin with `Source` fields. Begin with source-side behavior boundaries:

```text
Offer -> Select -> Obtain -> Verify
```

Then decide which semantic distinctions are actually required:

```text
declared offer
chosen offer
obtained material
verified material
```

Only after that should code define or rename concrete types.

## Analysis levels

Pulith should use abstraction levels, but the analysis order is behavior-first:

```text
behavior relation -> semantic states -> public nouns -> implementation
```

A type must belong to exactly one level unless there is a proven reason otherwise.

### Level A — Behavior relation

Question:

```text
What domain morphism exists, and how does it compose with neighboring morphisms?
```

Initial behavior relations:

```text
Acquire
Prepare
Apply
Remember
Inspect
Repair
```

Main operation path begins with:

```text
Acquire -> Prepare -> Apply -> Receipt
```

But the behavior is not defined by a sample path. Each behavior is defined by valid compositions.

Examples:

```text
Acquire is valid if its output can be prepared or rejected with explicit evidence.
Prepare is valid if its output can be applied or rejected with explicit evidence.
Apply is valid if it produces facts sufficient for receipt/remember/inspect.
Remember, Inspect, and Repair may be later behaviors, but they still constrain what Receipt/Evidence must mean.
```

### Level B — Semantic state

Question:

```text
What state does a behavior consume or produce?
```

Semantic, stable, and behavior-defined.

Candidate nouns:

```text
App
Item
Source
Target
Op
Need
Receipt
Evidence
```

Current skeleton names can stay temporarily:

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

But future naming should prefer shorter final nouns if/when code is touched.

Semantic states must not contain:

```text
HTTP client details
archive decoder details
store key derivation
state upsert details
filesystem temp workspace details
manual stage/commit workflow states
```

### Level C — Public behavior API

Question:

```text
Which behavior relations are exposed for caller composition?
```

Keep the first public behavior API small:

```text
Acquire
Prepare
Apply
Pipeline
```

These are behavior slots. They are not old crate names.

Do not add peer behavior traits until behavior-relation analysis proves that another relation must be caller-composable.

### Level D — Implementation atoms

Question:

```text
What minimal implementation step performs real work?
```

Internal by default. Usually plain functions or small structs.

Atoms:

```text
read_source
choose_source
fetch
check
kind
unpack
plan_target
write_target
activate
snapshot
build_receipt
save_receipt
inspect
```

Longer design names used in previous report map to shorter implementation names:

| Previous atom | Short implementation name | Role |
|---|---|---|
| `SourceInterpret` | `read_source` | normalize source description |
| `SourceSelect` | `choose_source` | choose candidate/fallback |
| `MaterialReadOrFetch` | `fetch` | obtain material handle |
| `MaterialValidate` | `check` | integrity/trust validation |
| `MaterialIdentify` | `kind` | detect file/dir/archive/etc. |
| `MaterialTransform` | `unpack` or `prepare` | identity/extract/shape material |
| `TargetPlan` | `plan_target` | optional target mutation plan |
| `TargetMutate` | `write_target` | copy/link/replace target |
| `ActivationMutate` | `activate` | symlink/shim/activation side effect |
| `RecoverySnapshot` | `snapshot` | capture rollback state |
| `EvidenceBuild` | `build_receipt` | aggregate facts |
| `EvidencePersist` | `save_receipt` | persist receipt/facts |
| `EvidenceInspect` | `inspect` | query saved facts |

These names are deliberately mundane. They should not become public nouns unless the public API needs them.

### Level E — Mechanism

Question:

```text
How is an atom implemented?
```

Private helpers. No domain vocabulary unless unavoidable.

Examples:

```text
copy_dir
link_or_copy
atomic_write
sanitize_path
hash_stream
retry_delay
open_zip
read_tar
write_json
```

Mechanisms must not leak upward as user concepts.

Bad upward leaks:

```text
StoreKey in caller API
Workspace in user workflow
ArchiveReport required to call install
FetchReceipt required to call store
InstallFlow<S> required to install
```

## Semantic vs implementation ownership

### Semantic owner

The semantic owner names what something means to the user.

Examples:

```text
Source means where material comes from.
Target means where material is applied.
Op means create/replace/upgrade/uninstall style intent.
Need means constraints: network, digest, trust, rollback.
Receipt means what happened.
```

### Implementation owner

The implementation owner performs mechanics behind a semantic concept.

Examples:

```text
net fetch implements Acquire for remote Source.
archive unpack implements Prepare for archive material.
fs write implements Apply for file/dir Target.
persist save implements receipt retention.
```

### Rule

Do not create an implementation-shaped type when a semantic type already carries the meaning.

Examples:

| Avoid | Keep |
|---|---|
| `InstallSpec` as peer to `Application` | `Application` / `Op` |
| `ActivationTarget` as peer to `Target` | `Target` plus activation option |
| `FetchSource` as peer to `Source` | `Source` |
| `SourceDefinition` as peer to `Source` | `Source` |
| `ResourceLocator` as peer to `Source` | `Source` |
| `ResolvedLocator` as peer state | selected source evidence |
| `StoredArtifact` as peer material | material/evidence depending on role |
| `ExtractedArtifact` as peer prepared type | prepared material/evidence |

## Duplicate-semantic type audit

These current pairs/groups represent similar semantics and should collapse before migration.

### Source duplicates

Current surfaces:

```text
ResourceLocator
SourceDefinition
RemoteSource
LocalSource
FetchSource
ResolvedSourceCandidate
SourceSet
SourcePlan<S>
PlannedSources
```

Semantic split:

```text
Source       # caller intent
ChosenSource # acquire evidence/internal selected candidate, if needed
```

Implementation details:

```text
mirror expansion
URL join
git rev/subpath
local path normalization
fallback order
```

Delete/internal:

```text
SourcePlan<S>
PlannedSources
SourceSpec
FetchSource
ResolvedSourceCandidate as public caller API
```

Naming rule:

```text
Use Source for intent.
Use ChosenSource only if evidence needs to record actual selected candidate.
Do not use Locator, Definition, Spec, Plan, FetchSource for the same semantic layer.
```

### Resource/item duplicates

Current surfaces:

```text
ResourceId
ResourceSpec
Resource<S>
RequestedResource
ResolvedResource
ResolvedResourceContext
ArtifactDescriptor
```

Semantic split:

```text
Item      # what is being managed
Version   # optional user intent or resolved fact
Receipt   # resolved facts after work
```

Implementation details:

```text
parse id
select version
validate version requirement
format display label
```

Delete/internal:

```text
RequestedResource
ResolvedResource
ResolvedResourceContext
ResourceBehaviorContract
```

Naming rule:

```text
Use one caller noun for managed thing: Item or Resource.
Do not expose Requested/Resolved as separate caller types unless the caller must branch between them.
Resolved facts go into Receipt/Evidence.
```

### Material/prepared artifact duplicates

Current surfaces:

```text
ArtifactForm
MaterializationSpec
FetchReceipt.destination
Extracted
WorkspaceExtraction
StoredArtifact
ExtractedArtifact
InstallInput
```

Semantic split:

```text
Material  # acquired thing before preparation
Prepared  # thing Apply can consume
```

Implementation details:

```text
file path
directory path
archive path
extracted root
store path
```

Delete/internal:

```text
InstallInput public enum
IntoInstallInput
StoredArtifact as install input
ExtractedArtifact as install input
WorkspaceExtraction if it duplicates Prepared
```

Naming rule:

```text
Use Material for Acquire output.
Use Prepared for Prepare output.
Do not create Staged/Stored/Extracted peer types unless their invariants differ and are consumed independently.
```

### Operation/install duplicates

Current surfaces:

```text
Operation
InstallMode
InstallSpec
InstallPlanningRequest
InstallWorkflowVariant
InstallCapabilities
InstallPlanReport
InstallFlow<S>
PlannedInstall
StagedInstall
InstalledInstall
ActivatedInstall
LifecycleOperationPhase
```

Semantic split:

```text
Op       # caller intent: create/replace/upgrade/uninstall
Target   # destination and optional activation endpoint
Need     # constraints: network, rollback, writable scope
Receipt  # outcome
```

Implementation details:

```text
stage temp dir
commit replace
state upsert
activation record append
rollback snapshot
```

Delete/internal:

```text
InstallSpec
InstallPlanningRequest
InstallWorkflowVariant unless diagnostic-only
InstallFlow<S> public typestate
Planned/Staged/Installed/Activated public workflow states
```

Naming rule:

```text
Use Op for intent.
Use Receipt for outcome.
Use internal phase names only inside apply implementation.
```

### Evidence/receipt duplicates

Current surfaces:

```text
FetchReceipt
ArchiveReport
StoreProvenance
StoreMetadataRecord
InstallReceipt
ActivationReceipt
BackupReceipt
RollbackReceipt
RestoreReceipt
UninstallReceipt
LifecycleOperationReceipt
LockFile
LockDiff
ResourceInspectionReport
```

Semantic split:

```text
Receipt       # main operation outcome
Evidence      # optional structured facts inside receipt
SavedReceipt  # persisted reference, if persistence is enabled
Report        # inspect/query output, not operation input
```

Implementation details:

```text
JSON layout
key derivation
atomic file write
lock diff serialization
```

Delete/internal:

```text
StoreProvenance as caller glue
StoreMetadataRecord as caller object
LockFile as apply path input
FetchReceipt/ArchiveReport required as next-step inputs
```

Naming rule:

```text
Use Receipt for operation result.
Use Evidence for nested facts.
Use Report only for read-only inspection.
Do not require a Report/Receipt from one phase as manual caller input to the next phase.
```

### Store/state duplicates

Current surfaces:

```text
StoreRoots
StoreReady
StoreKey
StateReady
StateAnalysisIndex
ResourceRecordPatch
IntoResourceUpsert
StoredArtifact
ExtractedArtifact
```

Semantic split:

```text
Memory   # optional retained facts
State    # current known installed/applied condition, queried later
```

But first slice should not expose either as public API.

Implementation details:

```text
root paths
metadata schema
key derivation
record patch
indexing
```

Delete/internal:

```text
StoreKey as caller object
IntoResourceUpsert
StoredArtifact/ExtractedArtifact as cross-crate glue
StateReady as main workflow input
```

Naming rule:

```text
Prefer save/load/inspect functions behind persist/inspect modules.
Avoid Store/State public nouns until the product requires explicit memory management.
```

## Layered type budget

A small type budget prevents semantic duplicates.

### Public semantic budget

Allowed public nouns for the main operation:

```text
App or Application
Item or Resource
Source
Target
Op or Operation
Need or Requirements
Receipt
Evidence
Pipeline
```

No peer public nouns for the same roles.

### Public behavior budget

Allowed public behavior traits:

```text
Acquire
Prepare
Apply
```

No peer public traits named:

```text
Fetcher
SourceAdapter
EntrySource
Activator
IntoInstallInput
IntoArtifactRegistration
IntoExtractRegistration
IntoResourceUpsert
```

`Activator` may exist internally if activation has real interchangeable implementations.

### Internal atom budget

Allowed internal atom names should be short verbs/nouns:

```text
source
select
fetch
check
kind
unpack
write
activate
snapshot
receipt
save
inspect
```

Avoid long class-like names unless the invariant earns it.

### Mechanism helper budget

Mechanism helpers should stay descriptive and private:

```text
copy_dir
replace_dir
atomic_write
hash_file
sanitize_path
retry_delay
```

No mechanism helper should become the user-facing design center.

## Naming rules

### Prefer short semantic names

Prefer:

```text
App
Item
Source
Target
Op
Need
Receipt
Evidence
```

Accept current longer names while code is still exploratory:

```text
Application
Resource
Operation
Requirements
EvidencePolicy
```

But do not introduce more long peer nouns.

### Avoid suffix bloat

Avoid when the suffix does not add a new semantic layer:

```text
Spec
Definition
Descriptor
Context
Plan
Request
Ready
Registration
Input
Info
Data
Manager
Handler
Adapter
```

Allowed only when the suffix has a precise role:

```text
Receipt  # operation happened
Report   # read-only inspection/query
Policy   # user-selected behavior rule
Options  # implementation knobs, internal/advanced
```

### Use one name per meaning

If two names answer the same question, delete one.

Examples:

```text
SourceDefinition + ResourceLocator -> Source
FetchSource + ResolvedSourceCandidate -> ChosenSource or evidence
InstallSpec + Application -> Application
InstallMode + OperationMode -> Op.mode
ActivationTarget + Target -> Target activation field
```

### Verbs for implementation, nouns for semantics

Semantic public types are nouns:

```text
Source
Target
Receipt
```

Implementation atoms are verbs or verb phrases:

```text
fetch
check
unpack
write
activate
save
```

This prevents implementation mechanics from becoming fake domain concepts.

### Avoid crate-name vocabulary in final API

Avoid final public names that encode old crate split:

```text
Fetch*
Archive*
Store*
Install*
State*
```

Exceptions:

- `archive` can be a feature/module because archive extraction is a real mechanism family.
- `install` may remain in docs if product language demands it, but public operation should not require `InstallFlow`/`InstallSpec` vocabulary.
- `state`/`store` should not be in the first user-facing path.

## Unified architecture sketch

### Public use shape

Target ergonomic shape, not code to implement now:

```text
let app = App {
    item,
    source,
    target,
    op,
    need,
};

let receipt = Pipeline::new(acquire, prepare, apply).run(app)?;
```

The caller should not manually do:

```text
resolve resource
plan sources
fetch receipt
archive report
extract registration
store key
install input
install flow stage
commit
activate
state upsert
```

### Internal composition shape

Implementation may internally perform:

```text
read_source
choose_source
fetch
check
kind
unpack
snapshot
write_target
activate
build_receipt
save_receipt
```

But those are implementation steps, not user choreography.

## Per-area atomic redesign

### Source/acquire

Semantic API:

```text
Source
Need.network
Need.trust/checks
```

Internal atoms:

```text
read_source
choose_source
fetch
check
```

Keep:

```text
URL/path validation
mirror expansion
ordered fallback/race if really used
HTTP streaming
retry/resume mechanics
checksum/signature verification
```

Delete/internal:

```text
SourcePlan<S>
PlannedSources
SourceSpec
SourceAdapter if duplicating Acquire
FetchSource
FetchReceipt as manual next-step object
```

Naming target:

```text
Source -> ChosenSource evidence -> Material
```

Not:

```text
Locator -> Definition -> Spec -> Plan -> Candidate -> FetchSource
```

### Prepare/archive

Semantic API:

```text
Need.unpack or Need.shape if caller actually chooses it
```

Internal atoms:

```text
kind
unpack
check_entry
```

Keep:

```text
format detection
zip/tar decoder
safe path validation
permission handling
entry evidence
```

Delete/internal:

```text
ArchiveReport as caller input
EntrySource public trait
WorkspaceExtraction if it duplicates Prepared
Extracted as peer API unless it owns unique invariant
```

Naming target:

```text
Material -> Prepared
Prepared evidence includes entries/root/format
```

Not:

```text
ArchiveReport + root tuple -> ExtractedTreeRegistration -> ExtractRegistration
```

### Target/apply

Semantic API:

```text
Target
Op
Need.rollback
```

Internal atoms:

```text
snapshot
write_target
activate
```

Keep:

```text
create-only vs replace/upgrade behavior
copy/link/replace dir
activation symlink/shim/copy
rollback snapshot/restore
platform-specific errors
```

Delete/internal:

```text
InstallSpec
InstallInput
IntoInstallInput
InstallPlanningRequest
InstallFlow<S>
Planned/Staged/Installed/Activated public types
```

Naming target:

```text
Prepared -> Apply -> Receipt
```

Not:

```text
InstallInput -> InstallFlow<Planned> -> StagedInstall -> InstalledInstall -> ActivatedInstall
```

### Memory/persist/inspect

Semantic API:

```text
Receipt
Evidence
```

Public optional API later only if needed:

```text
save(receipt)
inspect(item)
```

Internal atoms:

```text
build_receipt
save_receipt
inspect
```

Keep:

```text
metadata schema
atomic write
key derivation
lock diff/inspection logic
repair planning if product needs it
```

Delete/internal:

```text
StoreKey as caller input
StoreReady as main operation context
StateReady as main operation context
IntoArtifactRegistration
IntoExtractRegistration
IntoResourceUpsert
StoredArtifact/ExtractedArtifact as cross-boundary input types
```

Naming target:

```text
Receipt -> SavedReceipt -> Report
```

Not:

```text
StoreProvenance -> StoreMetadataRecord -> LockFile -> ResourceRecordPatch as caller flow
```

## Implementation-vs-glue decision tests

Before moving any code, classify it with these tests.

### Implementation test

A surface is implementation if it answers:

```text
Does this perform work or enforce an invariant even if all old crate boundaries disappear?
```

If yes, keep the behavior, possibly with a shorter internal name.

Examples:

```text
sanitize archive path
copy file atomically
retry HTTP stream
compute digest
restore backup
```

### Glue test

A surface is glue if it answers:

```text
Does this mainly convert one crate's product into another crate's input?
```

If yes, delete or internalize.

Examples:

```text
IntoArtifactRegistration
IntoExtractRegistration
IntoInstallInput
IntoResourceUpsert
ExtractedTreeRegistration
FetchReceipt -> store registration
ArchiveReport + root -> extract registration
```

### Semantic duplicate test

A surface is a duplicate if it answers a question already answered by another type at the same layer.

Examples:

```text
SourceDefinition vs Source
ResourceLocator vs Source
InstallSpec vs Application
ActivationTarget vs Target
FetchSource vs ChosenSource/Source
```

If yes, choose the clearer name and delete the other.

### Layer leak test

A surface leaks layers if a low-level mechanism appears in caller workflow.

Examples:

```text
Workspace
StoreKey
StateReady
ArchiveReport
FetchReceipt
InstallFlow<S>
```

If yes, move it below behavior level or make it evidence only.

## Concrete rename/collapse candidates

These are design decisions, not implementation instructions yet.

| Current family | Final semantic owner | Notes |
|---|---|---|
| `ResourceLocator`, `SourceDefinition`, `FetchSource` | `Source` | one source vocabulary |
| `ResolvedSourceCandidate` | `ChosenSource` or evidence | only if selected source must be recorded |
| `ResourceSpec`, `InstallSpec` | `App` / `Application` | one request object |
| `InstallMode`, `OperationMode` | `Op.mode` | one operation mode |
| `ActivationTarget` | `Target` | activation endpoint is target semantics |
| `MaterializationSpec`, `ArtifactForm`, `UnpackPolicy` | `Need` only if user-controlled | otherwise internal prepare choice |
| `FetchReceipt`, `ArchiveReport`, `InstallReceipt` | `Evidence` inside `Receipt` | not manual input chain |
| `StoreKey`, `StoreProvenance`, `StoreMetadataRecord` | `save_receipt` internals / saved evidence | not public apply input |
| `LockFile`, `LockDiff`, `InspectionReport` | `Report` | query output only |

## Current-stage non-goals

Do not decide these yet:

```text
file organization
module names
private helper layout
local-path implementation shape
archive/net/persist feature layout
```

Those are downstream consequences. They should not drive the domain model.

Do decide behavior relations first:

```text
which morphisms exist
which semantic states each morphism consumes/produces
which compositions are valid
which compositions are forbidden
which evidence makes the composition observable
```

## Behavior-spec migration plan

The next design artifact should not be a local-target implementation design.

Next document:

```text
docs/report/pulith-behavior-morphism-spec.md
```

Purpose:

```text
Define Pulith's behavior morphisms from DDD first principles before migrating concrete semantics or implementations.
```

It should specify behavior relations, not examples.

Required sections:

### Behavior catalog

For each behavior:

```text
name
source semantic state
target semantic state
valid previous behaviors
valid next behaviors
forbidden compositions
required evidence
laws/invariants
```

Initial behavior candidates:

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

These are not final API names. They are domain relations for analysis.

### Semantic states derived from behavior

Do not start with old type names. Derive only states required by behavior composition:

```text
Declared intent
Offered source
Chosen source
Acquired material
Verified material
Prepared material
Applied target
Remembered fact
Observed state
Repair plan
```

Then map old public types into these states.

### Composition laws

Examples of laws to define:

```text
Apply cannot consume unverified material when Need requires verification.
Prepare cannot hide source/evidence facts needed by Receipt.
Remember cannot create lifecycle truth that Apply did not produce.
Inspect observes remembered/applied facts; it does not mutate them.
Repair proposes or performs a new behavior; it does not silently rewrite history.
```

### Old surface migration by behavior semantics

After behaviors and states are defined, classify old surfaces by the behavior relation they support:

```text
ResourceLocator / SourceDefinition / SourcePlan -> Offer/Select semantics
Fetcher / FetchReceipt -> Acquire/Verify semantics
ArchiveReport / ExtractedTreeRegistration -> Prepare/Evidence semantics
InstallSpec / InstallFlow / InstallReceipt -> Apply/Receipt semantics
StoreKey / StoreMetadataRecord / LockFile -> Remember/Inspect semantics
```

This is migration of behavior-defined semantics, not migration of implementations.

### No examples as definitions

Examples may be used only to check coverage after the behavior spec exists.

They must not define the behavior.

Bad:

```text
Local path install defines Acquire/Prepare/Apply.
```

Good:

```text
Acquire is a morphism from Offered/Chosen source to Acquired material plus evidence.
Local path is one implementation that satisfies that morphism.
```

## Revised next step

Write:

```text
docs/report/pulith-behavior-morphism-spec.md
```

Do not write:

```text
docs/report/pulith-local-target-atomic-design.md
```

until behavior morphisms and behavior-defined semantic states are accepted.

## Rules before code

No implementation migration until the behavior-morphism spec exists and is accepted.

When code begins later:

1. migrate behavior-defined semantic states first;
2. then attach behavior implementations to those states;
3. delete glue, no compatibility aliases;
4. keep semantic names singular;
5. keep mechanisms private;
6. verify with focused Rust checks plus structural absence checks.

## Health checklist

This document is healthy if:

- it defines behavior-first abstraction levels;
- it treats behavior as morphism/composition, not examples;
- it says behavior-defined semantics migrate before semantic-owned behavior implementation;
- it separates semantics from implementation mechanisms;
- it identifies duplicate semantic types;
- it prefers short names;
- it keeps only `Acquire`, `Prepare`, `Apply` as public behavior traits;
- it treats conversion traits and typestate workflow surfaces as glue;
- it explicitly avoids bottom-level design/file organization at this stage;
- it defines `docs/report/pulith-behavior-morphism-spec.md` as the next design document before code.
