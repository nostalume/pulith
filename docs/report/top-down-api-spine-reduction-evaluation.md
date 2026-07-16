# Top-Down API Spine Reduction Evaluation

## Status

Design/evaluation report only. No Rust code changes are authorized by this report alone.

This report replaces crate-semantics-first thinking with API-spine-first thinking.

The working correction is:

```text
The important question is not whether fetch/fs/lock/version/store are elegant concepts.
The important question is: what does a caller actually need to do, and why is the current API forcing them through so many incidental objects?
```

Compatibility is not a constraint for this evaluation. These crates have no external consumers that matter for the current redesign, so the optimization target is API shape and internal quality, not preserving old entry points.

## Evaluation method

Instead of starting from crate names, start from the caller's job.

A resource manager needs to express this main line:

```text
Acquire material -> prove/shape it -> remember it if useful -> apply it -> record lifecycle facts
```

More concretely:

```text
Install this resource from this source into this target, with this activation behavior, and keep enough evidence to inspect/rollback/repair later.
```

Everything else is implementation support unless it changes one of those decisions.

## Current API spine observed in live code

Representative remote/local archive install path from `examples/runtime-manager/src/main.rs`:

```rust
let resource = resolved_remote_resource(...)?;

let fetcher = Fetcher::new(ReqwestClient::new()?, workspace_root.join("fetch-workspace"));
let multi = MultiSourceFetcher::new(Arc::new(fetcher));
let fetched = runtime.block_on(async {
    multi.fetch_resolved_resource_with_receipt(
        &resource,
        SelectionStrategy::OrderedFallback,
        &destination,
        &FetchOptions::default(),
    ).await
})?;

let extract_root = workspace_root.join("extracted").join(...);
fs::create_dir_all(&extract_root)?;
let fetched_file = fs::File::open(&fetched.destination)?;
let report = extract_from_reader(fetched_file, &extract_root, &ExtractOptions::default())?;

let store = init_store(workspace_root)?;
let key = StoreKey::NamedVersion { ... };
let extracted = store.register_extract(
    &key,
    ExtractedTreeRegistration::from_fetch_archive(&fetched, extract_root.as_path(), &report),
)?;

let state = init_state(workspace_root)?;
let receipt = execute_install_with_plan(
    InstallReady::new(state),
    InstallSpec::new(resource, InstallInput::ExtractedArtifact(extracted), install_root)
        .replace_existing()
        .activation(ActivationTarget { path: activation_target }),
    InstallPlanningRequest { ... },
)?;
```

This is structurally correct but not ergonomic. It exposes too many intermediate owners to the caller.

## Extracted main line

The caller's actual intent is closer to:

```text
install archive resource:
  resource identity/version/source
  source execution preference
  extraction policy
  store key / memory policy
  install root
  activation target
  replacement/rollback expectation
```

The current API forces that into this mechanical chain:

```text
Resource values
  -> Source planning helper
  -> Fetcher construction
  -> MultiSourceFetcher wrapper
  -> Runtime bridge
  -> FetchReceipt
  -> destination path convention
  -> extract root convention
  -> File::open
  -> extract_from_reader
  -> ArchiveReport
  -> StoreReady
  -> StoreKey reconstruction
  -> ExtractedTreeRegistration
  -> ExtractedArtifact
  -> InstallInput
  -> StateReady
  -> InstallSpec
  -> InstallPlanningRequest
  -> PlannedInstall
  -> stage
  -> commit
  -> activate
  -> finish
```

This is too many public-facing nouns for one common operation.

## Complexity inventory

### Essential caller decisions

These should remain explicit:

1. **What resource?**
   - id, version, locator/source hint, trust requirement.
2. **What material form?**
   - file, archive tree, pre-stored artifact, already-extracted tree.
3. **Where to install?**
   - install root.
4. **How to activate?**
   - symlink/copy/custom activator and activation target.
5. **What mutation mode?**
   - create/replace/upgrade and rollback expectation.
6. **What policy gates?**
   - offline requirement, writable scope, activation availability, rollback availability.

### Incidental mechanics currently leaked

These are implementation details unless the caller explicitly customizes them:

