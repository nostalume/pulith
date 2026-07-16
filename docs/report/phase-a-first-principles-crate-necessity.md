# Phase A First-Principles Concept Implementation and Crate Necessity

## Status

Design/evaluation report only. Do not rename crates, fold crates, split modules, or edit Rust code from this report alone.

This report responds to two corrections:

1. DDD concept names are not enough. We need to derive concepts from first principles, define how each concept is implemented, and specify interfaces to adjacent concepts.
2. More crates are not automatically better. We need to evaluate whether the current crate count is necessary and whether a single crate would improve compile speed or efficiency.

## Current evidence used

Current active Pulith crates and internal/external dependency profile from `cargo metadata --no-deps --format-version 1`:

```text
pulith-version   internal=- external_count=6
pulith-resource  internal=pulith-version external_count=4
pulith-source    internal=pulith-resource external_count=2
pulith-fs        internal=- external_count=6
pulith-verify    internal=- external_count=6
pulith-archive   internal=pulith-fs external_count=10
pulith-fetch     internal=pulith-fs,pulith-resource,pulith-source,pulith-verify external_count=17
pulith-store     internal=pulith-archive,pulith-fetch,pulith-fs,pulith-resource external_count=5
pulith-state     internal=pulith-fs,pulith-resource,pulith-store external_count=5
pulith-install   internal=pulith-archive,pulith-fetch,pulith-fs,pulith-resource,pulith-state,pulith-store,pulith-source external_count=10
```

A fresh incremental check with timings enabled produced:

```text
cargo check --workspace --all-features --timings
Timing report saved to target/cargo-timings/...
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.50s
```

Caveat: this is incremental/warm evidence, not a cold build benchmark. It proves the current workspace is not obviously slow in the current warmed state. It does not prove current crate count is optimal.

`cargo check --timings=json` was not accepted by this installed Cargo; only HTML timing output is available through `--timings`.

## First-principles starting point

Pulith's goal is not “have crates for layers.”

Pulith's goal is:

```text
Build reusable mechanisms for resource managers.
```

A resource manager repeatedly solves this problem:

```text
Given an intended managed thing, find acceptable material, prove what it is, place it somewhere useful, expose it if needed, and record enough facts to inspect, repair, rollback, prune, or explain it later.
```

From that goal, the domain pattern is not arbitrary. It is forced by the lifecycle of any managed thing:

```text
Intent -> Offer -> Evidence -> Memory -> Mutation -> State -> Inspection/Repair
```

Each concept exists because it answers a different first-principles question.

## What is a resource?

A resource is not a file, URL, package, archive, executable, directory, or installed path.

A resource is:

```text
A named managed capability or artifact whose desired existence can be declared, whose acceptable material forms can be obtained and verified, whose local lifecycle can be mutated, and whose facts can be inspected later.
```

That definition intentionally separates the thing from its representations.

Examples:

```text
Resource: example/runtime
Possible source offer: https://example.com/runtime.zip
Possible materialized evidence: fetched bytes with sha256 digest and extraction report
Possible artifact memory: stored archive key + extracted tree key + provenance
Possible mutation: staged into install root and activated as bin/runtime
Possible lifecycle state: Installed + active target history + repair findings
```

A resource therefore has several roles:

| Role | Meaning | Example |
| --- | --- | --- |
| Identity | Stable name of the managed thing | `example/runtime` |
| Intent | Desired version/trust/behavior | `stable`, sha256 required, path activation |
| Offer | Where acceptable material may come from | URL, mirror, git, local path |
| Evidence | What was actually obtained/proven | digest, bytes count, extraction report |
| Memory | Durable local artifact/provenance | store key + metadata JSON |
| Mutation | Install/activation operation | stage, commit, activate, rollback |
| State | Durable lifecycle fact | installed path, activation history |

## How to extract the general pattern

Use this extraction method for each concept:

