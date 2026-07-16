# Pulith Local File Apply Hardening Execution Report

## Status

Implemented the file-interaction hardening slice requested after the file I/O analysis.

Scope implemented:

```text
walkdir / same-file optional dependencies
staged file Apply
same-file source-target guard
atomic-ish file Create / Replace / CreateOrReplace
staged directory Apply using walkdir
default symlink rejection for directory Apply
Apply evidence: files / directories / bytes / strategy
```

Net Acquire remains intentionally postponed.

## Files changed

```text
Cargo.toml
crates/pulith/Cargo.toml
crates/pulith/src/evidence.rs
crates/pulith/src/error.rs
crates/pulith/src/lib.rs
crates/pulith/src/local.rs
crates/pulith/src/archive.rs
```

`archive.rs` was adjusted only because `ApplyEvidence` now has richer fields; archive-tree local apply now records copied tree stats instead of constructing path-only evidence.

## Dependency changes

Added workspace dependencies:

```toml
same-file = "1"
walkdir = "2.5"
```

Added optional local dependencies in `crates/pulith/Cargo.toml`:

```toml
local = ["dep:same-file", "dep:tempfile", "dep:walkdir"]

same-file = { workspace = true, optional = true }
tempfile = { workspace = true, optional = true }
walkdir = { workspace = true, optional = true }
```

Reason:

```text
same-file: cross-platform same-object guard before copying
walkdir: robust directory traversal while Pulith owns policy/evidence
tempfile: same-directory staged files/directories and cleanup ownership
```

## Behavior changes

### File Apply

Previous behavior:

```text
fs::copy(source, target)
Replace/CreateOrReplace removed target before copy
```

New behavior:

```text
create target parent
create NamedTempFile in target parent
copy source file into temp file
publish temp file to final target
```

Create uses no-clobber publish:

```text
NamedTempFile::persist_noclobber(target)
```

Replace/CreateOrReplace use replacement publish:

```text
NamedTempFile::persist(target)
```

Properties:

```text
source is fully copied before final target mutation
copy failure leaves old target untouched
same filesystem placement is preserved by creating temp file in target parent
same-file source/target is rejected before copy
```

### Directory Apply

Previous behavior:

```text
recursive std::fs copy directly into final target
Replace/CreateOrReplace removed target before copy
```

New behavior:

```text
create TempDir under target parent
walk source with walkdir follow_links(false)
copy regular files into staging tree
reject symlink/special entries by default
publish completed staging tree
```

Create:

```text
rename staged directory -> target
```

Replace/CreateOrReplace when target exists:

```text
rename target -> backup sibling
rename staged directory -> target
remove backup on success
attempt backup restore on failure
```

This is explicitly atomic-ish rather than strictly atomic, because portable non-empty directory replacement cannot be modeled as one atomic rename.

### Guards

Added:

```text
same-file guard for existing target paths
source/target containment guard for directory Apply
source symlink/special-entry rejection
walkdir symlink-entry rejection
```

Errors added:

```rust
PulithError::ApplySameFile(PathBuf)
PulithError::ApplyPathConflict { source, target }
PulithError::UnsupportedLocalEntry(PathBuf)
```

## Evidence changes

`ApplyEvidence` now records:

```rust
pub struct ApplyEvidence {
    pub target: PathBuf,
    pub files: usize,
    pub directories: usize,
    pub bytes: u64,
    pub strategy: LocalPlacement,
}
```

Added:

```rust
pub enum LocalPlacement {
    Copied,
    Hardlinked,
    Mixed,
    Removed,
}

pub struct LocalApplyStats {
    pub files: usize,
    pub directories: usize,
    pub bytes: u64,
    pub strategy: LocalPlacement,
}
```

Current implemented strategy is `Copied`; `Hardlinked` and `Mixed` are reserved for a future hardlink-or-copy slice.

## Tests added/updated

Local file tests now cover:

```text
create_and_replace_are_typed_apply_laws
create_or_replace_rejects_same_file_source_target
file_replace_stages_before_touching_target
```

Directory tests now cover:

```text
directory_create_stages_tree_and_records_counts
directory_replace_preserves_old_target_when_preflight_fails
directory_create_rejects_target_inside_source
directory_apply_rejects_symlink_by_default  # unix only
```

The existing typed local flow test remains:

```text
local_tree_runs_create_or_replace_file
```

## Verification so far

Ran:

```bash
cargo fmt --all
cargo check -p pulith --no-default-features
cargo check -p pulith --features 'sync local'
cargo check -p pulith --features 'sync local hash blake3 zip tar gzip xz zstd'
cargo check --workspace --all-features
cargo test --workspace --all-features
```

Result:

```text
PASS
29 tests passed; 0 failed
```

## pulith-fs deletion decision

Do **not** delete `crates/pulith-fs` yet.

Reason:

```text
The main staged file/directory Apply behavior has migrated, but not every useful pulith-fs mechanism has been intentionally accepted or rejected:
- hardlink_or_copy strategy is not implemented yet
- durability/fsync policy is not implemented yet
- permission/timestamp preservation policy is not implemented yet
- Windows retry/backoff around directory replacement is only partially superseded by backup/rollback shape
```

Next deletion gate:

```text
Delete pulith-fs after either migrating or explicitly rejecting those remaining mechanisms in a follow-up design/execution slice.
```

## Next recommended slice

Before returning to net Acquire:

```text
1. Decide whether to implement PlacementStrategy::HardlinkOrCopy.
2. Decide whether DurabilityPolicy::FileSync / FileAndParentSync is needed.
3. Decide whether permission/timestamp preservation is a Pulith requirement.
4. Then delete pulith-fs if no remaining behavior is needed.
```