1. `Fetcher::new(...)` and `MultiSourceFetcher::new(...)` wiring.
2. `tokio::runtime::Runtime::new()?.block_on(...)` in sync examples.
3. Manual download destination naming.
4. Manual extraction root naming.
5. `fs::create_dir_all(&extract_root)`.
6. `File::open(&fetched.destination)`.
7. Manual `extract_from_reader(...)` in product-level flow.
8. Manual `StoreKey::NamedVersion` reconstruction from the same resource id/version.
9. Manual `ExtractedTreeRegistration::from_fetch_archive(...)`.
10. Manual `InstallInput::ExtractedArtifact(extracted)`.
11. Repeated `InstallReady::new(init_state(...))` plumbing.
12. Repeated `InstallSpec::new(...).replace_existing().activation(...)` construction.
13. Repeated `InstallPlanningRequest` construction for common variants.

These details should not all be in the caller's main line.

## Design problem statement

Pulith has successfully extracted domain concepts, but its API is still a bottom-up composition API. The caller composes every primitive manually.

The result is:

```text
conceptually clean internals, mechanically noisy API
```

The design target should shift from:

```text
Are the crates semantically separated?
```

to:

```text
What is the shortest honest API spine for each common resource-manager operation?
```

## What not to optimize

Do not spend the next slice optimizing these as first-class design topics:

- the naming purity of `fetch`, `fs`, `store`, `version`, `lock`, or similar lower concepts;
- preserving compatibility aliases;
- splitting large files by module aesthetics;
- adding a new orchestration crate;
- making a universal manager framework;
- improving every primitive before the main line is simplified.

These are secondary. The main issue is caller experience and API shape.

## API-spine-first model

The top-down API should expose a small number of task-shaped surfaces over the same internal safety boundaries.

Candidate external spine:

```text
PulithWorkspace
  .install(ResourceInstallRequest)
  .materialize(MaterializationRequest)
  .inspect(ResourceRef)
  .repair(RepairRequest)
  .uninstall(UninstallRequest)
```

This does **not** mean adding a heavy orchestrator crate. It can be an owner-local façade in the crate that owns the mutation workflow, or an example manager API if still experimental.

The important point is that the public main line should be request/receipt oriented:

```text
Request -> Plan -> Apply -> Receipt
```

not primitive-by-primitive object choreography.

## Proposed canonical operation spines

### 1. Materialize archive to store

Current:

```text
plan/fetch -> open -> extract -> register -> extracted artifact
```

Target shape:

```rust
let extracted = materializer.materialize_archive(ArchiveMaterializationRequest {
    resource,
    source,
    store_key,
    destination_policy,
    extraction_policy,
})?;
```

or, if kept inside install/store without a new materializer:

```rust
let input = InstallInput::from_archive_source(ArchiveInstallInputRequest { ... })?;
```

Expected hidden mechanics:

- destination path derivation;
- extraction root derivation;
- create directories;
- open fetched file;
- extract archive;
- register store provenance;
- convert to install input.

Expected explicit caller choices:

- source / resource;
- extraction options if non-default;
- store memory policy;
- whether fetch evidence is required.

### 2. Install materialized input

Current:

```text
InstallSpec::new(...)
  .replace_existing()
  .activation(...)
PlannedInstall::new(...).stage()?.commit()?.activate()?.finish()
```

Target shape:

```rust
let receipt = workspace.install(InstallRequest {
    resource,
    input,
    root,
    mode,
    activation,
    requirements,
})?;
```

Optional explicit plan path:

```rust
let plan = workspace.plan_install(&request)?;
let receipt = plan.apply()?;
```

This preserves preview/apply without exposing every type-state step in the common path.

The existing type-state path can remain internal or advanced, but it should not be the only ergonomic API.

### 3. Install archive from source

This is likely the most important top-level user story.

Target shape:

```rust
let receipt = workspace.install_archive(InstallArchiveRequest {
    resource,
    source,
    install_root,
    activation,
    mode: ReplaceExisting,
    planning: InstallRequirements { ... },
    extraction: ExtractOptions::default(),
})?;
```

This request should not hide policy. It simply makes the obvious mechanical chain one operation.

It should still expose a plan/receipt:

