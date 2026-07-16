# Top-Down Main Task Workflow Reduction

## Status

Design/evaluation report only. No Rust code changes are authorized by this report alone.

This report corrects the previous API-spine report: the next reduction should **not** focus on any one concrete behavior such as archive installation. The next reduction should first define the main task workflow from the user's expression, then subtract incidental mechanics, and only later map the reduced workflow onto concrete downloadable/local/archive/file behaviors.

Compatibility is not a constraint. The current crates have no external consumers that require preserving redundant APIs.

## User expression to preserve

The caller wants to express:

```text
把这个 resource 从这个 source 安装到这个 target，
按这个 activation/mode/requirement 执行，
并保留 evidence 供 inspect/rollback/repair。
```

English shape:

```text
Apply this resource from this source to this target,
using this activation/mode/requirement,
and keep evidence for inspect/rollback/repair.
```

This expression is the top-down API authority for the next design slice.

## Consequence

Do not start from concrete transport or format paths:

```text
install archive
install file
fetch url
extract zip
git source
store artifact
```

Those are replaceable material-shaping strategies inside the workflow. The main API should first model the behavior independent of download/extraction/file-format conventions.

## Reduced main task

The main task is:

```text
ResourceApplication
```

Not because this is the final Rust type name, but because it describes the behavior without prematurely choosing fetch/store/install vocabulary.

A resource application has five essential inputs:

```text
1. resource     — what managed thing is being applied
2. source       — where acceptable material comes from
3. target       — where/how the result should become useful
4. operation    — mutation mode and requirements
5. evidence     — what must be retained for inspection/recovery
```

Candidate expression:

```rust
let receipt = workspace.apply(ResourceApplication {
    resource,
    source,
    target,
    operation,
    evidence,
})?;
```

This is intentionally behavior-first. It does not care whether the source is a URL, local path, archive, git checkout, pre-stored artifact, or already-extracted tree.

## Behavior atoms

The workflow must remain decomposable into atom operations, but callers should not have to hand-compose every atom for the common path.

### Atom 1 — Resolve material

Question:

```text
Given resource + source, what material candidate should be used?
```

Inputs:

- resource identity/version/trust constraints;
- source offer or explicit material handle;
- source selection requirement if multiple candidates exist.

Output:

```text
MaterialHandle
```

This is not necessarily a fetched file. It may be:

- local file;
- downloaded file;
- directory tree;
- existing stored artifact;
- generated/staged material;
- source checkout.

### Atom 2 — Prove material

Question:

```text
What evidence says this material is acceptable to use?
```

Inputs:

- material handle;
- integrity/trust requirements;
- shape/safety requirements.

Output:

```text
MaterialEvidence
```

This may include digest, byte count, transfer receipt, extraction report, path containment evidence, source candidate chosen, and timestamps.

### Atom 3 — Shape material

Question:

```text
What usable shape must the material have before application?
```

Examples of shapes:

- single file;
- executable file;
- directory tree;
- extracted tree;
- generated wrapper/shim;
- pre-staged store artifact.

Output:

```text
PreparedMaterial
```

Important: archive extraction is one possible shaping strategy, not the top-level workflow.

### Atom 4 — Remember material/evidence

Question:

```text
What should be retained so future operations can inspect, reuse, rollback, repair, or prune?
```

Output:

```text
RememberedMaterial
```

This atom may use store/state internally, but the top-level behavior should not force callers to manually derive store keys, registration objects, and install inputs unless they need an advanced path.

### Atom 5 — Apply to target

Question:

```text
How does the prepared material become useful at the target?
```

Inputs:

- prepared or remembered material;
- target;
- activation behavior;
- mutation mode;
- requirements.

Output:

```text
ApplicationReceipt
```

This atom covers staging, commit, activation, replacement, rollback snapshot, and lifecycle state updates.

### Atom 6 — Inspect/recover view

Question:

```text
What evidence and state were retained so later inspect/rollback/repair can work?
```

Output:

```text
ApplicationEvidence
```

This is not another action in the common happy path. It is the evidence envelope created by the previous atoms.

## Proposed atom chain

The decomposable workflow is:

```text
ResourceApplication
  -> resolve material
  -> prove material
  -> shape material
  -> remember material/evidence
  -> apply to target
  -> emit application receipt/evidence
```

Each arrow can be an overridable atom. The common path should run them from a single request.

## Current API after subtracting behavior from mechanics

Current caller-visible chain includes these mechanical steps:

```text
Fetcher construction
MultiSourceFetcher construction
runtime bridging
manual destination path
manual extract root path
create directory
open fetched file
extract reader
manual store key
register store artifact/extract
manual InstallInput conversion
manual InstallSpec construction
manual planning request construction
manual PlannedInstall stage/commit/activate/finish
manual state/store initialization
```

After subtraction, the main task only needs:

```text
resource
source
target
operation
requirements
evidence policy
```

Therefore the reduction target is not a better archive helper. It is a smaller main task API where fetch/archive/store/install are atom implementations behind a behavior request.

## Candidate top-level request objects

### `ResourceApplication`

```rust
pub struct ResourceApplication {
    pub resource: ResourceSubject,
    pub source: MaterialSource,
    pub target: ApplicationTarget,
    pub operation: ApplicationOperation,
    pub requirements: ApplicationRequirements,
    pub evidence: EvidencePolicy,
}
```

### `ResourceSubject`