1. Start with the resource-manager goal.
2. Ask what question must be answered before the next step can happen.
3. Name the answer as a domain object.
4. Identify what facts the object must contain.
5. Identify what it must not decide.
6. Define the input/output interface to adjacent concepts.
7. Only then decide whether the object deserves a crate, module, type, trait, or method.

Applied to Pulith:

### 1. Intent

Question:

```text
What managed thing does the caller want, under which semantic constraints?
```

Domain object:

```text
Resource Intent
```

Concretization:

- `ResourceId`
- `VersionSelector`
- `ResourceLocator`
- `VerificationRequirement`
- `TrustPolicy`
- `ResourceBehaviorContract`
- `RequestedResource`
- `ResolvedResource`

Interface to next concept:

```text
RequestedResource / ResolvedResource -> SourceSpec::from_requested_resource / from_resolved_resource
```

Must not decide:

- retry policy
- cache policy
- install path
- lifecycle cleanup

### 2. Offer

Question:

```text
Given the intent, where can acceptable material come from?
```

Domain object:

```text
Source Offer
```

Concretization:

- `SourceDefinition`
- `RemoteSource`
- `SourceSet`
- `SourceSpec`
- `PlannedSources`
- `ResolvedSourceCandidate`
- `SelectionStrategy`

Interface to next concept:

```text
PlannedSources / ResolvedSourceCandidate -> fetch execution request
```

Must not decide:

- network transport details
- cache freshness execution
- byte verification result
- store registration

Open problem:

`pulith-fetch` currently has `DownloadSource`, `MultiSourceOptions`, and `SourceSelectionStrategy`. These may duplicate Source Offer. The next boundary report should decide whether fetch keeps only execution strategy while source owns semantic candidate planning.

### 3. Evidence

Question:

```text
What was actually obtained, and why should we trust it enough to use it?
```

Domain object:

```text
Materialized Evidence
```

Concretization:

- `FetchReceipt`
- `VerificationReceipt`
- `ArchiveReport`
- digest/size facts
- path/symlink containment facts
- workspace placement facts

Interface to next concept:

```text
FetchReceipt / ArchiveReport -> StoreProvenance / ArtifactRegistration / ExtractRegistration
```

Must not decide:

- lifecycle truth
- install target
- retention policy
- user/product fallback policy

Open problem:

`pulith-fetch/codec/verify.rs` may duplicate `pulith-verify`. Evidence should have one integrity vocabulary, with fetch adapting it into receipts.

### 4. Memory

Question:

```text
How do we remember materialized artifacts so future workflows can reuse and explain them?
```

Domain object:

```text
Artifact Memory
```

Concretization:

- `StoreKey`
- `StoredArtifact`
- `ExtractedArtifact`
- `StoreProvenance`
- `StoreMetadataRecord`
- metadata prune plans

Interface to next concept:

```text
StoredArtifact / ExtractedArtifact -> InstallInput / IntoInstallInput
```

Interface to state:

```text
StoreKey -> ResourceRecord.artifact_key / StoreKeyReference / retention protection
```

Must not decide:

- whether a resource is installed
- whether to uninstall
- external cleanup timing

### 5. Mutation

Question:

```text
How do we safely change install roots and activation targets using materialized inputs?
```

Domain object:

```text
Mutation Workflow
```

Concretization:

- `InstallInput`
- `InstallSpec`
- `InstallPlanReport`
- `PlannedInstall -> StagedInstall -> InstalledInstall -> ActivatedInstall`
- `Activator`
- `ActivationReceipt`
- `RollbackReceipt`
- `BackupReceipt`
- `RestoreReceipt`
- `UninstallReceipt`
- `LifecycleOperationReceipt`

Interface to state:

```text
Installed/Activated/Uninstalled operation -> StateReady updates resource and activation records
```

Must not decide:

- dependency solving
- source ranking
- fetch retry policy
- store prune policy

### 6. State

Question:

```text
What lifecycle facts are true now, and what repairs/retention decisions are available?
```

Domain object:

```text
Lifecycle State
```

Concretization:

- `StateSnapshot`
- `ResourceRecord`
- `ActivationRecord`
- `ResourceInspectionReport`
- `ResourceStateRepairPlan`
- `StateAnalysisIndex`
- `LockFile` as state export view
- retention and activation ownership reports

Interface to caller policy:

```text
Inspection/repair/retention reports -> caller chooses apply/ignore/prune/rollback
```

Must not decide:

- product policy timing
- dependency graph solving
- network/archive execution

## Interface graph between concepts

Concept interfaces should look like this:

```text
Resource Intent
  RequestedResource / ResolvedResource
    -> Source Offer

Source Offer
  PlannedSources / ResolvedSourceCandidate
    -> Materialized Evidence

Materialized Evidence
  FetchReceipt / VerificationReceipt / ArchiveReport
    -> Artifact Memory

Artifact Memory
  StoredArtifact / ExtractedArtifact / StoreKey / StoreProvenance
    -> Mutation Workflow
    -> Lifecycle State references

Mutation Workflow
  InstallReceipt / ActivationReceipt / LifecycleOperationReceipt
    -> Lifecycle State updates

Lifecycle State
  InspectionReport / RepairPlan / RetentionPlan / LockFile
    -> Caller Policy
```

Design rule:

```text
Adjacent interfaces should pass semantic objects, not paths reconstructed from conventions.
```

## Is it necessary to keep this many crates?

Short answer:

```text
No, DDD does not require ten crates.
But a single crate is also not automatically better.
```

Crate count should be decided by product and build boundaries, not concept count.

A Rust crate is justified when at least one of these is true:

1. It has an independent product/publish story.
2. It protects a safety/security boundary.
3. It isolates heavy optional dependencies or features.
4. It is a stable primitive used by multiple higher crates.
5. It prevents dependency cycles by giving lower concepts a leaf boundary.
6. It gives useful incremental compilation isolation.

A crate is suspicious when:

1. It is a wrapper over one mature wheel/library.
2. It only re-exports aliases.
3. It has no independent concept and one active consumer.
4. It exists only because a file was large.
5. It adds a public abstraction before two real implementations exist.

The already-deleted crates matched the suspicious list. The remaining crates are less obvious.

## Does one single crate improve compile speed?

Not necessarily.

Rust/Cargo compile tradeoff:

| Layout | Potential compile benefit | Potential compile cost |
| --- | --- | --- |
| One big crate | fewer rustc crate invocations; fewer workspace package metadata steps | any edit rechecks a larger crate; less parallelism; harder feature/dependency isolation; downstream users may compile heavy code they do not need unless feature gating is strict |
| Many small crates | better incremental isolation; parallel compilation; cleaner dependency/features; downstream users can depend on smaller slices | more rustc invocations; more package metadata; more public boundary maintenance |
| Balanced crates | keeps heavy/safety boundaries isolated while avoiding tiny satellites | requires discipline about what earns a crate |

For this repo, the main compile-cost suspects are not crate count alone. They are heavy external dependency clusters:

- `pulith-fetch`: `reqwest`, async/futures, compression, base64, serde/json, checksums, temp/test deps.
- `pulith-archive`: archive/compression stack: `tar`, `zip`, `zstd`, `xz2`, `flate2`, hashing.
- `pulith-fs`: filesystem/platform primitives.

Merging `pulith-fetch`, `pulith-archive`, `pulith-verify`, and `pulith-fs` into one crate may reduce some crate invocation overhead, but it also risks forcing heavy fetch/archive deps onto callers who only need verification, filesystem, or archive primitives.

Therefore:

```text
Single crate is not the default optimization.
A facade crate may improve ergonomics later, but core implementation should stay split where dependency/safety boundaries are real.
```

## Crate necessity scorecard

