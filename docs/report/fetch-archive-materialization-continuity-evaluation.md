# Fetch / Archive Materialization Continuity Evaluation

## Question

Based on the DDD concept model, evaluate the next slice after deleting `pulith-verify`:

```text
resource -> source -> fetch -> archive -> store -> install
```

The concrete design question is:

```text
Should `pulith-fetch` and `pulith-archive` remain separate crates, merge, or gain a continuous workflow surface so callers do not hand-stitch download, extraction, provenance, store registration, and install input creation?
```

This report focuses on the current `resource`, `source`, `fetch`, and `archive` relationship, then extends the evaluation to `store` and `install` because the actual continuity break appears at their call boundary.

## Current evidence from code

Active workspace metadata now contains these relevant packages:

```text
pulith-resource
pulith-source
pulith-fetch
pulith-archive
pulith-store
pulith-install
```

`pulith-verify` is absent from current Cargo metadata and its crate directory is deleted in the active diff. The current integrity check has been folded into `pulith-fetch` as owner-local SHA-256 transfer evidence.

Relevant current call paths:

```text
examples/runtime-manager/src/main.rs
  install_local_archive:
    source/resource -> fetch_resolved_resource_with_receipt
    fs::create_dir_all(extract_root)
    File::open(receipt.destination)
    extract_from_reader(file, extract_root, ExtractOptions::default())
    store.register_extract((&receipt, extract_root, &report))
    InstallInput::ExtractedArtifact(extracted)
    PlannedInstall::new(...).stage().commit().activate()

  install_remote_archive:
    same fetch -> extract -> register_extract -> install chain

  install_airgapped_archive:
    File::open(archive_path)
    extract_from_reader(file, extract_root, ExtractOptions::default())
    InstallInput::from_extracted_tree(extract_root)
```

Integration tests repeat the same shape:

```text
crates/pulith-install/tests/workspace_pipeline.rs
  archive_extract_store_install_pipeline:
    extract_from_reader -> register_extract -> InstallInput::ExtractedArtifact

  local_archive_fetch_extract_store_install_pipeline:
    fetch_local_resource_to -> File::open(fetched.destination)
    -> extract_from_reader -> register_extract((&fetched, extract_root, &report))
    -> InstallInput::ExtractedArtifact

  archive_replace_activate_rollback_restores_previous_activation_snapshot:
    fetch_local_resource_to -> File::open(fetched.destination)
    -> extract_from_reader -> register_extract((&fetched, extract_root, &report))
    -> InstallInput::ExtractedArtifact
```

Current store already absorbs provenance through trait inputs:

```rust
StoreReady::register_artifact(key, &FetchReceipt)
StoreReady::register_extract(key, (&Path, &ArchiveReport))
StoreReady::register_extract(key, (&FetchReceipt, &Path, &ArchiveReport))
```

Current install already consumes semantic handles:

```rust
InstallInput::StoredArtifact(StoredArtifact)
InstallInput::ExtractedArtifact(ExtractedArtifact)
InstallInput::ExtractedTree { root }
```

## DDD concept boundaries

### `resource`: Resource Intent

Question answered:

```text
What should exist?
```

Owned facts:

```text
resource id
version selector / resolved version
locator intent or hint
metadata / digest requirement
```

Must not own:

```text
candidate ordering
network/local transfer
archive extraction
store registration
install mutation
```

### `source`: Source Offer

Question answered:

```text
Where could this resource come from, and in what candidate order?
```

Owned facts:

```text
SourceSpec
ResolvedSourceCandidate
PlannedSources
SelectionStrategy
```

Must not own:

```text
HTTP execution
atomic file placement
archive tree safety
install lifecycle mutation
```

### `fetch`: Byte Materialization

Question answered:

```text
How do source bytes become a local file with transfer evidence?
```

Owned facts:

```text
FetchOptions
FetchReceipt
FetchSource
bytes_downloaded
total_bytes
sha256_hex when requested/computed
retry/progress/range/cache mechanics
```

Must not own:

```text
source semantic vocabulary duplicated from pulith-source
archive tree expansion safety
store retention/prune policy
install lifecycle state
```

### `archive`: Tree Materialization / Extraction Safety

Question answered:

```text
How do archive bytes become a local directory tree safely?
```

Owned facts:

```text
ArchiveFormat
ExtractOptions
SanitizedPath
ArchiveReport
entry count
total extracted bytes
entry reports
path traversal / symlink escape / permission policy
```