Represents what is being managed.

```rust
pub struct ResourceSubject {
    pub id: ResourceId,
    pub version: VersionSelectorOrResolved,
    pub behavior: ResourceBehaviorContract,
}
```

Open decision:

```text
Do we use current RequestedResource/ResolvedResource directly, or collapse them into one main-task subject type?
```

Given no compatibility constraint, collapse is allowed if it deletes caller branching.

### `MaterialSource`

Represents where material may come from, not how to fetch/extract it.

```rust
pub enum MaterialSource {
    LocalPath(PathBuf),
    RemoteUrl(Url),
    PlannedSources(PlannedSources),
    ExistingArtifact(StoreKey),
    ExistingTree(PathBuf),
}
```

This is deliberately format-neutral. Format/transport-specific details belong to material strategy internals or optional policy fields.

### `ApplicationTarget`

Represents where the resource becomes useful.

```rust
pub struct ApplicationTarget {
    pub root: PathBuf,
    pub activation: Option<ActivationTarget>,
}
```

Target is not just an install root. It is the externally useful result boundary.

### `ApplicationOperation`

Represents mutation behavior.

```rust
pub struct ApplicationOperation {
    pub mode: ApplicationMode,
    pub rollback: RollbackRequirement,
}
```

Candidate modes:

```text
Create
Replace
Upgrade
Refresh
Remove
Repair
```

Current `InstallMode` is a lower implementation detail unless it maps cleanly onto this task-level operation.

### `ApplicationRequirements`

Represents capability/policy gates.

```rust
pub struct ApplicationRequirements {
    pub connectivity: ConnectivityRequirement,
    pub writable_scope: WritableScopeRequirement,
    pub activation: ActivationRequirement,
    pub integrity: IntegrityRequirement,
}
```

These fields preserve policy explicitly while removing mechanical planning boilerplate.

### `EvidencePolicy`

Represents what must be retained.

```rust
pub struct EvidencePolicy {
    pub retain_material: RetainMaterial,
    pub retain_provenance: bool,
    pub retain_lifecycle: bool,
    pub inspectable: bool,
    pub rollbackable: bool,
    pub repairable: bool,
}
```

This is the top-down counterpart to store/state/install receipts. The caller says what future operations must be possible; internals choose which evidence records are needed.

## Candidate receipt objects

### `ResourceApplicationReceipt`

```rust
pub struct ResourceApplicationReceipt {
    pub resource: ResourceId,
    pub target: ApplicationTargetReceipt,
    pub operation: OperationReceipt,
    pub evidence: ApplicationEvidence,
}
```

### `ApplicationEvidence`

```rust
pub struct ApplicationEvidence {
    pub material: MaterialEvidence,
    pub memory: Option<RememberedMaterial>,
    pub lifecycle: LifecycleEvidence,
}
```

The point is not to hide receipts. The point is to stop making the caller stitch them together manually.

## Main path and advanced path

The API should expose two layers:

### Main path

```rust
workspace.apply(ResourceApplication { ... })?
```

This is the ergonomic path.

### Atom path

```rust
let material = workspace.resolve_material(...)?;
let evidence = workspace.prove_material(...)?;
let prepared = workspace.shape_material(...)?;
let remembered = workspace.remember_material(...)?;
let receipt = workspace.apply_material(...)?;
```

This is the decomposable path.

The important design rule:

```text
The main path is built from the atom path, not separate from it.
```

This keeps the workflow拆卸组装 while still allowing the common call to be short.

## Reduction/deletion implications

Because compatibility does not matter, after this design is accepted we should be willing to delete or demote current APIs that only expose incidental steps.

Potential deletion/demotion candidates:

1. Public convenience APIs that combine source planning inside fetch when the main task request owns source.
2. Public `Into*` conversion traits that exist only to smooth noisy intermediate objects.
3. Repeated builder chains where a single request object is clearer.
4. Manual state/store initialization in product-level examples.
5. Public direct type-state workflow as the only common install path.

This does not mean deleting the internal operations. It means moving them behind atoms and exposing them only where they are real extension points.

## What to evaluate next

Do not pick archive/file/url as the next focus.

Instead, evaluate the current public API against the atom chain:

```text
resolve material
prove material
shape material
remember material/evidence
apply to target
emit inspect/recovery evidence
```

For each current type/function, classify it as one of:

```text
Essential request field
Atom operation
Atom implementation detail
Evidence/receipt field
Redundant glue
```

This classification should drive deletion and redesign.

## Proposed next report

Write:

```text
docs/report/main-task-api-surface-classification.md
```

It should inventory current public APIs from:

- `pulith-resource`
- `pulith-source`
- `pulith-fetch`
- `pulith-archive`
- `pulith-store`
- `pulith-install`
- `pulith-state`

and map each to:

```text
ResourceApplication request
atom operation
receipt/evidence
internal detail
delete/demote
```

Only after that classification should code change.

## Current recommendation

The immediate design direction is:

```text
1. Stop focusing on any specific concrete behavior such as install archive.
2. Define the resource application workflow from the user's expression.
3. Reduce the public main path to resource/source/target/operation/requirements/evidence.
4. Keep decomposable atom operations underneath that path.
5. Classify current APIs by whether they serve the main path, an atom path, evidence, or only glue.
6. Delete redundant glue without compatibility shims.
```

This is the correct top-down move because it starts from what the caller needs to say, not from what the existing crates happen to expose.