| Crate | Necessary as crate? | Reason |
| --- | --- | --- |
| `pulith-version` | Maybe | Pure semantic leaf. Could fold into `pulith-resource` if package count matters, but current compile cost is probably low. |
| `pulith-resource` | Yes | Canonical Resource Intent owner. |
| `pulith-source` | Maybe | Strong concept, but may live as `pulith-resource::source` if we choose fewer semantic crates. Needs fetch-overlap audit first. |
| `pulith-fs` | Yes | Shared side-effect primitive and cross-platform filesystem contract. |
| `pulith-verify` | Yes or Maybe | Integrity boundary is real. Could merge with materialization only if fetch/archive are feature-gated carefully. |
| `pulith-archive` | Yes | Security/safety boundary with heavy optional-ish dependencies and extraction semantics. |
| `pulith-fetch` | Yes | Transfer execution boundary, but its internal vocabulary likely needs reduction. |
| `pulith-store` | Yes | Artifact Memory owner. |
| `pulith-state` | Yes | Lifecycle State owner. |
| `pulith-install` | Yes | Mutation Workflow owner. |

## Candidate crate layouts

### Layout A — Current conceptual layout

```text
pulith-version
pulith-resource
pulith-source
pulith-fs
pulith-verify
pulith-archive
pulith-fetch
pulith-store
pulith-state
pulith-install
```

Pros:

- Clear lower-level boundaries.
- Good incremental isolation.
- Heavy materialization pieces remain separately usable.
- No churn.

Cons:

- More packages to publish/version/document.
- Some tiny pure semantic crates may not earn separate public packages long-term.
- Current user-facing ergonomics may require multiple dependencies.

Recommendation:

```text
Acceptable as internal workspace topology, but not necessarily the final publish topology.
```

### Layout B — Balanced fewer-crate topology

```text
pulith-resource      # version + resource + source offer modules
pulith-fs            # atomic filesystem/workspace primitives
pulith-verify        # integrity primitive, unless folded into materialize later
pulith-archive       # extraction/security boundary
pulith-fetch         # transfer execution over source candidates
pulith-store         # artifact memory
pulith-state         # lifecycle state
pulith-install       # mutation workflow
```

This folds:

```text
pulith-version -> pulith-resource::version
pulith-source  -> pulith-resource::source or pulith-resource::offer
```

Pros:

- Reduces package count from 10 to 8.
- Keeps all side-effect/safety boundaries separate.
- Keeps Resource Intent and Source Offer near each other because they are pure semantic planning concepts.

Cons:

- Makes `pulith-resource` heavier.
- Removes standalone version/source crates that may be useful to downstream callers.
- Requires import migration and documentation work.

Recommendation:

```text
Best first candidate if we decide fewer crates matter.
```

But do not implement until `source/fetch` overlap is audited; otherwise we may fold `pulith-source` before knowing what source vocabulary should survive.

### Layout C — Product/facade layout

```text
pulith              # facade crate for normal users, optional features
pulith-resource     # semantic core
pulith-fs
pulith-verify
pulith-archive
pulith-fetch
pulith-store
pulith-state
pulith-install
```

Pros:

- Better user ergonomics without destroying internal boundaries.
- Lets downstream users choose `pulith` with feature flags or depend on fine-grained crates.

Cons:

- Adds a crate instead of reducing count.
- Needs mature API before worthwhile.
- Facade crates can become re-export dumping grounds if introduced too early.

Recommendation:

```text
Do not add now. Consider near publish/release ergonomics only.
```

### Layout D — Single crate

```text
pulith
  version
  resource
  source
  fs
  verify
  archive
  fetch
  store
  state
  install
```

Pros:

- One package to publish/use.
- Fewer Cargo package boundaries.
- Easier initial onboarding.

Cons:

- Heavy dependency/features must be very carefully gated.
- Any edit rechecks a larger crate.
- Harder to keep primitive crates policy-free.
- Harder for downstream users to depend on only `resource` or `verify` without fetch/archive deps.
- Security boundaries become module discipline rather than package dependency discipline.

Recommendation:

```text
Reject as the core implementation layout right now.
```

A single facade crate can be revisited later. A single implementation crate is premature and likely reduces architectural clarity more than it improves compile speed.

## Recommended decision

Use a two-level answer:

```text
Concept topology: keep the DDD concepts.
Physical crate topology: reduce only pure semantic satellites after boundary audit; keep side-effect/safety/heavy-dependency crates separate.
```

