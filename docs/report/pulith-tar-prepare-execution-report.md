# Pulith TAR Prepare Execution Report

## Status

Implemented the next archive migration slice:

```text
plain TAR only
sync only
local file material only
typed Prepare node
```

This follows the skill philosophy review:

```text
one pulith crate
feature-gated implementation type, not old crate compatibility
ZST capability marker
associated Need/Evidence/Output
no App/Context monolith
no EntrySource/PendingEntry/ArchiveReport public choreography
mature crate owns archive mechanism
Pulith owns path safety, limits, typed evidence, and composition
```

## Files changed

```text
crates/pulith/src/archive.rs
crates/pulith/src/lib.rs
docs/report/pulith-tar-prepare-execution-report.md
```

## Implemented public typed node

Behind `tar` feature:

```rust
Tar
```

Existing generic archive vocabulary is reused:

```rust
ArchiveNeed<Tar>
ArchiveTree<Tar>
ArchiveEvidence<Tar>
ArchivePrepare<Tar, ExistingExtractRoot>
```

The primary behavior implementation is:

```rust
impl<I, E> PrepareNode<Verified<I, LocalMaterial, E>>
    for ArchivePrepare<Tar, ExistingExtractRoot>
```

Associated types:

```rust
type Need = ArchiveNeed<Tar>;
type Prepared = ArchiveTree<Tar>;
type Evidence = ArchiveEvidence<Tar>;
type Output = Prepared<I, ArchiveTree<Tar>, EvidenceChain<E, ArchiveEvidence<Tar>>>;
```

## Mechanism boundary

Plain TAR parsing is delegated to the mature `tar` crate:

```rust
tar::Archive::new(file)
archive.entries()
```

Pulith does not implement TAR parsing.

Pulith owns:

```text
feature-gated Tar ZST marker
ArchiveNeed policy
ArchiveTree output
ArchiveEvidence facts
path safety
entry-count limit
total-byte limit
symlink/hardlink rejection for first slice
composition into LocalApply
```

## Generic apply continuity

`ArchiveTree<Zip>`-specific local apply impls were generalized to:

```rust
ArchiveTree<A> -> LocalApply<Create>
ArchiveTree<A> -> LocalApply<Replace>
ArchiveTree<A> -> LocalApply<CreateOrReplace>
```

This keeps Apply behavior about a prepared archive tree, not about a specific archive backend.

## Safety behavior implemented

The TAR Prepare slice enforces:

```text
source material must be a local file
extract root is created explicitly
parent/root/prefix path components are rejected
strip-components can skip leading normal path components
entry count limit is checked before extraction continues
total uncompressed byte limit is checked before file write
symlinks and hard links are rejected in the first slice
unsupported TAR entry types are rejected
files are written only under extraction root
```

## Tests added

```text
tar_prepare_extracts_archive_tree
tar_prepare_honors_strip_components_and_directories
tar_prepare_rejects_entry_limit
tar_prepare_rejects_parent_path
tar_prepare_rejects_symlink_entry
tar_prepare_rejects_byte_limit
tar_prepare_flows_into_local_apply
```

The parent-path test uses a raw test fixture patch because `tar::Header::set_path` refuses `..` paths at fixture construction time. The production path still validates entries independently through Pulith's `sanitize_relative` path policy.

## Design philosophy check

The implementation intentionally avoided drifting away from the skill:

```text
No old pulith-archive clone.
No public EntrySource/PendingEntry adapter protocol.
No detached ArchiveReport result bag.
No ArchiveFormat runtime switch in the static path.
No manual codec/container implementation.
No App/Context/Workspace monolith.
No registry/factory/plugin manager.
```

The implementation stayed inside the typed tree:

```text
Verified<I, LocalMaterial, E>
  -> Prepared<I, ArchiveTree<Tar>, EvidenceChain<E, ArchiveEvidence<Tar>>>
```

## Follow-up compression/decompression plan

### Rule

```text
Do not implement codec algorithms manually.
Use existing crates for mechanism.
Keep Pulith-owned typed behavior, policy, resource limits, path safety, and evidence.
```

### Next backend variants

Plain TAR is now implemented. Compressed TAR should be typed codec variants, not a string format switch:

```rust
pub struct Gzip;
pub struct Xz;
pub struct Zstd;
pub struct Tar<C = NoCompression>(PhantomData<C>);
```

Migration order:

```text
Tar<Gzip> via flate2::read::GzDecoder
Tar<Xz> via xz2::read::XzDecoder
Tar<Zstd> via zstd::stream::Decoder
```

Async archive decompression should be a later async-only path:

```text
AsyncArchivePrepare<Tar<Gzip>, TokioIo> via async-compression
AsyncArchivePrepare<Tar<Xz>, TokioIo> via async-compression
AsyncArchivePrepare<Tar<Zstd>, TokioIo> via async-compression
```

Do not force async-compression into sync TAR/ZIP paths.

## Next migration recommendation

Before net Acquire:

```text
1. Decide typed compression marker shape: Tar<Gzip> vs TarGzip.
2. Add gzip TAR first with flate2.
3. Then xz/zstd behind explicit features/deps.
4. Keep Pack/Compress archive creation separate from extraction Prepare.
```
