# Pulith File I/O Dependency and Behavior Design

## Scope

User direction:

```text
Prioritize file-operation crates and file-operation knowledge first.
Current file interaction implementation may not be good enough or suitable.
Combine dependency research, project requirements, behavior design, quality/performance assessment, and report.
Defer net Acquire.
```

This document therefore does **not** design net Acquire. It defines the next file-interaction migration and optimization target.

## Current Pulith context

Current active file interaction is in:

```text
crates/pulith/src/local.rs
```

Current typed path:

```text
Intent<Item, LocalTarget, O>
  -> WithSource<_, LocalPath>
  -> Chosen<_, LocalPath>
  -> Acquired<_, LocalMaterial, AcquireEvidence>
  -> Verified<_, LocalMaterial, E>
  -> Prepared<_, LocalPrepared, E>
  -> Applied<_, Receipt<O>, E>
```

Current operations:

```text
Create
Replace
CreateOrReplace
Forget
```

Current implementation shape:

```text
LocalAcquire: exists + File/Directory classification
IdentityPrepare: pass LocalMaterial to LocalPrepared
LocalApply: direct filesystem mutation
copy_prepared: std::fs::copy for files, recursive copy_dir_all for directories
remove_existing: remove_dir_all/remove_file before replacement
```

## Current implementation assessment

### Strengths

```text
Small implementation.
Typed behavior states are clean.
No App/Context monolith.
Create/Replace/CreateOrReplace/Forget are statically distinct operations.
Evidence chain is preserved.
```

### Correctness gaps

```text
Replace and CreateOrReplace remove the target before a replacement is fully staged.
File copy is not atomic; partial target files are possible on copy failure.
Directory copy is not transactional.
No staging root/resource object.
No rollback or cleanup boundary for failed staged operations.
No source == target guard.
No source-under-target / target-under-source guard for directory operations.
No explicit symlink behavior policy.
No hardlink policy.
No permission/timestamp preservation policy.
No fsync/durability policy.
No same-device/cross-device placement strategy.
```

### Performance gaps

```text
Directory copy is single-threaded recursive std::fs.
No hardlink fast path for large files.
No size threshold for copy vs link.
No directory walk crate for efficient traversal/reporting.
No reporting of bytes/files/directories copied.
No staging layout that can later support net Acquire landing efficiently.
```

## Old pulith-fs reference assessment

Old crate:

```text
crates/pulith-fs
```

Useful mechanisms found:

```text
Workspace staging root
relative path sanitizer
atomic_write / atomic_read
hardlink_or_copy with cross-device fallback
replace_dir with Windows retry loop
stage_file_by_size / stage_file_with_size_hint
WorkspaceReport with file_count/directory_count/total_bytes
Drop cleanup for uncommitted staging workspace
```

Important caveat:

```text
Do not migrate `Workspace` as public caller choreography.
```

Keep the mechanisms, not the old API shape. In the new design, staging belongs to `LocalApply` resources and evidence, not to a separate user-facing workflow object.

## Crate search and dependency evaluation

Commands run:

```bash
cargo search --registry crates-io "atomic file" --limit 8
cargo search --registry crates-io "copy directory" --limit 8
cargo search --registry crates-io "filesystem walk" --limit 8
cargo info --registry crates-io tempfile
cargo info --registry crates-io atomic-write-file
cargo info --registry crates-io fs-err
cargo info --registry crates-io fs_extra
cargo info --registry crates-io walkdir
cargo info --registry crates-io ignore
cargo info --registry crates-io same-file
cargo info --registry crates-io pathdiff
cargo info --registry crates-io jwalk
cargo info --registry crates-io filetime
cargo info --registry crates-io path-absolutize
cargo info --registry crates-io atomicwrites
```

### tempfile

Observed metadata:

```text
tempfile 3.27.0
license MIT OR Apache-2.0
rust-version 1.63
A library for managing temporary files and directories.
```

Assessment:

```text
High quality and already in workspace dependencies.
Best default for temporary directories/files and cleanup ownership.
Use for tests and optionally internal staging roots when caller does not provide one.
```

Recommendation:

```text
Keep/use.
Prefer `tempfile::Builder` or `TempDir` for unique staging paths.
Do not invent random temp naming manually when TempDir fits.
```