```text
InstallArchivePlan
InstallArchiveReceipt
```

Receipt should carry:

- fetch receipt, if fetched;
- archive report;
- store key / extracted artifact;
- install receipt;
- lifecycle receipt.

## Where should the façade live?

### Option A — `pulith-install` owns task-shaped install façades

Pros:

- install is already Mutation Workflow owner;
- common caller goal is install, not fetch/store purity;
- avoids a new thin `pulith-materialize` crate;
- can still delegate internals to fetch/archive/store/state.

Cons:

- risk that install becomes source/fetch policy owner;
- must keep request fields explicit enough to avoid hidden policy.

Evaluation:

```text
Best near-term candidate.
```

### Option B — examples/future manager owns the façade first

Pros:

- fastest experimentation;
- avoids committing API too soon;
- proves ergonomics against real call sites.

Cons:

- repeated internal examples may diverge;
- not enough pressure to simplify core APIs.

Evaluation:

```text
Good spike path before moving into pulith-install.
```

### Option C — new `pulith-workspace` or `pulith-materialize` crate

Pros:

- clean top-level name.

Cons:

- likely becomes a thin orchestrator;
- recreates glue-layer problem;
- adds another public package while the project is trying to reduce surface area.

Evaluation:

```text
Reject for now.
```

## Reduction criteria

A new API is accepted only if it passes these tests:

1. **Deletes caller choreography**
   - removes at least one repeated multi-step block from real examples/tests.
2. **Does not hide policy**
   - source ranking, retry, trust, activation fallback, cleanup remain explicit request fields or caller decisions.
3. **Preserves safety boundaries**
   - fetch/archive/store/install still own their invariants internally.
4. **Produces better receipts**
   - output explains what happened without caller manually stitching evidence.
5. **Has one canonical path**
   - no compatibility aliases or duplicate entry points.
6. **Can delete old public glue**
   - because compatibility does not matter, old redundant helpers should be removed in the same slice.

## Proposed next design slice

Write a concrete API design plan for the highest-value path:

```text
install archive source -> active installed resource
```

Plan file:

```text
docs/report/install-archive-api-spine-reduction-plan.md
```

It should include:

1. Current call spine with line references.
2. Minimal desired request type.
3. Minimal desired receipt type.
4. Which existing public helpers become redundant and can be deleted.
5. Which crate owns the façade initially.
6. Exact tests/examples to rewrite.
7. Verification gates.

## Preliminary target API sketch

This is a design sketch, not implementation approval:

```rust
pub struct InstallArchiveRequest {
    pub resource: ResolvedResource,
    pub source: ArchiveSource,
    pub install_root: PathBuf,
    pub activation: Option<ActivationTarget>,
    pub mode: InstallMode,
    pub requirements: InstallPlanningRequest,
    pub extraction: ExtractOptions,
    pub store_key: Option<StoreKey>,
}

pub enum ArchiveSource {
    LocalPath(PathBuf),
    Planned(PlannedSources),
    Fetched(FetchReceipt),
}

pub struct InstallArchiveReceipt {
    pub fetch: Option<FetchReceipt>,
    pub archive: ArchiveReport,
    pub extracted: ExtractedArtifact,
    pub install: InstallReceipt,
    pub lifecycle: LifecycleOperationReceipt,
}
```

Potential call site:

```rust
let receipt = workspace.install_archive(InstallArchiveRequest {
    resource,
    source: ArchiveSource::LocalPath(archive_path),
    install_root,
    activation: Some(ActivationTarget { path: activation_target }),
    mode: InstallMode::ReplaceExisting,
    requirements,
    extraction: ExtractOptions::default(),
    store_key: None,
})?;
```

This collapses the main line while preserving explicit policy.

## Immediate recommendation

Accept the user's correction fully:

```text
Stop treating fetch/fs/version/store semantics as the center of the design.
Treat them as implementation owners behind a shorter task-shaped API.
```

Next work should be:

```text
1. Design the install-archive API spine.
2. Use no compatibility constraints.
3. Rewrite real examples/tests to that spine.
4. Delete redundant lower-level public convenience APIs if they no longer carry unique value.
```

Do not start with module splits. Do not create a new crate. Do not preserve duplicate entry points.
