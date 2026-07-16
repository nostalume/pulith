# Phase A DDD Crate Layout Evaluation

## Status

Design/evaluation report only. Do not rename crates, fold crates, split modules, or edit Rust code from this report alone.

This report evaluates each current crate against the Phase A domain concept model:

```text
Intent -> Offer -> Evidence -> Memory -> Mutation -> State -> Inspection/Repair
```

Expanded Pulith concepts:

```text
Resource Intent
  -> Source Offer
  -> Materialized Evidence
  -> Artifact Memory
  -> Mutation Workflow
  -> Lifecycle State
  -> Inspection / Repair / Retention views
```

The goal is to propose a concept-driven crate layout, not a line-count-driven layout.

## Current workspace evidence

Current active crates from `cargo metadata` and source inventory:

```text
pulith-version        files=  2 loc=  634 deps=-
pulith-resource       files=  1 loc= 1117 deps=pulith-version
pulith-source         files=  1 loc=  442 deps=pulith-resource
pulith-fs             files= 14 loc= 1493 deps=-
pulith-verify         files=  4 loc=  424 deps=-
pulith-archive        files=  9 loc= 2275 deps=pulith-fs
pulith-fetch          files= 32 loc= 9819 deps=pulith-fs,pulith-resource,pulith-source,pulith-verify
pulith-store          files=  1 loc= 1280 deps=pulith-archive,pulith-fetch,pulith-fs,pulith-resource
pulith-state          files=  2 loc= 2887 deps=pulith-fs,pulith-resource,pulith-store
pulith-install        files=  1 loc= 2496 deps=pulith-archive,pulith-fetch,pulith-fs,pulith-resource,pulith-state,pulith-store,pulith-source
```

Already removed concept-satellite crates:

```text
pulith-shim          -> removed; activation/resolution belongs to Mutation Workflow
pulith-serde-backend -> removed; persistence format belongs to each schema owner
pulith-lock          -> removed; lock export is a Lifecycle State view
pulith-platform      -> removed; platform behavior is owner-local where it changes contracts
```

## Proposed concept-first crate layout

The proposed crate layout is not one crate per concept. Some concepts contain safety boundaries that earn separate crates.

```text
Resource Intent bounded context
  pulith-version
  pulith-resource

Source Offer bounded context
  pulith-source

Materialized Evidence bounded context
  pulith-fs
  pulith-verify
  pulith-archive
  pulith-fetch

Artifact Memory bounded context
  pulith-store

Lifecycle State / Inspection bounded context
  pulith-state

Mutation Workflow bounded context
  pulith-install

Caller Policy / Manager Adapter bounded context
  examples/pulith-backend-example
  examples/runtime-manager
```

This is the recommended **near-term crate topology**. It keeps the current post-cleanup crates, but redefines why they exist and what each must not own.

## Why not collapse to one crate per DDD concept?

A naive DDD collapse would produce something like:

```text
pulith-intent
pulith-offer
pulith-evidence
pulith-memory
pulith-mutation
```

Reject for now.

Reason:

- `Materialized Evidence` contains several different side-effect/safety contracts. `pulith-fs`, `pulith-verify`, `pulith-archive`, and `pulith-fetch` should not be merged just because they belong to the same concept stage.
- `Artifact Memory` and `Lifecycle State` are adjacent but separate: local artifact provenance is not installed lifecycle truth.
- `Resource Intent` could theoretically combine `version` and `resource`, but `pulith-version` is a pure reusable semantic primitive with no I/O and a simple dependency profile.
- A coarse collapse would reduce crate count while hiding safety and policy boundaries.

## Crate-by-crate evaluation

### `pulith-version`

Current concept:

```text
Resource Intent / version semantics
```

Current public surface evidence:

- `VersionKind`
- `CalVer`
- `Partial`
- `VersionRequirement`
- `VersionPreference`
- `SelectionPolicy`
- `select_preferred(...)`

Evaluation:

- This crate is small but conceptually clean.
- It is pure, reusable, and has no I/O or mutation.
- It supports Resource Intent without importing resource semantics.

Recommendation:

```text
Keep as `pulith-version`.
```

Alternative considered:

- Fold into `pulith-resource`.

Reject for now because version selection is a reusable semantic primitive and a dependency-free leaf. Folding it would make Resource Intent heavier without deleting false abstraction.

Future pressure:

- If `pulith-version` is never used independently outside `pulith-resource` and examples, revisit as an internal `resource::version` module. Do not do this before source/fetch/materialization evaluation.

### `pulith-resource`

Current concept:

```text
Resource Intent
```

Current public surface evidence:

- `ResourceId`
- `ResourceLocator`
- `VersionSelector`
- `VerificationRequirement`
- `TrustPolicy`
- `ResourceBehaviorContract`
- `RequestedResource`
- `ResolvedResource`

Evaluation:

- This is the canonical Resource Intent owner.
- Its architecture doc already says it does not fetch, store, or install.
- It is currently a single large-ish file, but the concept boundary is correct.

Recommendation:

```text
Keep as `pulith-resource`.
```

Internal future layout, if needed after behavior evaluation:

```text
identity.rs
locator.rs
version.rs or selector.rs
trust.rs
behavior.rs
resource.rs
```

Do not split now just because it is one file. First prove which concept terms are stable.

### `pulith-source`

Current concept:

```text
Source Offer
```

Current public surface evidence:

- `SourceDefinition`
- `RemoteSource`
- `SourceSet`
- `SourceSpec`
- `PlannedSources`
- `ResolvedSourceCandidate`
- `SelectionStrategy`
- `SourcePath`

Evaluation:

- This is the correct owner for “where can bytes/trees come from?”
- It should produce executable offers/candidates, not perform transfer.
- The current architecture already converges direct URL / mirror / git into `RemoteSource`.

Risk:

- `pulith-fetch` also exposes `DownloadSource`, `MultiSourceOptions`, and `SourceSelectionStrategy`.
- That may be a duplicate of Source Offer or may be fetch execution mechanics. It needs a focused boundary audit.

Recommendation:

```text
Keep as `pulith-source`, but evaluate overlap with fetch before any code decomposition.
```

Likely future target:

- `pulith-source` owns semantic candidate planning.
- `pulith-fetch` owns only execution over already-planned candidates.

### `pulith-fs`

Current concept:

```text
Materialized Evidence / filesystem safety primitive
```

Current public surface evidence:

- `atomic_write`, `atomic_read`
- `copy_dir_all`, `hardlink_or_copy`, `replace_dir`, `atomic_symlink`
- `Workspace`
- `Transaction`
- `PermissionMode`

Evaluation:

- This is an earned primitive boundary.
- It owns cross-platform mutation mechanics, not resources.
- It is used by archive/fetch/store/state/install paths.

Recommendation:

```text
Keep as `pulith-fs`.
```

Boundary clarification:

- `pulith-fs` owns mechanics, not domain evidence semantics.
- `WorkspaceReport` is a filesystem report, not install/archive/store provenance by itself.

### `pulith-verify`

Current concept:

```text
Materialized Evidence / integrity primitive
```

Current public surface evidence:

- `Hasher`
- `DigestHasher`
- `VerifiedReader`
- `VerificationReceipt`
- `verify_stream(...)`

Evaluation:

- This crate is small but owns an earned integrity boundary.
- It should be the canonical owner for streaming digest/size verification.

Risk:

- `pulith-fetch/codec/verify.rs` exposes `HashAlgorithm`, `ChecksumConfig`, `StreamVerifier`, and `MultiVerifier`.
- That may duplicate `pulith-verify` rather than adapt it.

Recommendation:

```text
Keep as `pulith-verify`; audit and reduce fetch-local verification vocabulary.
```

Target relation:

```text
pulith-fetch should consume pulith-verify primitives and emit fetch receipts.
pulith-fetch should not grow a parallel verification domain.
```

### `pulith-archive`

Current concept:

```text
Materialized Evidence / contained tree extraction and archive safety
```

Current public surface evidence:

- `ArchiveFormat`, `TarCompress`
- `ExtractOptions`
- `EntrySource`
- `Entry`, `ArchiveReport`
- `extract_from_reader(...)`
- `extract_to_workspace(...)`
- path/symlink sanitization and extraction limits

Evaluation:

- This is an earned security boundary.
- It owns path containment, symlink escape prevention, limits, and extraction reports.
- It should not be folded into fetch or store.

Recommendation:

```text
Keep as `pulith-archive`.
```

Boundary clarification:

- It may use `pulith-fs::Workspace`, but archive evidence and reports belong to archive.
- Store absorbs archive reports into provenance; archive should not know store policy.

### `pulith-fetch`

Current concept:

```text
Materialized Evidence / transfer execution
```