### atomic-write-file

Observed metadata:

```text
atomic-write-file 0.3.0
license BSD-3-Clause
rust-version 1.85
Write files atomically to a file system.
```

Assessment:

```text
Good focused candidate for atomic file writes.
Could replace hand-rolled temp-file + rename code for file content writes.
But Pulith's main file Apply copies existing files/directories, not just writes in-memory bytes.
It is useful for future small metadata/state writes and maybe Remember persistence.
```

Recommendation:

```text
Do not add for LocalApply copy first slice.
Consider later for persistent Remember/state writes.
```

### atomicwrites

Observed metadata:

```text
atomicwrites 0.4.4
license MIT
Atomic file-writes.
```

Assessment:

```text
Older/less compelling than atomic-write-file for new work.
```

Recommendation:

```text
Do not add.
```

### fs-err

Observed metadata:

```text
fs-err 3.3.1
license MIT OR Apache-2.0
A drop-in replacement for std::fs with more helpful error messages.
```

Assessment:

```text
Useful ergonomics crate.
Improves error messages but does not solve atomicity, staging, symlink policy, or performance.
Pulith already wraps errors with path context in PulithError::io.
```

Recommendation:

```text
Do not add initially.
Keep explicit PulithError path/context mapping.
```

### fs_extra

Observed metadata:

```text
fs_extra 1.3.0
license MIT
Expanding std::fs and std::io. Recursively copy folders with process information and more.
```

Current workspace already has:

```toml
fs_extra = "1.3"
```

Assessment:

```text
Reasonable option for recursive directory copy with copy options and progress-ish information.
However, generic recursive copy semantics may not match Pulith's strict policy needs: symlink reject/preserve/follow, source-under-target guard, staged replacement, and evidence counting.
```

Recommendation:

```text
Do not make it the core behavior engine.
Use only if it demonstrably matches Pulith policies after a focused spike.
For first hardening, prefer explicit walk + Pulith policy loop so behavior is auditable.
```

### walkdir

Observed metadata:

```text
walkdir 2.5.0
license Unlicense/MIT
Recursively walk a directory.
```

Assessment:

```text
Mature, tiny, widely used.
Good fit for explicit Pulith directory copy policy.
Lets Pulith own per-entry behavior while avoiding hand-rolled recursive traversal.
```

Recommendation:

```text
Best candidate to add for directory traversal.
Use for serial deterministic copy/report first.
```

### ignore

Observed metadata:

```text
ignore 0.4.28
license Unlicense OR MIT
Fast library for matching ignore files like .gitignore.
```

Assessment:

```text
High quality but not needed for exact local Apply.
Useful only if Pulith later supports ignore-pattern filtered source trees.
```

Recommendation:

```text
Do not add now.
```

### same-file

Observed metadata:

```text
same-file 1.0.6
license Unlicense/MIT
Determine whether two paths point to the same file.
```

Assessment:

```text
Small, focused, high-value correctness guard.
Useful for source == target detection across symlinks/hardlinks/platform metadata.
```

Recommendation:

```text
Add or vendor equivalent small check only if std canonicalization/metadata comparison is insufficient.
Prefer adding `same-file` for explicit guard clarity.
```

### pathdiff

Observed metadata:

```text
pathdiff 0.2.3
license MIT/Apache-2.0
Library for diffing paths to obtain relative paths.
```

Assessment:

```text
Not central to Apply. Relative layout can be computed from walkdir strip_prefix against the source root.
```

Recommendation:

```text
Do not add.
```

### jwalk

Observed metadata:

```text
jwalk 0.8.1
license MIT
Filesystem walk performed in parallel with streamed and sorted results.
```

Assessment:

```text
Attractive for large-tree performance.
But parallel copy makes failure ordering, deterministic tests, resource limits, and rollback semantics more complex.
```

Recommendation:

```text
Do not use in first correctness slice.
Consider later behind a separate performance feature or resource policy if large directory copy becomes a measured bottleneck.
```

### filetime

Observed metadata:

```text
filetime 0.2.29
license MIT/Apache-2.0
rust-version 1.75.0
Platform-agnostic accessors of timestamps in File metadata.
```

