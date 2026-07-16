# Pulith Behavior Morphism Spec

## Status

Design/specification only. No Rust code changes are authorized by this document.

This document follows `docs/report/pulith-unified-atomic-architecture-analysis.md` and shifts analysis from type/file/module design to DDD behavior relations.

Current rule:

```text
Behavior first.
Semantics derived from behavior.
Implementation last.
Examples check coverage; they do not define behavior.
```

## Interpretation

A Pulith behavior is a domain morphism:

```text
source semantic state -> target semantic state + evidence
```

A behavior is defined by composition:

```text
valid previous behaviors
valid next behaviors
forbidden compositions
laws/invariants preserved by composition
observable evidence
```

This is the practical Yoneda-style lens:

```text
A behavior is understood by all valid ways adjacent behaviors can compose with it or observe it.
```

## Behavior catalog

The names below are analysis names, not final API names.

### Declare

Morphism:

```text
User intent -> Declared intent
```

Consumes:

```text
caller need
managed item identity
source offer expression
target expression
operation intent
requirements
```

Produces:

```text
Declared intent
```

Valid previous behaviors:

```text
none
```

Valid next behaviors:

```text
Offer
Remember, only if declarations become durable desired state later
```

Forbidden compositions:

```text
Declare -> Apply
Declare -> Remember as execution fact
Declare -> Inspect as if it were observed state
```

Evidence:

```text
none required for execution; optional validation diagnostics
```

Laws:

```text
Declaration contains intent, not proof.
Declaration does not select a source.
Declaration does not imply material exists.
Declaration does not mutate target or memory.
```

Old surfaces mapped here:

```text
ResourceSpec
InstallSpec
Application skeleton
Operation / InstallMode intent fields
Requirements / EvidencePolicy intent fields
```

### Offer

Morphism:

```text
Declared intent -> Offered source set
```

Consumes:

```text
Declared intent
source expression
requirements that constrain allowable source offers
```

Produces:

```text
Offered source set
```

Valid previous behaviors:

```text
Declare
```

Valid next behaviors:

```text
Select
Acquire, only when the offered set has exactly one usable offer and no selection policy is needed
Reject, if no offer satisfies requirements
```

Forbidden compositions:

```text
Offer -> Prepare
Offer -> Apply
Offer -> Remember as acquired material
```

Evidence:

```text
offers considered
invalid offers rejected
reason for empty offer set
```

Laws:

```text
Offer normalizes possible sources but does not perform transfer.
Offer may expand mirrors/candidates but does not claim one was used.
Offer cannot invent a source outside declared intent and policy.
```

Old surfaces mapped here:

```text
ResourceLocator
SourceDefinition
SourceSet
SourceSpec
RemoteSource
LocalSource
MirrorSource
GitSource
HttpAssetSource
```

### Select

Morphism:

```text
Offered source set -> Chosen source
```

Consumes:

```text
Offered source set
selection policy
availability constraints
```

Produces:

```text
Chosen source
```

Valid previous behaviors:

```text
Offer
Retry/Repair, if prior acquisition failure requests a different offer
```

Valid next behaviors:

```text
Acquire
Reject, if no candidate can be chosen
```

Forbidden compositions:

```text
Select -> Prepare
Select -> Apply
Select -> Remember as material fact
```

Evidence:

```text
chosen candidate
selection rule
rejected candidates and reasons, if known
```

Laws:

```text
Select chooses among offers; it does not obtain bytes or trees.
Select is repeatable under the same offered set and deterministic policy.
Race-style selection must still emit the chosen candidate and losing candidates if observable.
```

Old surfaces mapped here:

```text
SelectionStrategy
PlannedSources
SourcePlan<Planned>
ResolvedSourceCandidate
MultiSourceFetcher selection loop pieces
```

### Acquire

Morphism:

```text
Chosen source -> Acquired material
```

Consumes:

```text
Chosen source
network/offline requirement
workspace/cache policy if needed internally
```