Must not own:

```text
network/local transfer
source selection
store key derivation
install lifecycle mutation
```

## Main finding

`pulith-fetch` and `pulith-archive` are adjacent but not identical bounded contexts.

They protect different invariants:

```text
fetch invariant:
  source bytes are transferred to a local file with explicit transfer evidence.

archive invariant:
  untrusted archive entries are expanded into a local tree without path/symlink escape and with extraction evidence.
```

Therefore, direct crate merge is not the first move.

However, the current public workflow is not continuous enough for product callers. The caller repeatedly hand-stitches this chain:

```rust
let fetched_file = fs::File::open(&fetched.destination)?;
let report = extract_from_reader(fetched_file, &extract_root, &ExtractOptions::default())?;
let extracted = store.register_extract(&key, (&fetched, extract_root.as_path(), &report))?;
let install_input = InstallInput::ExtractedArtifact(extracted);
```

This means the boundary problem is not primarily `fetch` vs `archive` crate count. The problem is missing a product-shaped materialization surface for the common operation:

```text
fetch archive bytes -> extract tree -> register tree memory -> produce install-ready semantic input
```

## What should not be done

### Do not create `pulith-materialize` now

A new crate would likely become a thin orchestration crate:

```text
pulith-materialize = fetch + archive + store glue
```

That repeats the same anti-pattern as removed satellite crates unless there are multiple independent callers that need a durable public API separate from install/store.

### Do not move archive into fetch now

Merging `archive` into `fetch` because download and extract are often adjacent would hide the archive-specific safety contract:

```text
path traversal
symlink escape
entry limits
permissions
format detection
extraction report
```

These are independently testable and independently reusable even when archive extraction follows fetch in normal product flows.

### Do not move fetch/archive execution into store

`pulith-store` owns memory/registration, not execution. It should absorb receipts and reports, but should not perform network transfer or unsafe-tree expansion.

Bad direction:

```rust
store.fetch_and_extract(...)
```

Better direction:

```rust
store.register_extract(key, materialized_tree)
```

where `materialized_tree` is already produced by the workflow owner.

## Where the continuous workflow should live

The current strongest owner candidate is `pulith-install`, but not as a global downloader.

Reason:

```text
install is the mutation workflow that needs a materialized semantic input.
```

It already owns:

```text
InstallInput
InstallSpec
PlannedInstall -> StagedInstall -> InstalledInstall -> ActivatedInstall
state updates
rollback/activation semantics
```

But it currently requires callers to prepare `InstallInput` manually. That is a good boundary for low-level composition, yet the examples/tests show the common archive materialization path is repeated enough to deserve an owner-local convenience type or function.

The proposed shape should be small and typed, not a god orchestrator.

## Proposed next API shape

### Option A — install-owned archive materialization helper

Add an install-owned request that converts already-fetched or local archive bytes into `InstallInput` through archive+store:

```rust
pub struct ArchiveInstallInputRequest<'a> {
    pub key: &'a StoreKey,
    pub archive_path: &'a Path,
    pub extract_root: &'a Path,
    pub fetch_receipt: Option<&'a FetchReceipt>,
    pub extract_options: ExtractOptions,
}

impl InstallInput {
    pub fn from_archive_registration(
        store: &StoreReady,
        request: ArchiveInstallInputRequest<'_>,
    ) -> Result<Self>;
}
```

Semantics:

```text
open archive_path
archive::extract_from_reader(...)
store.register_extract(key, (fetch_receipt?, extract_root, report))
return InstallInput::ExtractedArtifact(extracted)
```

Pros:

```text
- deletes repeated caller glue immediately
- keeps fetch and archive safety contracts separate
- keeps store as registration/memory owner
- keeps install as semantic input workflow owner
```

Cons:

```text
- introduces pulith-install dependency on pulith-archive and pulith-fetch if not already present
- install becomes aware of archive materialization, though only as input construction
```

Current manifests already show install participates in this workspace flow, so this is acceptable if the function remains narrow.

### Option B — store-owned registered extracted tree input

Keep extraction outside store but add a stronger typed registration input:

```rust
pub struct ExtractedTreeRegistration<'a> {
    pub root: &'a Path,
    pub fetch: Option<&'a FetchReceipt>,
    pub archive: &'a ArchiveReport,
}

store.register_extract(key, ExtractedTreeRegistration { ... })
```

This is mostly a naming cleanup around the existing tuple trait implementation:

```rust
(&FetchReceipt, &Path, &ArchiveReport)
```

Pros:

```text
- improves readability and DDD vocabulary
- avoids tuple-order fragility
```

Cons:

```text
- does not remove the repeated fetch -> open file -> extract -> register -> install glue by itself
```

### Option C — archive-owned extraction receipt

Have `pulith-archive` return an explicit materialized tree object:

```rust
pub struct ExtractedTree {
    pub root: PathBuf,
    pub report: ArchiveReport,
}

pub fn extract_file_to_tree(
    archive_path: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    options: &ExtractOptions,
) -> Result<ExtractedTree>;
```

Then store can accept:

```rust
store.register_extract(key, (&fetch_receipt, &extracted_tree))
```

Pros:

```text
- gives archive a first-class output object instead of passing root/report separately
- preserves archive ownership of tree materialization
- helps install/store consume tree evidence without tuple glue
```

Cons:

```text
- still needs caller to compose fetch and store
- may duplicate `WorkspaceExtraction` unless carefully scoped
```

## Recommended slice

Do this in two steps, not one broad rewrite.

### Step 1 — replace tuple glue with named materialized tree evidence

Add a typed output/registration shape for the existing repeated pair:

```text
root path + ArchiveReport + optional FetchReceipt
```

Best minimal target:

```rust
// pulith-store, registration boundary
pub struct ExtractedTreeRegistration<'a> {
    pub root: &'a Path,
    pub archive: &'a ArchiveReport,
    pub fetch: Option<&'a FetchReceipt>,
}
```

Then migrate examples/tests from tuple inputs:

```rust
store.register_extract(&key, (&fetched, extract_root.as_path(), &report))
```

to named input:

```rust
store.register_extract(
    &key,
    ExtractedTreeRegistration::from_fetch_archive(&fetched, extract_root.as_path(), &report),
)
```

This is a DDD improvement because it turns an implicit tuple protocol into a named boundary object.

Implementation slice 1 landed: archive tree registration now uses
`ExtractedTreeRegistration` instead of raw archive tuple protocols at active
call sites. This keeps `pulith-store` as the provenance/memory boundary while
making archive evidence named at the API boundary.

### Step 2 — only if repetition remains, add install-owned input constructor

After Step 1, re-check the call sites. If `fetch/open/extract/register/input` is still repeated in examples/tests, add an install-owned helper that produces `InstallInput` from a local archive path and optional fetch receipt.

Potential final call shape:

```rust
let install_input = InstallInput::from_registered_archive(
    &store,
    ArchiveInstallInputRequest {
        key: &key,
        archive_path: &fetched.destination,
        extract_root: &extract_root,
        fetch_receipt: Some(&fetched),
        extract_options: ExtractOptions::default(),
    },
)?;
```

This should be added only if it deletes multiple repeated glue blocks. It must not become a broad materialization orchestrator.

## Boundary decision

Current DDD decision:

```text
Keep pulith-resource.
Keep pulith-source.
Keep pulith-fetch.
Keep pulith-archive.
Do not recreate pulith-verify.
Do not add pulith-materialize.
Improve the continuity boundary through named evidence and, if still valuable, an install-owned input constructor.
```

## Acceptance criteria for the implementation slice

A successful next code slice should prove:

```text
1. No caller passes raw tuple protocols for fetched archive extraction registration.
2. The archive evidence object is named and readable at the store boundary.
3. Fetch remains byte materialization.
4. Archive remains tree materialization / extraction safety.
5. Store remains memory/provenance registration.
6. Install consumes semantic install input, and only gains a helper if it deletes real repeated glue.
7. `pulith-verify` remains absent from active Cargo metadata and active source imports.
```

Suggested verification after code:

```bash
cargo fmt --all --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps
```

Focused absence/layout checks:

```bash
cargo metadata --no-deps --format-version 1 | grep -v pulith-verify
rg "pulith_verify|pulith-verify" crates examples Cargo.toml Cargo.lock
rg "register_extract\([^\n]*\(&[^,]+,\s*[^,]+,\s*&report\)" crates examples
```

## Summary

The next slice should not merge `fetch` and `archive` yet. DDD says they are adjacent but protect different invariants.

The actual design smell is that callers pass an unnamed tuple across the `archive -> store -> install` continuity boundary. First turn that implicit tuple protocol into a named materialized-tree registration concept. Then decide whether an install-owned archive input constructor is still needed to remove repeated product workflow glue.