Near-term physical layout target:

```text
Keep now:
  pulith-version
  pulith-resource
  pulith-source
  pulith-fs
  pulith-verify
  pulith-archive
  pulith-fetch
  pulith-store
  pulith-state
  pulith-install

Evaluate next:
  fold pulith-version into pulith-resource?
  fold pulith-source into pulith-resource after source/fetch ownership is clarified?

Do not fold yet:
  pulith-fs
  pulith-verify
  pulith-archive
  pulith-fetch
  pulith-store
  pulith-state
  pulith-install
```

## What “how of concept implementation” should look like in code

Each concept should have three things:

1. **Domain object** — typed data that names the concept.
2. **Operation** — method/trait that transforms it to the next concept.
3. **Evidence/receipt** — output facts that explain the transformation.

Pattern:

```text
ConceptInput -> ConceptOperation -> ConceptOutput + Receipt/Evidence
```

Pulith examples:

```text
RequestedResource
  -> SourceSpec::from_requested_resource(...).plan(...)
  -> PlannedSources

PlannedSources / ResolvedSourceCandidate
  -> Fetcher::fetch(...)
  -> FetchReceipt

FetchReceipt + ArchiveReport
  -> StoreReady::register_artifact/register_extract_dir(...)
  -> StoredArtifact / ExtractedArtifact + StoreProvenance

StoredArtifact / ExtractedArtifact
  -> InstallInput / InstallSpec / PlannedInstall::stage().commit()
  -> InstalledInstall + lifecycle receipt

InstalledInstall
  -> activate(...)
  -> ActivatedInstall + ActivationReceipt

StateReady
  -> inspect/plan_repair/export_lock_file
  -> reports/plans/views
```

Good interface properties:

- contains typed semantic IDs, not only paths;
- carries enough evidence for the next concept;
- does not smuggle policy;
- returns a receipt/report when side effects happen;
- has a small canonical conversion path, not many aliases.

## Next required design report before code

Write this next:

```text
docs/report/source-fetch-verify-store-first-principles-boundary.md
```

It should derive the source/materialization/store interfaces from first principles:

1. Define `Source Offer` in terms of questions and objects.
2. Define `Transfer Execution` as a sub-concept of `Materialized Evidence`.
3. Define `Integrity Evidence` and choose the canonical owner.
4. Define `Artifact Memory` and the exact evidence it should persist.
5. Compare live types:
   - `pulith-source::ResolvedSourceCandidate`
   - `pulith-fetch::DownloadSource`
   - `pulith-fetch::SourceSelectionStrategy`
   - `pulith-fetch::FetchSource`
   - `pulith-fetch::FetchReceipt`
   - `pulith-verify::VerificationReceipt`
   - `pulith-store::StoreProvenance`
6. Decide whether `pulith-source` stays a crate or folds into `pulith-resource`.
7. Decide whether fetch-local verification vocabulary is deleted, adapted, or moved.
8. Only then propose code changes.

This report should include a cold/warm build measurement plan before claiming crate-count compile improvements.

## Compile measurement plan before crate-count changes

Before folding more crates for performance reasons, measure:

```bash
cargo clean
cargo check --workspace --all-features --timings
cargo test --workspace --all-features --no-run --timings
```

Then after a proposed fold, compare:

- total wall time;
- number of units in timing report;
- whether heavy external crates are pulled into callers that did not need them;
- incremental check after touching a resource-only file;
- incremental check after touching fetch/archive code.

Do not assume single-crate is faster without this evidence.

## Bottom line

- The current 10-crate layout is not obviously wrong after deleting the false satellites.
- It is also not sacred.
- DDD concepts should drive interfaces; crate count should follow compile/product evidence.
- The most plausible future reduction is semantic: fold `version` and/or `source` into `resource` if the source/fetch audit proves their standalone crate boundaries do not earn their cost.
- Keep side-effect/security/heavy-dependency crates separate unless measurement proves otherwise.