Produces:

```text
Acquired material
Acquisition evidence
```

Valid previous behaviors:

```text
Select
Offer, only if selection is identity
Repair/Retry, if previous acquire failed and a new chosen source exists
```

Valid next behaviors:

```text
Verify
Prepare, only if no verification is required
Reject, if acquisition fails
```

Forbidden compositions:

```text
Acquire -> Apply when material shape is not prepared
Acquire -> Remember as lifecycle state
Acquire -> Inspect as installed state
```

Evidence:

```text
chosen source
material location/handle
byte count when meaningful
transfer metadata
failure reason when failed
```

Laws:

```text
Acquire obtains material but does not certify trust unless Verify is identity by policy.
Acquire does not unpack archives.
Acquire does not mutate target.
Acquire evidence can be observed by Receipt but should not be manual glue for Prepare.
```

Old surfaces mapped here:

```text
Fetcher
HttpClient
ReqwestClient
FetchOptions transfer mechanics
FetchReceipt acquisition facts
FetchSource duplicate source vocabulary
```

### Verify

Morphism:

```text
Acquired material -> Verified material
```

Consumes:

```text
Acquired material
Need.checks / trust policy / digest requirement
```

Produces:

```text
Verified material
Verification evidence
```

Valid previous behaviors:

```text
Acquire
Prepare, only for post-extraction verification policies if explicitly defined later
```

Valid next behaviors:

```text
Prepare
Reject, if checks fail
```

Forbidden compositions:

```text
Verify -> Select
Verify -> Offer
Verify -> Apply if material still requires preparation
```

Evidence:

```text
digest algorithm/value
signature/trust result
policy applied
reason for failure
```

Laws:

```text
Verify never changes material identity.
Verify records facts about material and policy.
Verify cannot silently downgrade a required check.
If Need requires verification, Apply cannot consume unverified material.
```

Old surfaces mapped here:

```text
ValidDigest
DigestAlgorithm
VerificationRequirement
TrustPolicy
TrustDecision
checksum/signature code
```

### Prepare

Morphism:

```text
Verified material -> Prepared material
```

Consumes:

```text
Verified material
preparation requirements
```

Produces:

```text
Prepared material
Preparation evidence
```

Valid previous behaviors:

```text
Verify
Acquire, only when verification is identity/not required
```

Valid next behaviors:

```text
Apply
Remember, only for caching prepared material facts, not lifecycle truth
Reject, if preparation fails
```

Forbidden compositions:

```text
Prepare -> Select
Prepare -> Acquire
Prepare -> Apply if preparation evidence violates required safety policy
```

Evidence:

```text
material kind
format detected
entries extracted or identity shape
sanitized path decisions
prepared root/handle
```

Laws:

```text
Prepare transforms material shape, not target state.
Prepare must preserve evidence needed by Receipt.
Prepare cannot hide unsafe archive path decisions.
Prepare output is what Apply can consume.
```

Old surfaces mapped here:

```text
ArchiveFormat
ArchiveReport
Entry / EntryKind
Extracted
WorkspaceExtraction
ExtractedTreeRegistration as glue to delete
ExtractRegistration as glue to delete
```

### Apply

Morphism:

```text
Prepared material -> Applied target
```

Consumes:

```text
Prepared material
Target
Operation intent
Need.rollback / mutation constraints
```

Produces:

```text
Applied target
Apply evidence
Receipt core facts
```

Valid previous behaviors:

```text
Prepare
Verify, only when verified material is already prepared by identity
```

Valid next behaviors:

```text
Remember
Inspect
Repair, if apply produces recoverable partial/failure state
Forget, for uninstall-like operations later
```

Forbidden compositions:

```text
Apply -> Offer
Apply -> Select
Apply -> Acquire as hidden fallback
Apply -> Remember facts that did not occur
```

Evidence:

```text
target path
operation mode
created/replaced/activated facts
rollback snapshot facts
platform-specific mutation errors
```

