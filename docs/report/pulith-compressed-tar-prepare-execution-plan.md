# Pulith Compressed TAR Prepare Execution Plan

## Decision

Continue the archive `Prepare` migration before moving to `net Acquire`.

Next executable module:

```text
compressed TAR Prepare, sync/local path only
```

Order:

```text
Tar<Gzip> -> Tar<Xz> -> Tar<Zstd>
```

Rationale:

```text
ZIP Prepare is done.
Plain Tar Prepare is done.
Old pulith-archive still has useful compressed tar behavior.
Compressed tar is still archive materialization, so finishing it keeps the Prepare boundary coherent before moving to network Acquire.
```

## Skill philosophy review

This plan follows the `rust-composable-single-crate-design` skill:

```text
one user-facing pulith crate
feature-gated implementation capabilities
ZST typed backends instead of string/runtime format identity
associated Need/Evidence/Output per behavior
mature crates own codecs/container parsing
Pulith owns policy, path safety, limits, evidence, composition
no old pulith-archive public choreography
no App/Context monolith
no registry/factory/plugin manager
```

Do not migrate these old public shapes:

```text
ArchiveFormat
TarCompress
Decoder as caller-visible protocol
EntrySource
PendingEntry
ArchiveReport
WorkspaceExtraction
extract_from_reader
```

Useful old mechanism reference only:

```rust
flate2::read::GzDecoder::new(reader)
xz2::read::XzDecoder::new(reader)
zstd::stream::Decoder::new(reader)
tar::Archive::new(decoder)
```

## Cargo findings

Workspace already declares the codec crates:

```toml
flate2 = "1.1.8"
xz2 = "0.1.7"
zstd = "0.13.3"
```

Crates.io check:

```text
flate2 1.1.9 — MIT OR Apache-2.0, rust-version 1.67.0
xz2 0.1.7 — MIT/Apache-2.0, rust-version unknown
zstd 0.13.3 — MIT, rust-version 1.64
```

Use existing crates; do not implement gzip/xz/zstd manually.

## Design choice

Prefer typed codec parameterization over flat marker proliferation:

```rust
pub struct Plain;
pub struct Gzip;
pub struct Xz;
pub struct Zstd;
pub struct Tar<C = Plain>(PhantomData<C>);
```

Then current plain tar becomes:

```rust
Tar<Plain>
```

Migration must preserve public ergonomics. If changing `Tar` from a unit ZST to generic risks churn, use a compatibility alias during the same slice:

```rust
pub type Tar = TarArchive<Plain>;
pub struct TarArchive<C = Plain>(PhantomData<C>);
```

But avoid long-term duplicate semantic names. Preferred final public names:

```rust
Tar<Plain>
Tar<Gzip>
Tar<Xz>
Tar<Zstd>
```

## Feature surface

Current feature surface:

```toml
tar = ["archive", "dep:tar"]
compress = ["dep:async-compression"]
```

Proposed sync codec features:

```toml
gzip = ["tar", "dep:flate2"]
xz = ["tar", "dep:xz2"]
zstd = ["tar", "dep:zstd"]
```

Defer `compress`/`async-compression` to a later async-only slice. Do not make sync compressed tar depend on async-compression.

Add deps in `crates/pulith/Cargo.toml`:

```toml
flate2 = { workspace = true, optional = true }
xz2 = { workspace = true, optional = true }
zstd = { workspace = true, optional = true }
```

## Implementation plan

### Slice 1 — generic Tar codec shape

Goal:

```text
Represent plain and compressed tar with typed codec markers without changing behavior semantics.
```

Steps:

1. Replace unit `Tar` with typed codec shape.
2. Add `Plain`, `Gzip`, `Xz`, `Zstd` marker types behind features.
3. Keep `ArchiveNeed<Tar<C>>`, `ArchiveTree<Tar<C>>`, and `ArchiveEvidence<Tar<C>>` unchanged by relying on existing generic archive vocabulary.
4. Keep `ArchiveTree<A> -> LocalApply<_>` generic apply as-is.
5. Preserve plain tar tests under `Tar<Plain>`.

Possible compromise if name churn is awkward:

```rust
pub struct TarArchive<C = Plain>(PhantomData<C>);
pub type Tar = TarArchive<Plain>;
```

But prefer final cleanup over compatibility if the code remains local and tests can be updated immediately.

### Slice 2 — shared TAR extraction core

Goal:

```text
Avoid three copy-pasted extract functions.
```

Add private helper:

```rust
fn extract_tar_reader<A, R: Read>(reader: R, root: &Path, policy: &ArchivePolicy)
    -> Result<ArchiveEvidence<A>, PulithError>
```

This helper owns the existing TAR entry loop:

```text
entries
sanitize_relative
ensure_under_root
reject symlink/hardlink
reject unsupported type
limit checks
file/directory writes
evidence counting
```

Then backend-specific functions only open/decode:

```rust
extract_tar_plain(path, root, policy) -> extract_tar_reader::<Tar<Plain>, _>(File::open(path)?, ...)
extract_tar_gzip(path, root, policy) -> extract_tar_reader::<Tar<Gzip>, _>(GzDecoder::new(file), ...)
extract_tar_xz(path, root, policy) -> extract_tar_reader::<Tar<Xz>, _>(XzDecoder::new(file), ...)
extract_tar_zstd(path, root, policy) -> extract_tar_reader::<Tar<Zstd>, _>(zstd::stream::Decoder::new(file)?, ...)
```

### Slice 3 — backend Prepare impls

Implement one `PrepareNode` impl per enabled codec marker:

```rust
impl<I, E> PrepareNode<Verified<I, LocalMaterial, E>>
    for ArchivePrepare<Tar<Plain>, ExistingExtractRoot>

#[cfg(feature = "gzip")]
impl<I, E> PrepareNode<Verified<I, LocalMaterial, E>>
    for ArchivePrepare<Tar<Gzip>, ExistingExtractRoot>

#[cfg(feature = "xz")]
impl<I, E> PrepareNode<Verified<I, LocalMaterial, E>>
    for ArchivePrepare<Tar<Xz>, ExistingExtractRoot>

#[cfg(feature = "zstd")]
impl<I, E> PrepareNode<Verified<I, LocalMaterial, E>>
    for ArchivePrepare<Tar<Zstd>, ExistingExtractRoot>
```

If this produces obvious boilerplate, factor only the file/material check and `Prepared` construction into a private helper. Do not introduce a registry/factory/middleware layer.

### Slice 4 — tests

Keep behavior tests parallel to plain tar:

```text
tar_gzip_prepare_extracts_archive_tree
tar_gzip_prepare_rejects_parent_path
tar_gzip_prepare_rejects_byte_limit
tar_gzip_prepare_flows_into_local_apply
```

For xz/zstd, minimum tests can be smaller after gzip proves the shared loop:

```text
tar_xz_prepare_extracts_archive_tree
tar_zstd_prepare_extracts_archive_tree
```

But the shared loop must still be covered by plain/gzip hardening tests:

```text
strip-components + directories
entry-count limit
parent path rejection
symlink/hardlink rejection
total-byte limit
local apply continuity
```

Fixtures:

```rust
flate2::write::GzEncoder
xz2::write::XzEncoder
zstd::stream::Encoder
```

Do not add custom compression fixture encoders.

### Slice 5 — report and verification

Add/update report:

```text
docs/report/pulith-compressed-tar-prepare-execution-report.md
```

Run feature matrix:

```bash
cargo fmt --all --check
cargo check -p pulith --no-default-features
cargo check -p pulith --features 'sync local tar'
cargo check -p pulith --features 'sync local tar gzip'
cargo check -p pulith --features 'sync local tar gzip xz zstd'
cargo check -p pulith --features 'sync local zip tar gzip xz zstd'
cargo check -p pulith --features 'sync local hash blake3 tar gzip'
cargo check -p pulith --features 'sync local hash sha2 tar gzip'
cargo check -p pulith --features async
cargo check --workspace --all-features
cargo test --workspace --all-features
git diff --check -- crates/pulith/src/archive.rs crates/pulith/src/lib.rs crates/pulith/Cargo.toml docs/report/pulith-compressed-tar-prepare-execution-report.md
```

Also create focused ad-hoc verification under:

```text
F:\Stratum\TEMP\hermes-verify-*.py
```

It should assert structural markers:

```text
Tar<Plain>/Tar<Gzip>/Tar<Xz>/Tar<Zstd> typed markers exist
flate2/xz2/zstd are used only as private decoder mechanisms
no ArchiveFormat/TarCompress/EntrySource/PendingEntry/ArchiveReport reappears in active pulith archive.rs
compressed tar tests pass
```

## Risks and constraints

### Risk: generic `Tar<C>` name churn

Mitigation:

```text
Perform the generic marker refactor and test update in the same slice.
Avoid keeping both `Tar` and `TarArchive` permanently.
```

### Risk: `xz2` native dependency friction

Mitigation:

```text
Implement gzip first.
Only add xz/zstd after gzip passes the full matrix.
If xz build fails on Windows/MSYS, report concrete linker/system blocker and leave xz feature planned but not claimed implemented.
```

### Risk: private helper over-generalization

Mitigation:

```text
Only extract the shared TAR entry loop.
No public adapter trait.
No dyn EntrySource.
No registry.
```

## Stop condition for next implementation slice

The next implementation slice is done only when:

```text
plain tar still passes
compressed tar backend(s) pass behavior tests
all feature combos compile
workspace all-features tests pass
ad-hoc verification script passes and is cleaned
report records design choices and blockers, if any
```

## Next after compressed TAR

After sync compressed TAR:

```text
net Acquire
```

Recommended order:

```text
1. ureq sync Acquire from URL -> LocalMaterial
2. reqwest async Acquire from URL -> LocalMaterial
3. shared network resource policy: timeout, max bytes, temp/staging path
4. object_store async Acquire only if needed
```

Do not start persistent Remember before net Acquire unless the user explicitly asks for state/storage cleanup first.
