# Pulith File I/O Implementation Analysis Before Coding

## Question

Before implementing file Apply hardening, validate whether the implementation plan is grounded in file-operation knowledge, especially atomic-ish placement.

## Short answer

The previous document searched and evaluated crates, but it was not yet enough for implementation. It covered dependency suitability, not all low-level filesystem semantics needed to implement atomic-ish placement safely.

This document fills that gap before code changes.

## Sources checked now

Checked documentation/knowledge for:

```text
std::fs::rename
std::fs::copy
tempfile::NamedTempFile
tempfile::TempDir
same-file
walkdir
atomic-write-file
```

Also ran a local Windows/MSYS behavior probe for replacement semantics.

## Key filesystem facts for implementation

### std::fs::rename

Relevant documented behavior:

```text
rename replaces the original file if destination already exists.
Behavior differs by platform when both source and destination exist.
On Unix, directory replacement requires destination to be an empty directory.
On Windows, rename maps to MoveFileExW / SetFileInformationByHandle behavior.
Rename across mount points/filesystems fails.
```

Implementation implication:

```text
Use rename/replace for file publish only when staging is on the same filesystem as target.
For directory replacement, do not assume rename can replace a non-empty existing directory.
```

### Local Windows probe

Probe result on this host:

```text
os.replace(file, existing_file) succeeded
os.replace(staged_dir, existing_nonempty_dir) failed with PermissionError(13, '拒绝访问。')
```

Implementation implication:

```text
File replacement can use staged temp file + atomic-ish replace.
Directory replacement needs backup/rollback choreography, not a single rename over existing directory.
```

### std::fs::copy

Relevant documented behavior:

```text
std::fs::copy overwrites destination contents.
If source and destination point to the same file, the file will likely be truncated.
On Linux it may use copy_file_range/sendfile/splice.
On Windows it maps to CopyFileEx.
```

Implementation implication:

```text
Never call fs::copy(source, target) directly for final target placement.
Never call fs::copy before same-file guard.
Copy into a staging/temp path first.
```

### tempfile::NamedTempFile

Relevant documented behavior:

```text
NamedTempFile::persist atomically replaces target if target exists.
Failure returns the temp file inside PersistError.
Temporary files cannot be persisted across filesystems.
Neither file contents nor containing directory are synchronized by default.
```

Implementation implication:

```text
NamedTempFile in the target parent is a good primitive for file placement.
It naturally handles unique temp path and cleanup.
Do not place temp file in a global temp directory if final target may be on a different filesystem.
If durability is requested, explicitly sync file and maybe parent directory before/after persist.
```

### tempfile::TempDir

Relevant documented behavior:

```text
TempDir creates a directory and removes it on drop.
Can be created in a chosen location with Builder.
```

Implementation implication:

```text
Use TempDir for directory staging and operation cleanup.
Create staging under the target parent or a configured staging root guaranteed to be same-device when final rename is required.
```

### atomic-write-file

Relevant documented behavior:

```text
Creates a temporary file in the same directory as destination.
Writes, fsyncs, then renames over destination.
If path is a symlink, the symlink is replaced, not the target.
```

Implementation implication:

```text
This confirms the correct atomic file pattern.
But it mainly writes new in-memory content, while Pulith Apply copies existing files/directories.
Use the pattern; do not add the dependency for the copy slice unless later small-file state writes need it.
```

### same-file

Relevant documented behavior:

```text
Provides cross-platform detection that two paths refer to the same file or directory.
```

Implementation implication:

```text
Use same-file to prevent source == target and std::fs::copy truncation.
Still need containment checks for directory source/target relationships.
```

### walkdir

Relevant documented behavior:

```text
Efficient cross-platform recursive directory traversal.
Symbolic link following is off by default.
Can report errors and detect loops when following symlinks.
```

Implementation implication:

```text
Use walkdir for deterministic directory enumeration.
Keep symlink following disabled.
Reject symlink entries explicitly by default.
Pulith remains responsible for copy policy, evidence, and staging.
```

## Concrete implementation design

### File Create

Algorithm:

```text
1. Require target not exists.
2. Ensure source is regular file material.
3. If target exists or can be resolved, reject same-file source/target via same-file where applicable.
4. Create target parent directories.
5. Create NamedTempFile in target parent.
6. Copy source bytes into temp file.
7. Optionally sync temp file according to DurabilityPolicy.
8. Persist temp file to target with no-clobber semantics for Create if possible.
9. Record LocalApplyEvidence.
```

