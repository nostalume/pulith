# Top-Down Architecture Current Design Issues

## Status

Design/report only. No Rust code changes are authorized by this report alone.

This report re-reads the current top-down architecture after the recent materialization cleanup and confirms the next design problems to solve before more implementation.

Read/evidence sources:

- `docs/architecture.md`
- `docs/architecture/source.md`
- `docs/architecture/fetch.md`
- `docs/architecture/archive.md`
- `docs/architecture/store.md`
- `docs/architecture/install.md`
- `docs/report/top-down-architecture-reduction.md`
- `docs/report/phase-a-ddd-crate-layout-evaluation.md`
- `docs/report/phase-a-first-principles-crate-necessity.md`
- `docs/report/fetch-archive-materialization-continuity-evaluation.md`
- live `cargo metadata --no-deps --format-version 1`
- live source grep for fetch/archive/store/install composition call paths

## Current architecture authority

`docs/architecture.md` is now the best current top-down authority. It correctly frames Pulith as:

```text
Resource Intent -> Source Offer -> Materialized Evidence -> Artifact Memory -> Mutation Workflow -> Lifecycle State
```

Current live crate dependency evidence:

```text
pulith-archive: pulith-fs
pulith-fetch: pulith-fs, pulith-resource, pulith-source
pulith-resource: pulith-version
pulith-source: pulith-resource
pulith-store: pulith-archive, pulith-fetch, pulith-fs, pulith-resource
pulith-state: pulith-fs, pulith-resource, pulith-store
pulith-install: pulith-archive, pulith-fetch, pulith-fs, pulith-resource, pulith-state, pulith-store, pulith-source
runtime-manager-example: pulith-archive, pulith-backend-example, pulith-fetch, pulith-install, pulith-resource, pulith-source, pulith-state, pulith-store
```

Important correction from older reports:

```text
pulith-verify is no longer an active crate.
```

Therefore any older report section that says `pulith-verify` is a current Materialized Evidence owner is stale historical context, not current architecture authority.

## Confirmed healthy boundaries

### 1. Resource Intent is clean

Current owner:

```text
pulith-version + pulith-resource
```

Design state:

- expresses identity, version, locator/trust/behavior semantics;
- has no fetch/store/install execution;
- should not be decomposed by file size before intent vocabulary stabilizes.

Current issue level: low.

### 2. Source Offer is conceptually correct

Current owner:

```text
pulith-source
```

Design state:

- owns `SourceSpec`, `PlannedSources`, `ResolvedSourceCandidate`, `SelectionStrategy`;
- does not fetch;
- correctly answers “where could this resource come from?”

Current issue level: medium only because `pulith-fetch` still exposes convenience methods that construct plans from resource values.

### 3. Fetch and Archive should remain separate safety owners

Current owners:

```text
pulith-fetch   = byte materialization / transfer evidence
pulith-archive = tree materialization / extraction safety evidence
```

Confirmed DDD reason:

```text
fetch invariant:
  source bytes are transferred to a local file with transfer evidence.

archive invariant:
  untrusted archive entries expand into a local tree without path/symlink escape.
```

Do not merge these crates simply because download and extraction often happen in sequence.

### 4. Store boundary improved

Recent implementation added:

```rust
ExtractedTreeRegistration
```

This fixed the previous tuple protocol:

```rust
(&FetchReceipt, &Path, &ArchiveReport)
```

Now store has a named Artifact Memory boundary for:

```text
extracted tree root + archive evidence + optional fetch evidence
```

This is the right kind of DDD cleanup: it names an existing domain fact instead of adding a new orchestration layer.

## Confirmed design problems

## Problem 1 — Top-down reports are stale relative to current architecture

Older reports still contain stale current-state claims, especially:

```text
pulith-verify as an active Materialized Evidence owner
source/fetch/verify as the next current boundary phrase
verify.md as an active architecture doc
```

Current state:

```text
pulith-verify absent from metadata
checksum verification is fetch-owner-local
archive evidence is archive-owner-local
store absorbs fetch/archive evidence through ExtractedTreeRegistration
```

Design consequence:

- `docs/architecture.md` should remain canonical.
- `docs/report/top-down-architecture-reduction.md`, `phase-a-ddd-crate-layout-evaluation.md`, and `phase-a-first-principles-crate-necessity.md` should be treated as historical unless refreshed.
- Do not make decisions from their old crate lists.

Recommended doc cleanup slice:

