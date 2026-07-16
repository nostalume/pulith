# Pulith Compressed TAR Prepare Execution Report

## Status

Implemented the compressed TAR Prepare slice:

```text
Tar<Plain>
Tar<Gzip>
Tar<Xz>
Tar<Zstd>
```

Scope:

```text
sync only
local file material only
typed Prepare node
mature codec crates for decompression
shared Pulith-owned TAR extraction policy loop
```

## Files changed

```text
crates/pulith/Cargo.toml
crates/pulith/src/archive.rs
crates/pulith/src/lib.rs
docs/report/pulith-compressed-tar-prepare-execution-plan.md
docs/report/pulith-compressed-tar-prepare-execution-report.md
```

## Design philosophy check

The implementation stays inside the `rust-composable-single-crate-design` philosophy:

```text
one user-facing pulith crate
feature-gated backend capabilities
ZST typed behavior markers
associated Need/Evidence/Output
static typed composition before runtime selection
mature crates own compression/container mechanics
Pulith owns path safety, limits, evidence, and composition
```

The implementation intentionally did not reintroduce:

```text
ArchiveFormat
TarCompress
EntrySource
PendingEntry
ArchiveReport
WorkspaceExtraction
extract_from_reader
registry/factory/plugin manager
App/Context monolith
```

## Public typed markers

Plain TAR is now the default codec parameter:

```rust
pub struct Plain;
pub struct Tar<C = Plain>;
```

Compressed TAR markers are feature gated:

```rust
pub struct Gzip; // feature gzip
pub struct Xz;   // feature xz
pub struct Zstd; // feature zstd
```

Public typed Prepare shapes:

```rust
ArchivePrepare<Tar<Plain>, ExistingExtractRoot>
ArchivePrepare<Tar<Gzip>, ExistingExtractRoot>
ArchivePrepare<Tar<Xz>, ExistingExtractRoot>
ArchivePrepare<Tar<Zstd>, ExistingExtractRoot>
```

Each maps:

```text
Verified<I, LocalMaterial, E>
  -> Prepared<I, ArchiveTree<Tar<C>>, EvidenceChain<E, ArchiveEvidence<Tar<C>>> >
```

## Feature surface

Added sync codec features:

```toml
gzip = ["tar", "dep:flate2"]
xz = ["tar", "dep:xz2"]
zstd = ["tar", "dep:zstd"]
```

Added optional deps:

```toml
flate2 = { workspace = true, optional = true }
xz2 = { workspace = true, optional = true }
zstd = { workspace = true, optional = true }
```

`compress` / `async-compression` remains untouched for a later async-only slice.

## Mechanism boundary

Codec crates are private mechanisms:

```rust
flate2::read::GzDecoder::new(file)
xz2::read::XzDecoder::new(file)
zstd::stream::Decoder::new(file)
```

TAR parsing remains delegated to:

```rust
tar::Archive::new(reader)
```

Pulith owns the shared extraction policy loop:

```rust
extract_tar_reader<A, R: Read>(reader, root, policy)
```

That loop handles:

```text
entry iteration
strip-components
parent/root path rejection
root containment check
symlink/hardlink rejection
unsupported entry rejection
entry-count limit
total-byte limit
file/directory writes
evidence counting
```

## Tests added

Compressed TAR tests:

```text
tar_gzip_prepare_extracts_archive_tree
tar_gzip_prepare_rejects_parent_path
tar_gzip_prepare_rejects_byte_limit
tar_gzip_prepare_flows_into_local_apply
tar_xz_prepare_extracts_archive_tree
tar_zstd_prepare_extracts_archive_tree
```

Plain TAR tests still cover the full hardening loop:

```text
tar_prepare_extracts_archive_tree
tar_prepare_honors_strip_components_and_directories
tar_prepare_rejects_entry_limit
tar_prepare_rejects_parent_path
tar_prepare_rejects_symlink_entry
tar_prepare_rejects_byte_limit
tar_prepare_flows_into_local_apply
```

ZIP tests remain unchanged and continue to run under all features.

## Fixture note

`tar::Header::set_path` rejects `..` at fixture-construction time. For invalid-path tests, the fixture creates a safe TAR header, patches the raw header path bytes, and recomputes the TAR checksum. For gzip, the invalid TAR bytes are patched before compression.

This is test-only fixture construction; production code still validates paths through Pulith's shared `sanitize_relative` policy before writing.

## Implementation summary

The implementation avoided duplicated extraction policy by using one shared TAR reader loop. Backend-specific code is only open/decode:

```text
Tar<Plain> -> File
Tar<Gzip>  -> GzDecoder<File>
Tar<Xz>    -> XzDecoder<File>
Tar<Zstd>  -> zstd::stream::Decoder<File>
```

`ArchiveTree<A> -> LocalApply<_>` remains generic, so compressed archive trees flow into the same Create/Replace/CreateOrReplace apply path without codec-specific apply impls.

## Next migration recommendation

Archive Prepare is now coherent enough to move next to:

```text
net Acquire
```

Recommended order:

```text
1. ureq sync Acquire from URL -> LocalMaterial
2. reqwest async Acquire from URL -> LocalMaterial
3. shared network resource policy: timeout, max bytes, staging/temp root
4. object_store async Acquire only if needed
```

Do not start persistent Remember before net Acquire unless storage/state cleanup is explicitly prioritized.
