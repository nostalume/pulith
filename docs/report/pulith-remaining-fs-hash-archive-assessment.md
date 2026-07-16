# Pulith Remaining File Mechanisms + Hash/Archive File-I/O Assessment

## Status

Scope completed in this pass:

```text
review remaining pulith-fs mechanisms
assess each by dependency / semantic / behavior
inspect hash file-I/O behavior
inspect archive file-I/O behavior
fix discovered hash/archive stale file-operation behavior
```

Net Acquire remains deferred.

## Summary decision

Do **not** delete `crates/pulith-fs` yet.

The core staged `LocalApply` path has already migrated the most important old file behavior:

```text
same-parent file staging
atomic-ish file placement
staged directory apply
source/target containment guards
same-file guard
symlink rejection by default
files/directories/bytes/strategy evidence
```

But `pulith-fs` still contains mechanisms that are either useful future migrations or need explicit rejection before the crate can be removed.

Deletion gate remains:

```text
hardlink_or_copy strategy: migrate or reject
DurabilityPolicy / fsync: migrate or reject
permission/timestamp preservation: migrate or reject
Windows retry/backoff around directory replacement: migrate or reject
file-lock transaction: reject for Apply or move to Remember/state if needed
symlink creation: reject for current Apply/archive path unless explicit SymlinkPolicy is added
```

## Mechanism assessment

### 1. `primitives::rw::atomic_write`

Source:

```text
crates/pulith-fs/src/primitives/rw.rs
```

Dependency:

```text
uuid only for temporary file names
std::fs write/rename/sync_all
```

Current `pulith` equivalent:

```text
LocalApply file path uses tempfile::NamedTempFile::new_in(target_parent)
persist_noclobber for Create
persist for Replace/CreateOrReplace
```

Assessment:

```text
need: partially
migration: mechanism already migrated for Apply, but not for generic small-file state writes
```

Semantic fit:

- Fits `Remember` / state-file writes better than `LocalApply`, because it writes caller-owned bytes.
- `LocalApply` copies existing material, so `NamedTempFile` is better than the old uuid tmp-path helper.
- `sync` option is still useful as a future typed `DurabilityPolicy`.

Behavior:

- Old implementation writes temp in parent and renames, which is correct atomic-ish placement.
- Old implementation hand-rolls temp names and cleanup; current `tempfile` is safer.
- Old sync behavior only syncs the file, not the parent directory.

Decision:

```text
Do not migrate rw::atomic_write into LocalApply.
Migrate only the concept as future DurabilityPolicy / Remember-state write if needed.
```

### 2. `primitives::hardlink::hardlink_or_copy`

Source:

```text
crates/pulith-fs/src/primitives/hardlink.rs
```

Dependency:

```text
std::fs::hard_link
std::fs::copy fallback
EXDEV / CrossesDevices handling
```

Current `pulith` equivalent:

```text
LocalApply always copies
LocalPlacement has Hardlinked / Mixed reserved but unused
```

Assessment:

```text
need: yes, but not as default behavior
migration: defer to PlacementStrategy::HardlinkOrCopy
```

Semantic fit:

- It should become an explicit typed strategy, not implicit Apply behavior.
- Strategy must be reflected in evidence:

```text
Copied / Hardlinked / Mixed
```

Behavior:

- Good performance for same-device file material.
- Cross-device fallback to copy is useful, but weakens exact behavior guarantees.
- Direct hardlinking can surprise callers because later writes to one link affect same inode/file object.

Decision:

```text
Keep pulith-fs until this is explicitly migrated or rejected.
Recommended future shape: PlacementStrategy::Copy | HardlinkOrCopy { fallback }
```

### 3. `primitives::replace_dir::replace_dir`

Source:

```text
crates/pulith-fs/src/primitives/replace_dir.rs
```

Dependency:

```text
std::fs::rename
std::fs::remove_dir_all
Windows retry/backoff loop
```

Current `pulith` equivalent:

```text
LocalApply uses staged directory + backup rename:
target -> backup
staged -> target
remove backup
restore backup on publish failure
```

Assessment:

```text
need: partially
migration: core semantics migrated; retry/backoff not migrated
```

Semantic fit:

- Directory replacement is not strictly atomic and must remain explicitly "atomic-ish".
- Retry/backoff is a resource/policy concern, not behavior identity.

Behavior:

- Old Windows path removed destination first, then renamed source; that is less safe than current backup pattern.
- Old retry/backoff is still valuable for Windows open-handle races.

Decision:

```text
Do not migrate old replace_dir as-is.
Consider a future DirectoryReplacePolicy { retries, backoff } inside LocalApply resources.
```

### 4. `permissions::PermissionMode`

Source:

```text
crates/pulith-fs/src/permissions.rs
```

Dependency:

```text
std permissions
unix PermissionsExt on Unix
Windows readonly attribute
```

Current `pulith` equivalent:

```text
No permission preservation or explicit permission policy yet.
```

Assessment:

```text
need: maybe
migration: defer
```

Semantic fit:

- Not part of the first `LocalApply` safety contract.
- Should be explicit policy if added:

```text
PermissionPolicy::Inherit | PreserveSource | Explicit(...)
```

Behavior:

- Useful for package installation outputs.
- Can be platform-surprising if implicit.

Decision:

```text
Keep as future policy reference; do not add implicit permission preservation now.
```

### 5. `primitives::copy_dir::copy_dir_all`

Source:

```text
crates/pulith-fs/src/primitives/copy_dir.rs
```

Dependency:

```text
std::fs::read_dir
std::fs::copy
symlink creation helper
```

Current `pulith` equivalent:

```text
LocalApply uses walkdir with follow_links(false)
rejects symlink/special entries by default
copies into TempDir staging before publish
```

Assessment:

```text
need: no as implementation
migration: completed by replacement, not by direct port
```

Semantic fit:

- Old function preserved symlinks by creating symlinks.
- Current Pulith default is reject symlink unless explicit `SymlinkPolicy` exists.

Behavior:

- Old direct copy to destination is less safe than staging.
- Current walkdir staging is better for evidence and safety.

Decision:

```text
Do not migrate. Treat as obsolete for current default semantics.
```

### 6. `primitives::symlink::atomic_symlink`

Source:

```text
crates/pulith-fs/src/primitives/symlink.rs
```

Dependency:

```text
Unix symlink
Windows symlink_file / junction
junction crate
```

Current `pulith` equivalent:

```text
LocalApply and archive extraction reject symlinks by default.
```

Assessment:

```text
need: no for default Apply/archive path
migration: reject for now
```

Semantic fit:

- Creating symlinks is a separate behavior/policy and has security implications.
- Archive extraction should not create links by default.

Behavior:

- Windows symlink/junction semantics differ and can escape expected roots.

Decision:

```text
Do not migrate until an explicit SymlinkPolicy is designed.
```

### 7. `workflow::Workspace`

Source:

```text
crates/pulith-fs/src/workflow/workspace.rs
```

Dependency:

```text
hardlink/copy
atomic_write
replace_dir
relative path sanitizer
report counters
```

Current `pulith` equivalent:

```text
LocalApply now owns staging, placement, guards, and evidence internally.
```

Assessment:

```text
need: no as public choreography
migration: useful mechanisms already copied conceptually
```

Semantic fit:

- Workspace is an App/Context-like choreography object.
- Pulith direction is typed behavior nodes with associated Need/Evidence/Output.

Behavior:

- Its relative path sanitizer and report concept are useful.
- Commit via old replace_dir is weaker than current backup replacement.

Decision:

```text
Do not migrate as public API.
Keep only as reference until remaining hardlink/durability/retry decisions are closed.
```

### 8. `workflow::Transaction`

Source:

```text
crates/pulith-fs/src/workflow/transaction.rs
```

Dependency:

```text
fs2 file locks
read/write/seek/sync_all
```

Current `pulith` equivalent:

```text
No transaction/lock resource in pulith LocalApply.
```

Assessment:

```text
need: not for LocalApply
migration: maybe future Remember/state store only
```

Semantic fit:

- File locking is state/store behavior, not local material placement.
- Could be useful for `Remember` or registry/state writes.

Behavior:

- Existing implementation truncates and writes in place under lock; this is not staged placement.
- Useful for mutual exclusion, not atomic-ish replacement.

Decision:

```text
Do not migrate into Apply. Re-evaluate only when state/Remember storage is implemented.
```

### 9. `align::AlignedBuf`

Source:

```text
crates/pulith-fs/src/align.rs
```

Dependency:

```text
manual alloc/dealloc
unsafe Send/Sync
```

Current `pulith` equivalent:

```text
No direct I/O aligned buffer path.
```

Assessment:

```text
need: no
migration: reject for now
```

Semantic fit:

- No current direct I/O or page-aligned read/write requirement.
- Adds unsafe surface without proven performance need.

Behavior:

- More risk than benefit for current file Apply.

Decision:

```text
Do not migrate unless a benchmark-backed direct I/O path appears.
```

## Hash file-I/O assessment

Source:

```text
crates/pulith/src/hash.rs
```

Dependency:

```text
std::fs::File
std::io::Read
blake3 / sha2 digest crates
```

Semantic assessment:

- Hash verification should only digest regular local files.
- It should not follow symlinks implicitly.
- It should preserve the typed flow:

```text
Acquired<I, LocalMaterial, E> -> Verified<I, LocalMaterial, EvidenceChain<E, DigestEvidence<A>>>
```

Bug / stale behavior found:

```text
Path::is_file() follows symlinks.
read error evidence used Path::new("<stream>") instead of the actual file path.
```

Fix applied:

```text
use symlink_metadata before digesting
reject symlink or non-file material
pass the actual path into copy_into_hasher error reporting
```

Behavior after fix:

```text
regular file -> digest stream
symlink/special/non-file -> UnsupportedDigestMaterial
read failure -> io error with real path
```

## Archive file-I/O assessment

Source:

```text
crates/pulith/src/archive.rs
```

Dependency:

```text
zip / tar / flate2 / xz2 / zstd for container/codec
std::fs for extraction root, directory creation, file creation
LocalApply for final placement
```

Semantic assessment:

Archive Prepare should only unpack into a controlled tree and then flow into typed LocalApply:

```text
Verified<I, LocalMaterial, E>
  -> Prepared<I, ArchiveTree<A>, EvidenceChain<E, ArchiveEvidence<A>>>
  -> LocalApply<Create/Replace/CreateOrReplace>
```

Bug / stale behavior found 1 — extract root reuse:

```text
ExistingExtractRoot was create_dir_all only.
If stale files existed under the extraction root, they could remain there.
Then ArchiveTree -> LocalApply could copy stale files into the final target.
```

Fix applied:

```text
reset_extract_root(root)
```

Behavior:

```text
reject extract root if it is a symlink
remove existing directory/file root
create a fresh extraction root
extract only current archive contents
```

Bug / stale behavior found 2 — pre-existing symlink path under extraction root:

```text
Archive code rejected symlink entries from the archive,
but did not check whether an existing extraction path component was a symlink.
```

Fix applied:

```text
reject_existing_symlink_path(root, target)
```

Now each extracted target path checks existing path components with `symlink_metadata` before writing.

Bug / stale behavior found 3 — ArchiveTree final apply bypassed hardened LocalApply:

```text
ArchiveTree -> LocalApply used its own recursive copy_dir_all.
It removed existing target before copy for Replace/CreateOrReplace.
It did not reuse same staged directory Apply / backup replacement / walkdir symlink policy.
```

Fix applied:

```text
ArchiveTree<A> apply adapter now converts ArchiveTree<A> into LocalPrepared { kind: Directory }
and delegates to LocalApply<O>::apply_node.
```

Behavior after fix:

```text
Archive final placement uses the same staged directory Apply as normal local material.
Create / Replace / CreateOrReplace laws stay consistent.
Evidence is produced by hardened LocalApply.
```

## Deletion recommendation

Current recommendation:

```text
Do not delete crates/pulith-fs yet.
```

Reason by category:

### Dependency

`pulith-fs` still owns dependency examples not yet modeled in `crates/pulith`:

```text
fs2 locks
junction Windows link behavior
uuid temp naming history
manual aligned allocation
```

Some are likely rejected, but that rejection is not yet encoded in docs/tests/typed policies.

### Semantic

Current `pulith` semantics have intentionally chosen:

```text
copy-first default
reject symlink default
typed evidence
staged placement
```

But these old semantics remain undecided:

```text
hardlink-or-copy
permission preservation
durability/fsync
state transaction locking
```

### Behavior

Current `LocalApply` is safer than old `copy_dir_all` and old `replace_dir`, but old `pulith-fs` still contains operational knowledge:

```text
Windows retry/backoff
hardlink cross-device fallback
permission application
file lock transaction
```

Therefore delete only after the next explicit prune pass decides each item as migrated or rejected.

## Next recommended slice

If continuing file behavior:

```text
1. Add explicit PlacementStrategy::Copy | HardlinkOrCopy or reject hardlink permanently.
2. Add DurabilityPolicy::None | FileSync | FileAndParentSync or reject durability for Apply.
3. Decide PermissionPolicy::Inherit | PreserveSource | Explicit or reject.
4. Decide Windows retry/backoff resource for directory replacement.
5. Then delete pulith-fs if no remaining mechanism is needed.
```
