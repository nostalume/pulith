# Pulith Typed Archive Prepare Migration Plan

## Status

Migration analysis and execution plan. The first ZIP Prepare slice has now been executed; see:

```text
docs/report/pulith-zip-prepare-execution-report.md
```

This is the next module after the typed-tree rewrite:

```text
Intent -> WithSource -> Chosen -> Acquired -> Verified -> Prepared -> Applied -> Remembered
```

The previous slice removed the `App` monolith and replaced stringly hash behavior with feature-gated ZST digest semantics. The next implementation boundary should be `Prepare` for archive materialization.

## Inputs read

Project files inspected:

```text
docs/architecture/archive.md
docs/report/fetch-archive-materialization-continuity-evaluation.md
crates/pulith-archive/src/lib.rs
crates/pulith-archive/src/extract.rs
crates/pulith-archive/src/options.rs
crates/pulith-archive/src/entry.rs
crates/pulith/src/local.rs
crates/pulith/src/behavior.rs
crates/pulith/Cargo.toml
```

Cargo survey commands run:

```text
cargo search --registry crates-io --limit 5 zip
cargo info --registry crates-io zip
cargo search --registry crates-io --limit 5 tar
cargo info --registry crates-io tar
cargo search --registry crates-io --limit 5 safe_unzip
cargo info --registry crates-io safe_unzip
```

Observed current active feature surface:

```toml
archive = []
zip = ["archive", "dep:zip"]
tar = ["archive", "dep:tar"]
compress = ["dep:async-compression"]
```

## Cargo/crate findings

## Compression/decompression finding

Conclusion:

```text
Do not manually implement archive compression or decompression algorithms.
Use existing crates for codec mechanisms.
Pulith should only model typed behavior, policy, resource limits, path safety, and evidence.
```

The library boundary is:

```text
ZIP container + internal compression methods -> `zip`
TAR container only -> `tar`
gzip/zlib/deflate streams -> `flate2`
xz streams -> `xz2` or async-compression with xz feature
zstd streams -> `zstd` or async-compression with zstd feature
async codec adapters -> `async-compression`
```

The current old `pulith-archive` already confirms this pattern:

```rust
zip::ZipArchive<R>
tar::Archive<Decoder>
flate2::read::GzDecoder::new(reader)
xz2::read::XzDecoder::new(reader)
zstd::stream::Decoder::new(reader)
```

So the migration rule is:

```text
Manual implementation: no.
Typed wrapping/adaptation: yes.
Security/policy/evidence around extraction: yes.
```

Pulith-owned code may still implement:

```text
path normalization and destination containment checks
strip-components policy
entry-count and byte limits
symlink policy
evidence counting and reporting
typed feature-gated behavior nodes
```

But Pulith should not implement:

```text
DEFLATE
gzip
xz/lzma
zstd
ZIP parsing
TAR parsing
compression encoders/decoders
```

### `zip`

Observed:

```text
zip = "8.6.0" current latest stable; 9.0.0-pre2 also exists
Library to support reading and writing zip files
license MIT
rust-version 1.88
features include default compression families
```

Disposition:

```text
Use `zip` as mechanism for ZIP reading/extraction.
Do not migrate custom ZIP parsing.
Pulith owns typed Prepare semantics, path safety policy, resource limits, and evidence.
```

Because the workspace already uses `zip` and Cargo lock currently resolves stable `zip`, the next implementation should keep workspace `zip` stable unless there is a concrete reason to move to pre-release.

### `tar`

Observed:

```text
tar = "0.4.46"
MIT OR Apache-2.0
rust-version 1.63
streaming reader/writer
compression not handled directly
```

Disposition:

```text
Use `tar` for TAR entry iteration.
Do not build a custom TAR reader.
Treat compression as a separate typed capability (`compress`) when needed.
```

### `safe_unzip`

Observed:

```text
safe_unzip = "0.1.6"
Secure zip extraction, prevents Zip Slip and Zip Bombs
license MIT OR Apache-2.0
rust-version unknown
features include async, cli, tar, sevenz
```

Disposition:

```text
Do not adopt immediately as core dependency.
Keep it as candidate/reference for safety behavior.
First typed Prepare should use `zip` directly with Pulith-owned path-safety evidence.
Revisit `safe_unzip` only if it cleanly maps to typed evidence without leaking its API.
```

Reason: rust-version unknown and broad behavior surface make it less predictable than direct `zip` for the first typed slice.

## Old `pulith-archive` semantics audit

Old module exposes mechanism-shaped public API:

```text
ArchiveFormat
ExtractOptions
SanitizedPath
ArchiveReport
Entry
EntryKind
EntrySource
PendingEntry
ZipSource
TarSource
extract_from_reader
extract_to_workspace
WorkspaceExtraction
```

The valuable semantics are not the module names. They are:

```text
archive bytes become a local directory tree safely
entry paths cannot escape destination
symlink targets cannot escape destination
optional strip-components policy
optional entry-count and byte limits
prepared root exists only after successful extraction
extraction evidence reports observed format, entries, bytes, and sanitized targets
```

Mechanism or old public choreography to avoid migrating as-is:

```text
EntrySource trait as public caller protocol
PendingEntry as public caller vocabulary
ArchiveReport as top-level result object detached from tree node
WorkspaceExtraction as public transaction wrapper unless exclusive staging is needed
ExtractOptions as global bag copied from old crate
HashStrategy inside archive Prepare path
```

## Boundary decision

The next module is not `pulith-archive` as a crate clone. It is:

```text
Archive Prepare implementation for the typed behavior tree.
```

Behavior mapping:

```text
Verified<I, LocalMaterial, E> -> Prepared<I, ArchiveTree<A>, EvidenceChain<E, ArchiveEvidence<A>>>
```

Where `A` is a feature-gated archive algorithm/type marker.

## Typed design target

### Feature-gated archive markers

```rust
#[cfg(feature = "zip")]
pub struct Zip;

#[cfg(feature = "tar")]
pub struct Tar;
```

Do not use:

```text
format: String
ArchiveFormat enum as primary static path
extension string matching as behavior identity
```

Runtime format detection, if needed, is a boundary adapter only:

```rust
pub enum AnyArchiveKind {
    #[cfg(feature = "zip")]
    Zip,
    #[cfg(feature = "tar")]
    Tar,
}
```

Core typed path should be:

```rust
ArchivePrepare::<Zip>::new(staging).prepare_node(verified, ArchiveNeed::<Zip>::new(...))
ArchivePrepare::<Tar>::new(staging).prepare_node(verified, ArchiveNeed::<Tar>::new(...))
```

### Prepare need

Target:

```rust
pub struct ArchiveNeed<A, P = DefaultArchivePolicy> {
    policy: P,
    _archive: PhantomData<A>,
}
```

Policy should be typed or semantic, not old option bag:

```rust
DefaultArchivePolicy
SafeArchivePolicy
StripComponents<const N: usize>
BoundedArchivePolicy { max_entries, max_total_bytes }
```

First slice can use a small non-generic value policy if const/generic policy overcomplicates:

```rust
ArchivePolicy {
    strip_components: usize,
    max_entries: Option<usize>,
    max_total_bytes: Option<u64>,
}
```

But the policy belongs to `ArchiveNeed<A>`, not global `App` or universal `Need`.

### Prepared output

Target:

```rust
pub struct ArchiveTree<A> {
    root: PathBuf,
    _archive: PhantomData<A>,
}
```

This becomes the `Prepared` material:

```rust
Prepared<I, ArchiveTree<Zip>, EvidenceChain<E, ArchiveEvidence<Zip>>>
```

### Evidence

Target:

```rust
pub struct ArchiveEvidence<A> {
    root: PathBuf,
    entries: usize,
    total_bytes: u64,
    files: usize,
    directories: usize,
    symlinks: usize,
    _archive: PhantomData<A>,
}
```

Keep entry-level details private at first unless a behavior requires them.

Do not port full `Entry` vector into public evidence by default. It is too heavy and risks recreating `ArchiveReport` as a detached bag.

### Implementation node

Target:

```rust
pub struct ArchivePrepare<A, R = TempStaging> {
    resources: R,
    _archive: PhantomData<A>,
}
```

Resource control:

```text
TempStaging / ExistingExtractRoot is exclusive prepare resource.
No global ResourceManager.
No App field for staging.
```

Trait mapping:

```rust
impl<I, E, R> PrepareNode<Verified<I, LocalMaterial, E>> for ArchivePrepare<Zip, R>
where
    R: ArchiveStaging,
{
    type Need = ArchiveNeed<Zip>;
    type Prepared = ArchiveTree<Zip>;
    type Evidence = ArchiveEvidence<Zip>;
    type Error = PulithError;
    type Output = Prepared<I, ArchiveTree<Zip>, EvidenceChain<E, ArchiveEvidence<Zip>>>;
}
```

## First executable slice

Recommended first implementation:

```text
ZIP only, sync only, local file material only.
```

Scope:

```text
feature: zip
module: crates/pulith/src/archive.rs
input: Verified<I, LocalMaterial, E>
need: ArchiveNeed<Zip>
output: Prepared<I, ArchiveTree<Zip>, EvidenceChain<E, ArchiveEvidence<Zip>>>
mechanism: zip crate
path safety: reject absolute paths and parent escapes
resource limits: max_entries and max_total_bytes
```

Out of scope for first slice:

```text
TAR
compression
async archive prepare
permissions
symlink creation
full entry vector evidence
hashing every extracted entry
transactional workspace commit
net acquire
store/persist remember
```

Reason: this keeps the slice small enough to verify typed tree shape without recreating `pulith-archive` wholesale.

## Deletion/adaptation decisions for old code

### Delete or do not port

```text
EntrySource public trait
PendingEntry public type
ArchiveReport as detached top-level public result
HashStrategy inside archive extraction
old callback progress surface
WorkspaceExtraction as public transaction layer
```

### Adapt privately if useful

```text
path normalization / strip-components logic
entry/byte limit checks
zip-slip safety checks
entry kind counting
```

### Keep as Pulith-owned semantics

```text
ArchiveNeed<A>
ArchivePolicy
ArchiveTree<A>
ArchiveEvidence<A>
ArchivePrepare<A, R>
```

## Execution plan

### Step 1 — Add archive module surface

Modify:

```text
crates/pulith/src/lib.rs
crates/pulith/src/archive.rs
```

Add exports behind:

```rust
#[cfg(feature = "archive")]
pub mod archive;
```

Re-export typed public nodes only behind relevant features:

```rust
#[cfg(feature = "zip")]
pub use archive::{ArchiveNeed, ArchivePolicy, ArchivePrepare, ArchiveTree, ArchiveEvidence, Zip};
```

### Step 2 — Implement typed ZIP Prepare

Implement:

```text
Zip ZST
ArchivePolicy
ArchiveNeed<Zip>
ArchiveTree<Zip>
ArchiveEvidence<Zip>
ArchivePrepare<Zip, ExistingExtractRoot>
```

Use `zip::ZipArchive` directly.

### Step 3 — Safety rules

Implement path safety before writing files:

```text
reject absolute paths
normalize components
strip configured components
reject parent traversal that escapes root
create directories under root only
ignore/skip unsupported symlink behavior for first slice or reject symlinks explicitly
```

First slice should reject symlinks rather than creating them. This is safer and avoids Windows semantics.

### Step 4 — Tests

Add tests for:

```text
zip prepare extracts a file into ArchiveTree<Zip>
zip-slip path is rejected before writing outside root
entry limit rejects excessive archive
byte limit rejects excessive archive
prepared archive tree can flow into LocalApply<CreateOrReplace>
```

The last test proves typed tree continuity:

```text
LocalAcquire -> IdentityVerify or HashVerify<Blake3> -> ArchivePrepare<Zip> -> LocalApply<CreateOrReplace>
```

### Step 5 — Verification matrix

Run:

```text
cargo fmt --all --check
cargo check -p pulith --no-default-features
cargo check -p pulith --features 'sync local zip'
cargo check -p pulith --features 'sync local hash blake3 zip'
cargo check -p pulith --features 'sync local hash sha2 zip'
cargo check --workspace --all-features
cargo test --workspace --all-features
```

Ad-hoc structural checks:

```text
Zip type is cfg-gated by feature zip.
ArchivePrepare<Zip> exists only with zip.
No public ArchiveReport detached result is introduced.
No EntrySource/PendingEntry public protocol is introduced.
No algorithm/format string drives static behavior.
```

## Next after ZIP Prepare

After ZIP Prepare passes:

1. Add typed `Tar` marker and `ArchivePrepare<Tar>` using `tar` crate.
2. Decide whether compression is separate source material transformation or archive Prepare resource.
3. Only then move to `reqwest`/`ureq` typed Acquire.

Net Acquire should remain later because it introduces:

```text
async runtime ownership
shared HTTP client resource
bandwidth/concurrency controls
redirect/timeout policy
credential boundary concerns
```

## Summary

```text
Next module: archive Prepare, not old pulith-archive crate cloning.
First implementation: Zip-only typed Prepare.
Use mature `zip` crate for mechanism.
Own path safety, resource limits, typed prepared tree, and evidence.
Do not port EntrySource/PendingEntry/ArchiveReport as public caller vocabulary.
Do not introduce App/Context/Workspace monoliths.
Verify with feature matrix and ad-hoc structural checks.
```