```text
Refresh top-down reports or add a short supersession note pointing to docs/architecture.md and this report.
Do not edit Rust code for this cleanup.
```

## Problem 2 — Source/fetch boundary is better but still not fully crisp

Live code evidence:

```rust
MultiSourceFetcher::fetch_planned_sources_with_receipt(&PlannedSources, ...)
MultiSourceFetcher::fetch_source_spec_with_receipt(SourceSpec, SelectionStrategy, ...)
MultiSourceFetcher::fetch_requested_resource_with_receipt(&RequestedResource, SelectionStrategy, ...)
MultiSourceFetcher::fetch_resolved_resource_with_receipt(&ResolvedResource, SelectionStrategy, ...)
```

Interpretation:

- The canonical DDD interface should be:

```text
Source Offer -> PlannedSources / ResolvedSourceCandidate -> Fetch execution
```

- `fetch_planned_sources_with_receipt` matches that boundary.
- The resource/spec entry helpers are ergonomic, but they let `pulith-fetch` create source plans internally.

This is not necessarily wrong. It may be acceptable glue reduction. But it is a design pressure because `pulith-fetch` then participates in Source Offer construction instead of only executing offers.

Decision needed:

```text
Are resource/spec entry helpers canonical public API, or transitional convenience?
```

Recommended direction:

```text
Keep `fetch_planned_sources_with_receipt` as the canonical boundary.
Treat resource/spec entry helpers as convenience only unless they delete substantial caller glue.
Do not add more fetch-owned source vocabulary.
```

Potential future cleanup:

- examples and higher workflow code may plan sources explicitly before fetch;
- fetch docs should emphasize planned-source execution as canonical;
- if helpers remain, document them as thin convenience entry points, not separate source ownership.

## Problem 3 — Materialization continuity still has repeated workflow glue

The previous tuple protocol was fixed, but product callers still repeat this sequence:

```rust
fetch_resolved_resource_with_receipt(...)
fs::create_dir_all(&extract_root)
File::open(&receipt.destination)
extract_from_reader(file, &extract_root, &ExtractOptions::default())
store.register_extract(
    &key,
    ExtractedTreeRegistration::from_fetch_archive(&receipt, extract_root.as_path(), &report),
)
InstallInput::ExtractedArtifact(extracted)
```

Live examples:

- `examples/runtime-manager/src/main.rs::install_local_archive`
- `examples/runtime-manager/src/main.rs::install_remote_archive`
- `crates/pulith-install/tests/workspace_pipeline.rs` archive pipeline helpers

The repeated glue is now clearer and safer, but still exists.

Design question:

```text
Should we stop at named evidence, or add a narrow install-owned constructor for archive-backed install input?
```

Do not answer this by adding `pulith-materialize`. That would recreate a thin orchestration crate.

Better candidate:

```rust
InstallInput::from_registered_archive(...)
```

or a small install-owned request type, but only if it deletes multiple call-site blocks and does not absorb source/fetch policy into install.

Constraint:

```text
install may build semantic InstallInput from already materialized/registerable archive facts;
install must not become source resolver, retry-policy owner, or global fetch/archive orchestrator.
```

Recommended next design slice:

```text
Evaluate whether an install-owned archive input constructor deletes enough real glue after ExtractedTreeRegistration.
```

## Problem 4 — Archive output is still `root + ArchiveReport`, not a single tree evidence object

`pulith-archive` currently returns:

```rust
ArchiveReport
```

while callers independently track:

```text
extract_root path
```

Store now names the registration evidence with `ExtractedTreeRegistration`, but archive itself does not yet return a first-class object like:

```rust
ExtractedTree { root: PathBuf, report: ArchiveReport }
```

This may or may not be necessary.

Decision rule:

```text
Only add archive-owned ExtractedTree if it deletes repeated `root + report` pairing across callers without duplicating Store's Artifact Memory role.
```

Do not add it merely because it is aesthetically cleaner.

## Problem 5 — Store/state/install cleanup and recovery semantics are still cross-cutting

Current architecture docs correctly separate:

```text
store = artifact memory/provenance
state = lifecycle facts, inspection, repair, retention
install = mutation workflow, backup/restore, uninstall, rollback
```

But the next hard design boundary is still unresolved:

```text
Which owner names and applies cleanup/recovery decisions?
```

Known pressure points:

- store has orphan inspection and prune planning;
- state has lifecycle inspection, repair, ownership, and retention views;
- install has uninstall, rollback, backup/restore receipts;
- lifecycle receipt language spans install and state outcomes.