Current public surface evidence:

- `Fetcher`, `FetchReceipt`, `FetchSource`
- `FetchOptions`, `RetryPolicy`, progress types
- `HttpClient`, `ReqwestClient`
- `ConditionalFetcher`, `ResumableFetcher`, `MultiSourceFetcher`, `SegmentedFetcher`, `BatchFetcher`
- `DownloadSource`, `MultiSourceOptions`, `SourceSelectionStrategy`
- codec/checksum/signature/decompression helpers
- cache/rate/perf/progress helpers

Evaluation:

- This crate is conceptually needed, but it is the highest-risk boundary.
- It currently spans multiple sub-concepts:
  - transfer execution;
  - execution strategy;
  - cache/conditional/resume mechanics;
  - progress/rate/perf telemetry;
  - checksum/signature/decompression helper vocabulary;
  - source/multi-source vocabulary that may overlap `pulith-source`.

Recommendation:

```text
Keep as `pulith-fetch`, but make it the first focused boundary evaluation.
```

Do not split or rename yet. First answer:

1. What is semantic Source Offer vs fetch execution strategy?
2. What verification vocabulary should move to or wrap `pulith-verify`?
3. Which advanced fetchers are stable public behaviors vs maturing internal mechanics?
4. What exact evidence crosses from fetch into store provenance?

Possible future internal layout after evaluation:

```text
client.rs       # HttpClient / ReqwestClient
receipt.rs      # FetchReceipt / FetchSource
options.rs      # FetchOptions / RetryPolicy / progress callback contracts
execute.rs      # Fetcher base path
sources.rs      # only execution adapters over pulith-source candidates, if needed
cache.rs        # cache mechanics
resume.rs       # resumable/conditional mechanics
rate.rs         # throttling/backoff mechanics
codec.rs        # decompression only, if checksum/signature move toward verify
```

But this is not approved code work yet.

### `pulith-store`

Current concept:

```text
Artifact Memory
```

Current public surface evidence:

- `StoreRoots`, `StoreReady`
- `StoreKey`
- `StoredArtifact`, `ExtractedArtifact`
- `StoreProvenance`, `StoreMetadataRecord`
- artifact/extract registration traits
- metadata orphan inspection and prune planning

Evaluation:

- This is the correct owner for local artifact memory and provenance.
- It depends on fetch/archive so it can absorb receipts/reports into provenance.
- It should not own lifecycle truth or install policy.

Recommendation:

```text
Keep as `pulith-store`.
```

Risk:

- Prune planning touches cleanup semantics. It should remain artifact-memory cleanup, while lifecycle protection/reasoning belongs to `pulith-state`.

Future internal layout, if needed:

```text
key.rs
roots.rs
artifact.rs
extract.rs
provenance.rs
metadata.rs
prune.rs
```

Do not split before lifecycle cleanup semantics are evaluated.

### `pulith-state`

Current concept:

```text
Lifecycle State / Inspection / Repair / Retention
```

Current public surface evidence:

- `StateReady`, `StateSnapshot`
- `ResourceRecord`, `ResourceRecordPatch`, `ResourceLifecycle`
- `ActivationRecord`
- inspection reports/findings
- repair plans/actions
- ownership and retention reports
- `StateAnalysisIndex`
- state-owned `LockFile`, `LockedResource`, `LockDiff`

Evaluation:

- This is the correct owner for lifecycle state and state-derived views.
- The lock fold made the concept cleaner: lock export is a deterministic view, not an independent package-manager lock product.
- It is large and will likely need internal modules, but only after lifecycle semantics are clarified.

Recommendation:

```text
Keep as `pulith-state`.
```

Possible future internal layout:

```text
snapshot.rs
record.rs
activation.rs
inspection.rs
repair.rs
retention.rs
analysis.rs
lock.rs   # already exists
```

Do not merge with store. Artifact memory and lifecycle truth are different concepts.

### `pulith-install`

Current concept:

```text
Mutation Workflow
```

Current public surface evidence:

- `InstallReady`
- `InstallInput`, `InstallSpec`, `InstallPlanReport`
- `PlannedInstall`, `StagedInstall`, `InstalledInstall`, `ActivatedInstall`
- `Activator`, `SymlinkActivator`, `CopyFileActivator`, shim activators
- backup/restore/uninstall receipts/options
- `LifecycleOperationReceipt`

Evaluation:

- This is the correct owner for install-root mutation and activation.
- It is currently a large file, but direct decomposition is not the next step under the DDD correction.
- It depends on many crates because mutation workflow composes the whole pipeline.

Recommendation:

```text
Keep as `pulith-install`; defer internal split until lifecycle/mutation behavior is accepted.
```

Concept pressure:

- Backup/restore/uninstall and lifecycle receipt envelope may overlap with state/store cleanup semantics.
- Install should own operation receipts, but state should own lifecycle facts and restore payload semantics.

Possible future internal layout after concept approval:

```text
error.rs
input.rs
plan.rs
activation.rs
flow.rs
ready.rs
receipt.rs
fs_ops.rs  # private only if shared by several modules
```

### examples

Current concept:

```text
Caller Policy / Manager Adapter
```

Current public surface evidence:

- `examples/pulith-backend-example`
- `examples/runtime-manager`

Evaluation:

- These are important composition gates.
- They are where product/manager policy can be shown without leaking it into core crates.

Recommendation:

```text
Keep examples as the policy boundary and use them to validate reduced manual glue.
```

## New crate layout proposal

### Recommended near-term layout

Keep the current active crates, but document them under DDD bounded contexts:

```text
crates/
  # Resource Intent
  pulith-version/
  pulith-resource/

  # Source Offer
  pulith-source/

  # Materialized Evidence
  pulith-fs/
  pulith-verify/
  pulith-archive/
  pulith-fetch/

  # Artifact Memory
  pulith-store/

  # Lifecycle State / Inspection
  pulith-state/

  # Mutation Workflow
  pulith-install/

examples/
  # Caller Policy / Manager Adapter
  pulith-backend-example/
  runtime-manager/
```

This is a new conceptual layout, not a filesystem rename.

### Target dependency direction

```text
pulith-version
  <- pulith-resource
     <- pulith-source
        <- pulith-fetch

pulith-fs
  <- pulith-archive
  <- pulith-fetch
  <- pulith-store
  <- pulith-state
  <- pulith-install

pulith-verify
  <- pulith-fetch

pulith-archive
  <- pulith-store
  <- pulith-install

pulith-fetch
  <- pulith-store
  <- pulith-install

pulith-store
  <- pulith-state
  <- pulith-install

pulith-state
  <- pulith-install

all core crates
  <- examples / manager adapters
```

This direction is mostly current reality. The suspect zone is not dependency direction; it is vocabulary overlap inside the materialization path.

### Names not recommended right now

Do not rename crates to these yet:

```text
pulith-intent
pulith-offer
pulith-evidence
pulith-memory
pulith-mutation
```

Reason:

- The names are useful concept headings, but not necessarily better public Rust crate names.
- Existing crate names are concrete, familiar, and map to behaviors.
- Renaming would create churn without deleting false abstraction.

Use concept headings in architecture docs first. Rename only if a later package/publish strategy demands it.

## Concrete next report before code

The next useful report should be:

```text
docs/report/source-fetch-verify-store-boundary-evaluation.md
```

It should evaluate the most suspicious concept group:

```text
Source Offer -> Materialized Evidence -> Artifact Memory
```

Scope:

- `pulith-source`
- `pulith-fetch`
- `pulith-verify`
- `pulith-store`
- parts of `pulith-archive` only where archive reports become store provenance

Questions:

1. Does `pulith-fetch::DownloadSource` duplicate `pulith-source::ResolvedSourceCandidate`?
2. Does `pulith-fetch::SourceSelectionStrategy` duplicate `pulith-source::SelectionStrategy`?
3. Should fetch-local checksum verification collapse toward `pulith-verify`?
4. Is signature verification a real current domain or a speculative fetch-local subsystem?
5. Are cache/resume/conditional concepts execution mechanics or Artifact Memory?
6. What is the canonical evidence handoff into `StoreProvenance`?

Expected output:

- concept-by-concept API map;
- duplicate vocabulary list;
- proposed owner for each duplicate;
- delete/fold/keep recommendations;
- exact code slice if implementation is later approved.

## Do not implement yet

This report intentionally does not authorize code changes.

Before any Rust change, the next accepted design artifact should identify one behavior slice, not one crate/file by LOC.

Follow-up first-principles report:

- `docs/report/phase-a-first-principles-crate-necessity.md`

That report answers whether this many crates are necessary, why a single crate is not automatically faster, how to measure crate-count compile impact, and how to derive each concept from first principles before deciding crate/module layout.