Laws:

```text
Apply is the mutation boundary.
Apply does not choose sources.
Apply does not invent acquisition/preparation evidence.
Apply must produce enough facts for Remember/Inspect to observe what happened.
Rollback evidence must describe what was captured before mutation.
```

Old surfaces mapped here:

```text
InstallInput as glue to delete
IntoInstallInput as glue to delete
InstallFlow<S> as public choreography to delete/internalize
InstallSpec as duplicate declaration/apply intent
InstallReceipt / ActivationReceipt / RollbackReceipt as apply evidence
Activator as internal interchangeable implementation only if needed
```

### Remember

Morphism:

```text
Receipt / Applied target -> Remembered fact
```

Consumes:

```text
Receipt
Evidence retention policy
```

Produces:

```text
Remembered fact
Saved receipt reference, if persistence is enabled
```

Valid previous behaviors:

```text
Apply
Prepare, only for cache/prepared-material memory, not lifecycle truth
Declare, only for desired-state declarations if that product exists later
```

Valid next behaviors:

```text
Inspect
Repair
Forget
```

Forbidden compositions:

```text
Remember -> Apply as if memory were prepared material
Remember -> Offer as if persisted facts were source intent
Remember creating lifecycle truth not produced by Apply
```

Evidence:

```text
saved receipt location/key
schema version
stored evidence summary
```

Laws:

```text
Remember persists facts; it does not perform source acquisition or target mutation.
Memory cannot be more authoritative than the evidence it stores.
Store keys are implementation details, not behavior inputs.
```

Old surfaces mapped here:

```text
StoreKey
StoreRoots
StoreReady
StoreProvenance
StoreMetadataRecord
IntoArtifactRegistration as glue to delete
IntoExtractRegistration as glue to delete
```

### Inspect

Morphism:

```text
Remembered fact / Applied target -> Observed state
```

Consumes:

```text
remembered facts
optional live target observation
```

Produces:

```text
Observed state
Inspection report
```

Valid previous behaviors:

```text
Remember
Apply, for immediate inspect without durable persistence
```

Valid next behaviors:

```text
Repair
Forget
Declare, if user turns observed state into desired intent later
```

Forbidden compositions:

```text
Inspect -> Apply without a new declared operation
Inspect -> Remember as if observation were mutation evidence
```

Evidence:

```text
observed paths/facts
missing data
ownership/activation observations
state-vs-evidence differences
```

Laws:

```text
Inspect observes; it does not mutate.
Inspect must distinguish missing evidence from negative evidence.
Inspect reports uncertainty explicitly.
```

Old surfaces mapped here:

```text
ResourceInspectionReport
ResourceInspectionFinding
ActivationOwnershipReport
LockFile
LockDiff
StateAnalysisIndex
```

### Repair

Morphism:

```text
Observed state -> Declared intent or Applied target
```

Consumes:

```text
Observed state
repair policy
```

Produces one of:

```text
Repair plan
Declared repair intent
Applied repair mutation plus receipt
```

Valid previous behaviors:

```text
Inspect
Apply, only for immediate rollback/recovery path with explicit failure evidence
```

Valid next behaviors:

```text
Declare
Apply
Remember
```

Forbidden compositions:

```text
Repair silently rewriting Remembered facts
Repair mutating target without new Apply evidence
Repair masking missing evidence as successful repair
```

Evidence:

```text
repair plan
actions proposed/performed
facts restored or deleted
unresolved gaps
```

Laws:

```text
Repair is a new behavior, not a hidden side effect of Inspect.
Repair must preserve prior evidence or explicitly record replacement facts.
Repair cannot claim authority beyond observed and remembered facts.
```

Old surfaces mapped here:

```text
ResourceRepairPlan
ResourceRepairAction
RollbackReceipt
RestoreReceipt
BackupReceipt
UninstallReceipt, for forget/remove variants
```

### Forget

Morphism:

```text
Remembered fact / Applied target -> Forgotten fact or removed target
```

Consumes:

```text
remembered facts and/or applied target
forget/uninstall policy
```

Produces:

```text
Forget evidence
Receipt
```

Valid previous behaviors:

```text
Inspect
Remember
Apply, for immediate uninstall/cleanup behavior if explicitly requested
```

Valid next behaviors:

```text
Remember, to retain forget receipt
Inspect, to confirm absence
```

Forbidden compositions:

```text
Forget without explicit policy
Forget that erases evidence without a receipt when retention is required
Forget that removes target while leaving lifecycle facts claiming active state
```

Evidence:

```text
removed target facts
removed activation facts
removed remembered facts
retained tombstone/receipt if policy requires
```

Laws:

```text
Forget is explicit behavior, not cleanup hidden inside Apply.
Forget must preserve enough evidence to explain absence when retention requires it.
```

Old surfaces mapped here:

```text
UninstallOptions
UninstallDisposition
UninstallReceipt
state remove helpers
store prune/retention planning
```

## Behavior-defined semantic states

These states are derived from behavior composition. They are not final type names yet.

| Semantic state | Defined by incoming behavior | Valid outgoing behaviors | Main old surfaces to map |
|---|---|---|---|
| Declared intent | `Declare` | `Offer`, maybe `Remember` later | `Application`, `ResourceSpec`, `InstallSpec` |
| Offered source set | `Offer` | `Select`, maybe identity `Acquire` | `SourceDefinition`, `SourceSet`, `SourcePlan<Unplanned>` |
| Chosen source | `Select` | `Acquire` | `ResolvedSourceCandidate`, `PlannedSources` |
| Acquired material | `Acquire` | `Verify`, identity `Prepare` | `FetchReceipt.destination`, local path handles |
| Verified material | `Verify` | `Prepare` | `ValidDigest`, `TrustDecision` facts |
| Prepared material | `Prepare` | `Apply`, maybe cache `Remember` | `Extracted`, `WorkspaceExtraction`, `ExtractedArtifact` |
| Applied target | `Apply` | `Remember`, `Inspect`, `Repair`, `Forget` | `InstallReceipt`, `ActivationReceipt`, lifecycle facts |
| Remembered fact | `Remember` | `Inspect`, `Repair`, `Forget` | `StoreMetadataRecord`, `LockFile`, resource records |
| Observed state | `Inspect` | `Repair`, `Forget`, `Declare` | `ResourceInspectionReport`, ownership reports |
| Repair plan | `Repair` as plan-only | `Declare`, `Apply` | `ResourceRepairPlan` |

## Migration implication

Old objects should be migrated only after assigning them to behavior-defined semantic states.

Examples:

```text
ResourceLocator is not migrated to Source because it is old source vocabulary.
It is migrated only if Offer needs a declared/offered source state with those facts.

FetchReceipt is not migrated as a type because fetch had a receipt.
Its facts are migrated only if Acquire/Verify evidence needs them.

ArchiveReport is not migrated because archives exist.
Its facts are migrated only if Prepare evidence must preserve entries/format/path decisions.

InstallFlow<S> is not migrated because it encodes staging order.
Its real behavior is decomposed into Apply/Repair/Forget laws and internal choreography.

StoreKey is not migrated because persistence exists.
Its meaning is internal to Remember unless another behavior must compose with it.
```

## Immediate next analysis task

Next analysis should classify every old public surface into:

```text
behavior relation
semantic state
required facts
composition laws it supports
whether it is behavior-defined semantic, implementation, evidence, or glue
```

Not into:

```text
new module
new file
new implementation function
```

## Verification checklist

This document is healthy if:

- behaviors are defined without examples;
- every behavior has source state, target state, compositions, evidence, and laws;
- semantic states are derived from behavior composition;
- implementation and file layout are explicitly out of scope;
- old surfaces are mapped by behavior semantics, not by crate name.