Important detail:

```text
Create should not use a replace-style persist if the target appears after preflight. Prefer persist_noclobber if available, or handle race by checking final operation semantics.
```

### File Replace

Algorithm:

```text
1. Require target exists.
2. Reject source == target.
3. Create NamedTempFile in target parent.
4. Copy source bytes into temp file.
5. Optionally sync temp file.
6. Persist temp file over target.
7. Record LocalApplyEvidence.
```

Failure property:

```text
If copy fails, target has not been touched.
If persist fails, tempfile error retains temp; target should usually remain old target unless platform failure occurs at final replacement boundary.
```

### File CreateOrReplace

Algorithm:

```text
Same as File Replace but target existence is optional.
Stage fully before replacing/creating.
```

### Directory Create

Algorithm:

```text
1. Require target not exists.
2. Reject source == target.
3. Reject target inside source and source inside target when both can be canonicalized or lexically resolved enough.
4. Create TempDir under target parent, e.g. .pulith-stage-XXXX.
5. Walk source with walkdir, follow_links(false).
6. For each directory, create relative directory in staging.
7. For each regular file, copy into staging relative path.
8. For symlink or special file, reject by default.
9. Rename completed staging directory to target.
10. Record files/directories/bytes.
```

Failure property:

```text
If traversal/copy fails, TempDir cleanup removes staging and target is untouched.
```

### Directory Replace/CreateOrReplace

Algorithm:

```text
1. Build full staged directory under target parent first.
2. If target does not exist and op allows create, rename staged -> target.
3. If target exists:
   a. Rename target -> backup sibling.
   b. Rename staged -> target.
   c. Remove backup after success.
   d. If staged -> target fails, try to restore backup -> target.
```

Failure property:

```text
This is atomic-ish, not strictly atomic.
There is a small window where target path is absent or backup exists.
But old target is not deleted before the replacement is ready, and rollback is possible.
```

Reason:

```text
Directory replacement over non-empty target is not portable as a single atomic operation, confirmed by Windows probe and std::fs::rename docs.
```

## Required policies before code

Start with these defaults:

```rust
SymlinkPolicy::Reject
PlacementStrategy::Copy
DurabilityPolicy::None
```

Why:

```text
Rejecting symlinks mirrors archive Prepare safety.
Copy-first avoids hardlink evidence complexity.
Durability fsync is expensive and platform-sensitive, so make it explicit later.
```

## Dependencies to use in first implementation

```text
tempfile: yes, already available at workspace level.
walkdir: add for local directory traversal.
same-file: add for same-object guard.
```

Do not add for first slice:

```text
fs_extra: too high-level for Pulith policy/evidence loop.
atomic-write-file: useful pattern, but not needed for file-copy Apply.
jwalk: parallel traversal later, not correctness slice.
filetime: defer until timestamp preservation policy exists.
```

## Tests required before claiming implementation complete

File tests:

```text
create_file_does_not_overwrite
replace_file_stages_before_touching_target
create_or_replace_file_rejects_same_file_source_target
file_apply_records_bytes_and_counts
```

Directory tests:

```text
directory_create_publishes_only_after_full_stage
directory_replace_preserves_old_target_when_stage_fails
directory_replace_uses_backup_restore_shape
directory_apply_rejects_symlink_by_default
directory_apply_rejects_target_inside_source
directory_apply_records_file_directory_byte_counts
```

Feature/matrix tests:

```text
cargo fmt --all --check
cargo check -p pulith --no-default-features
cargo check -p pulith --features 'sync local'
cargo check --workspace --all-features
cargo test --workspace --all-features
ad-hoc F:\Stratum\TEMP\hermes-verify-*.py for changed file behavior
```

## Decision

Do not implement from the earlier high-level design alone.

Implement only after encoding these concrete filesystem semantics into the code plan:

```text
file placement: NamedTempFile in target parent + persist/persist_noclobber
file replacement: staged copy before persist
same-file guard: same-file crate
folder traversal: walkdir follow_links(false)
directory replacement: staged tree + backup/rollback, explicitly atomic-ish not atomic
symlinks: reject by default
evidence: files/directories/bytes/placement strategy
```