Design risk:

```text
If cleanup/recovery APIs grow independently, Pulith may get three different preview/apply vocabularies.
```

Recommended later report:

```text
docs/report/lifecycle-cleanup-recovery-boundary-evaluation.md
```

It should define one preview/apply language across store/state/install without making a hidden orchestrator.

## Problem 6 — Large owner files are real but not the next top-down issue

Large files remain:

```text
pulith-store/src/lib.rs
pulith-state/src/lib.rs
pulith-install/src/lib.rs
pulith-resource/src/lib.rs
```

But the current architecture reading says:

```text
Do not split by LOC.
Split only after the concept owner and public behavior boundary are accepted.
```

The recent `ExtractedTreeRegistration` slice is a good example: it changed the boundary first, then call sites, without splitting files.

Recommended rule:

```text
Internal module decomposition is allowed only after a behavior report names stable owners and accepted API surfaces.
```

## Problem 7 — Docs spine still has stale links and authority drift

Examples found in docs search:

- `docs/AGENT.md` references deleted `docs/roadmap.md`.
- old reports still refer to `pulith-verify` as active.
- publish docs intentionally mention historical `pulith-verify` but need clear current/historical labels.
- `docs/architecture/fetch.md` says fetch feeds “future crates such as `pulith-store`, `pulith-resource`, and `pulith-install`” even though those crates are active.

Design impact:

```text
Low code risk, but high future-agent risk.
```

Recommended doc cleanup:

```text
Keep docs/architecture.md as current authority.
Add supersession notes to stale reports.
Update docs/AGENT.md and fetch.md stale wording.
Do not rewrite all historical reports unless they are being used as active planning contracts.
```

## Prioritized next actions

### Priority 1 — Refresh architecture authority / stale report guard

Goal:

```text
Prevent old reports from reintroducing pulith-verify or source/fetch/verify framing.
```

Work:

- add a current-state note to `docs/report/top-down-architecture-reduction.md`;
- optionally update `phase-a-*` reports with a supersession note;
- fix stale `docs/roadmap.md` links in `docs/AGENT.md` if desired;
- update `docs/architecture/fetch.md` “future crates” wording.

Verification:

```bash
git diff --check -- docs
rg "pulith-verify.*current|verify.md|docs/roadmap.md|future crates such as `pulith-store`" docs
```

### Priority 2 — Evaluate install-owned materialized archive input constructor

Goal:

```text
Decide whether remaining fetch/open/extract/register/input glue deserves a narrow install-owned constructor.
```

Do not code first. Write:

```text
docs/report/install-archive-input-continuity-evaluation.md
```

Questions:

- How many active call sites still do the full archive materialization chain?
- Would an install-owned helper delete real code or just hide composition?
- Does the helper require fetch policy or only already-materialized archive path + optional fetch receipt?
- Should the helper return `InstallInput` or `ExtractedArtifact`?

Likely acceptable shape:

```rust
ArchiveInstallInputRequest {
    key,
    archive_path,
    extract_root,
    fetch_receipt: Option<&FetchReceipt>,
    extract_options,
}

InstallInput::from_registered_archive(&store, request)
```

Only implement if the report proves net code/glue reduction.

### Priority 3 — Source/fetch canonical boundary cleanup

Goal:

```text
Make PlannedSources -> fetch execution the canonical API while preventing fetch from becoming a second Source Offer owner.
```

Questions:

- Keep, demote, or remove `fetch_requested_resource_with_receipt` / `fetch_resolved_resource_with_receipt`?
- Should examples plan sources explicitly before fetch?
- What source vocabulary still lives in fetch after previous cleanup?

### Priority 4 — Lifecycle cleanup/recovery language

Goal:

```text
Unify preview/apply vocabulary across store prune, state repair/retention, install rollback/uninstall/restore.
```

This should come after materialization continuity because store/install call paths are currently being cleaned.

## Current recommendation

Do **not** jump into module decomposition next.

The next best design move is:

```text
1. Refresh stale architecture authority so current reports cannot reintroduce deleted verify concepts.
2. Write a focused install-archive-input continuity evaluation.
3. Only if that report proves real glue reduction, implement a narrow install-owned constructor.
```

This keeps the project aligned with the current standard:

```text
reduce glue;
keep iteration fast;
preserve fetch/archive safety boundaries;
do not add orchestration crates;
do not split files by LOC;
name real domain evidence before adding helpers.
```