Assessment:

```text
Useful only if Pulith decides to preserve mtime/atime in Apply policy.
Not needed for initial safety/atomicity hardening.
```

Recommendation:

```text
Defer until preservation policy exists.
```

### path-absolutize

Observed metadata:

```text
path-absolutize 4.0.1
license MIT
rust-version 1.80
Absolute path and dot removal helpers.
```

Assessment:

```text
Could help lexical normalization, but Apply safety should prefer actual filesystem metadata/canonicalization where possible and explicit containment checks.
```

Recommendation:

```text
Do not add now.
```

## Dependency recommendation

For the next file Apply hardening slice:

```text
Add: walkdir
Possibly add: same-file
Keep using: tempfile
Do not add: fs_extra as core, jwalk, ignore, pathdiff, path-absolutize, atomicwrites
Defer: atomic-write-file, filetime
```

Suggested feature/dependency shape:

```toml
[features]
local = ["dep:walkdir", "dep:same-file"]

[dependencies]
walkdir = { version = "2.5", optional = true }
same-file = { version = "1", optional = true }
```

Rationale:

```text
walkdir gives robust traversal without taking over behavior semantics.
same-file gives a sharp guard against self-overwrite/self-copy.
tempfile handles unique staging roots.
Pulith still owns staging, atomic placement, symlink policy, and evidence.
```

## Pulith file behavior requirements

### Required laws

```text
Create must not overwrite an existing target.
Replace must fail if the target does not exist.
CreateOrReplace may create or replace.
Forget removes target if present and is idempotent enough for absent target.
A failed Apply must not leave the final target partially written.
A failed Replace/CreateOrReplace should preserve the old target whenever feasible.
Source must not be the same object as target.
Directory source must not be copied into itself or into its descendant.
Directory target must not be copied from its descendant.
Symlink handling must be explicit.
Evidence must report observable placement facts.
```

### Required policies

```rust
pub enum SymlinkPolicy {
    Reject,
    Preserve,
    Follow,
}

pub enum PlacementStrategy {
    Copy,
    HardlinkOrCopy,
}

pub enum DurabilityPolicy {
    None,
    FileSync,
    FileAndParentSync,
}
```

Initial defaults:

```text
SymlinkPolicy::Reject
PlacementStrategy::Copy
DurabilityPolicy::None
```

Reason:

```text
Reject symlinks first is safest and matches archive Prepare hardening.
Hardlink can be enabled after tests prove evidence and fallback behavior.
Full fsync is expensive and platform-sensitive; make it explicit.
```

## Proposed typed design

### Resource type

```rust
pub struct LocalFs {
    staging_root: PathBuf,
    symlink_policy: SymlinkPolicy,
    placement: PlacementStrategy,
    durability: DurabilityPolicy,
    hardlink_threshold_bytes: u64,
}
```

### Apply type

Current:

```rust
LocalApply<O>
```

Recommended migration:

```rust
LocalApply<O, R = DirectLocalFs>
```

Where:

```text
DirectLocalFs keeps current simple behavior for minimal default if desired.
StagedLocalFs / LocalFs resource enables hardened Apply.
```

But avoid over-abstracting. If only one hardened behavior remains, prefer:

```rust
LocalApply<O> { fs: LocalFs }
```

with `Default` using a temp root.

### Evidence

Current `ApplyEvidence` only records target path. Extend or add behavior-specific local evidence:

```rust
pub struct LocalApplyEvidence {
    pub target: PathBuf,
    pub files: usize,
    pub directories: usize,
    pub bytes: u64,
    pub strategy: LocalPlacement,
}

pub enum LocalPlacement {
    Copied,
    Hardlinked,
    Mixed,
    Removed,
}
```

Need to keep evidence chain compatible:

```text
EvidenceChain<E, LocalApplyEvidence>
```

If changing `ApplyEvidence` is too broad, add a local-specific evidence type and gradually retire generic `ApplyEvidence`.

## Proposed implementation behavior

### File Create

```text
1. Check target does not exist.
2. Check source != target using same-file/canonical guard when target exists or parent exists.
3. Create parent directories.
4. Copy or hardlink source to temp file in target parent or staging root on same filesystem.
5. Rename temp file to final target.
6. Record bytes and strategy.
```

