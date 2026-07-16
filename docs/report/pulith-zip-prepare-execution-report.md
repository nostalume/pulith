# Pulith ZIP Prepare Execution Report

## Status

Implemented the first archive migration slice:

```text
ZIP only
sync only
local file material only
typed Prepare node
```

This is not a clone of old `pulith-archive`. It is a typed behavior-tree implementation:

```text
Verified<I, LocalMaterial, E>
  -> Prepared<I, ArchiveTree<Zip>, EvidenceChain<E, ArchiveEvidence<Zip>>>
```

## Files changed

```text
crates/pulith/src/archive.rs
crates/pulith/src/error.rs
crates/pulith/src/lib.rs
docs/report/pulith-zip-prepare-execution-report.md
```

## Implemented public typed nodes

Behind `archive` / `zip` features:

```rust
Zip
ArchivePolicy
ArchiveNeed<A>
ArchiveTree<A>
ArchiveEvidence<A>
ExistingExtractRoot
ArchivePrepare<A, R>
```

The primary behavior implementation is:

```rust
impl<I, E> PrepareNode<Verified<I, LocalMaterial, E>>
    for ArchivePrepare<Zip, ExistingExtractRoot>
```

Associated types:

```rust
type Need = ArchiveNeed<Zip>;
type Prepared = ArchiveTree<Zip>;
type Evidence = ArchiveEvidence<Zip>;
type Output = Prepared<I, ArchiveTree<Zip>, EvidenceChain<E, ArchiveEvidence<Zip>>>;
```

## Mechanism boundary

ZIP parsing/decompression is delegated to the mature `zip` crate:

```rust
zip::ZipArchive::new(file)
archive.by_index(index)
```

Pulith does not implement ZIP parsing or compression codecs.

Pulith owns:

```text
feature-gated ZST behavior marker
ArchiveNeed policy
ArchiveTree output
ArchiveEvidence facts
path safety
entry-count limit
total-byte limit
symlink rejection for first slice
composition into LocalApply
```

## Safety behavior implemented

The ZIP Prepare slice currently enforces:

```text
source material must be a local file
extract root is created explicitly
zip-slip entries are rejected through enclosed-name/path validation
absolute/root/prefix/parent components are rejected
strip-components can skip leading normal path components
entry count limit is checked before extraction continues
total uncompressed byte limit is checked before file write
symlinks are rejected in the first slice
files are written only under extraction root
```

## Local Apply continuity

`ArchiveTree<Zip>` can flow into local apply for:

```text
Create
Replace
CreateOrReplace
```

This proves the typed tree path:

```text
Intent
 -> WithSource<LocalPath>
 -> Chosen
 -> LocalAcquire
 -> IdentityVerify / HashVerify
 -> ArchivePrepare<Zip>
 -> LocalApply<CreateOrReplace>
```

## Tests added

```text
zip_prepare_extracts_archive_tree
zip_prepare_honors_strip_components_and_directories
zip_prepare_rejects_entry_limit
zip_prepare_rejects_zip_slip_path
zip_prepare_rejects_symlink_entry
zip_prepare_rejects_byte_limit
zip_prepare_flows_into_local_apply
```

These run alongside existing local typed-tree tests.

## Follow-up compression/decompression plan

### Rule

```text
Do not implement codec algorithms manually.
Use existing crates for mechanism.
Keep Pulith-owned typed behavior, policy, resource limits, path safety, and evidence.
```

### ZIP

Current status:

```text
implemented via zip crate
```

Completed ZIP hardening checks:

```text
strip-components behavior
directory entry counting
symlink rejection
zip-slip rejection
entry-count limit
total-byte limit
ArchiveTree<Zip> continuity into LocalApply
```

Remaining ZIP refinements:

```text
add explicit symlink policy type instead of hard reject only
add optional overwrite/clean extraction-root policy
consider explicit compression method evidence if needed
```

### TAR

Next archive backend:

```text
Tar ZST
ArchivePrepare<Tar>
tar::Archive<R>
```

Plain TAR should be implemented before compressed TAR.

### Compressed TAR

Model compression as typed stream codec, not manual decoding:

```text
Tar<Gzip> -> flate2::read::GzDecoder
Tar<Xz>   -> xz2::read::XzDecoder
Tar<Zstd> -> zstd::stream::Decoder
```

Possible typed shapes:

```rust
pub struct Gzip;
pub struct Xz;
pub struct Zstd;
pub struct Tar<C = NoCompression>(PhantomData<C>);
```

Then:

```rust
ArchivePrepare<Tar<NoCompression>>
ArchivePrepare<Tar<Gzip>>
ArchivePrepare<Tar<Xz>>
ArchivePrepare<Tar<Zstd>>
```

### Async compression/decompression

Use `async-compression` only for async paths:

```text
AsyncArchivePrepare<Tar<Gzip>, TokioIo>
AsyncArchivePrepare<Tar<Xz>, TokioIo>
AsyncArchivePrepare<Tar<Zstd>, TokioIo>
```

Do not force async-compression into sync ZIP/TAR paths.

### Archive creation/compression

Creation/compression should be a separate behavior from extraction Prepare.

Candidate behavior names:

```text
Pack
Compress
ArchiveWrite
```

Do not overload `Prepare` with both extraction and archive creation. Extraction prepares material for Apply. Packing/compressing creates an archive artifact and should have its own Need/Evidence/Output.

Possible later shape:

```rust
PackArchive<Zip>
PackNeed<Zip>
PackedArchive<Zip>
PackEvidence<Zip>
```

## Next migration recommendation

Before moving to net Acquire, finish archive in this order:

1. Harden ZIP Prepare tests: strip-components, directory entries, symlink rejection.
2. Add plain `Tar` Prepare using `tar` crate.
3. Add typed compressed TAR variants using `flate2`/`xz2`/`zstd` only where features are enabled.
4. Decide whether archive creation belongs now or after persistent Remember.
5. Then migrate net Acquire (`reqwest`/`ureq`).

## Non-goals preserved

```text
No App/Context monolith.
No EntrySource/PendingEntry public protocol.
No detached ArchiveReport public result.
No manual ZIP/TAR/DEFLATE/gzip/xz/zstd implementation.
No runtime string format switch for static behavior path.
```