### File Replace/CreateOrReplace

```text
1. Check Replace target exists if Replace.
2. Stage new file fully first.
3. Rename staged file over target where platform semantics permit.
4. If Windows replacement semantics require delete first, use backup/rollback path:
   - rename old target to backup
   - rename staged to target
   - remove backup after success
   - restore backup on failure when possible
5. Record evidence.
```

### Directory Create

```text
1. Check target does not exist.
2. Reject source-under-target/target-under-source cycles.
3. Walk source with walkdir.
4. Stage full directory tree under a unique staging directory.
5. Reject symlinks by default.
6. Copy files into staging, count evidence.
7. Rename staging directory to target.
```

### Directory Replace/CreateOrReplace

```text
1. Build full staged directory first.
2. Do not remove target until staging is complete.
3. Use backup/rollback strategy for existing target.
4. Rename staged directory to final target.
5. Cleanup backup after success.
```

### Forget

```text
Forget may remain direct removal, but should distinguish file/dir/symlink metadata and record removed evidence.
For stronger safety, target deletion can be moved to a trash/staging path before final cleanup.
```

## Why not use fs_extra as the core?

`fs_extra` can copy directories, but Pulith needs behavior-specific laws:

```text
typed operation semantics
symlink policy
source/target containment guards
staged replacement
evidence chain
hardlink/copy strategy
failure atomicity
```

A crate that performs generic recursive copy would hide too much of this unless wrapped heavily. The better fit is:

```text
walkdir for enumeration
std::fs/tempfile/same-file for controlled actions
Pulith-owned policy loop
```

## Migration plan

### Slice 1: analysis-only guard document

This document.

### Slice 2: dependency surface

Add optional dependencies:

```text
walkdir
same-file
```

Wire them to `local` feature if local Apply always needs them.

### Slice 3: staged file Apply

Implement staged file apply first:

```text
Create file
Replace file
CreateOrReplace file
```

Tests:

```text
create_file_does_not_overwrite
replace_file_preserves_old_target_if_stage_copy_fails
create_or_replace_file_writes_via_temp_then_rename
local_apply_rejects_same_file_source_and_target
```

### Slice 4: staged directory Apply

Implement directory staging with walkdir:

```text
Create directory
Replace directory
CreateOrReplace directory
```

Tests:

```text
directory_create_stages_tree_before_publish
directory_replace_preserves_old_target_when_stage_fails
directory_apply_rejects_symlink_by_default
directory_apply_rejects_target_inside_source
directory_apply_records_file_directory_byte_counts
```

### Slice 5: optional hardlink strategy

Implement after staged copy is stable:

```text
PlacementStrategy::HardlinkOrCopy
cross-device fallback to copy
strategy evidence: Copied/Hardlinked/Mixed
```

Tests:

```text
hardlink_strategy_falls_back_to_copy_when_link_fails
hardlink_strategy_records_strategy
```

### Slice 6: delete pulith-fs

Only delete `crates/pulith-fs` after:

```text
staged file Apply migrated
staged directory Apply migrated
symlink/default safety migrated
hardlink-or-copy decision made
workspace all-features tests pass
ad-hoc verification passes
```

## Verification requirements

After implementation:

```bash
cargo fmt --all --check
cargo check -p pulith --no-default-features
cargo check -p pulith --features 'sync local'
cargo check -p pulith --features 'sync local hash blake3 zip tar gzip xz zstd'
cargo check --workspace --all-features
cargo test --workspace --all-features
git diff --check -- crates/pulith/src/local.rs crates/pulith/Cargo.toml Cargo.toml docs/report/pulith-file-io-dependency-and-behavior-design.md
```

Ad-hoc script under:

```text
F:\Stratum\TEMP\hermes-verify-*.py
```

Must verify changed file Apply behavior, not just old suite green.

## Decision

Proceed next with file Apply hardening, not net Acquire.

Concrete next implementation target:

```text
LocalApply file path: staged atomic-ish file placement with same-file guard and evidence counts.
```

Then:

```text
staged directory placement with walkdir and symlink reject policy
```

Only after file Apply is robust should net Acquire resume, because network Acquire will land bytes into this local material/staging boundary.
