# Source / Fetch / Verify / Store First-Principles Boundary Evaluation

## Status

Design/evaluation artifact only. Do not edit Rust code from this report alone.

This report is the next Phase A artifact after:

- `docs/architecture.md` defined the DDD concept chain;
- `docs/report/phase-a-ddd-crate-layout-evaluation.md` evaluated active crates by concept;
- `docs/report/phase-a-first-principles-crate-necessity.md` evaluated crate necessity and single-crate compile tradeoffs.

Scope here:

```text
Source Offer -> Materialized Evidence -> Artifact Memory
```

Crates read for this report:

- `crates/pulith-source/src/lib.rs`
- `crates/pulith-fetch/src/lib.rs`
- `crates/pulith-fetch/src/fetch/fetcher.rs`
- `crates/pulith-fetch/src/fetch/multi_source.rs`
- `crates/pulith-fetch/src/config/sources.rs`
- `crates/pulith-fetch/src/codec/verify.rs`
- `crates/pulith-verify/src/reader.rs`
- `crates/pulith-store/src/lib.rs`

## First-principles questions

A resource manager asks these questions in order:

```text
1. What do I want?
2. Where may acceptable material come from?
3. Which candidate should I execute, and how?
4. What bytes/tree did I actually get?
5. How do I prove and describe it?
6. How do I remember it for later install/state workflows?
```

This report covers questions 2-6.

## Concept definitions

### Source Offer

Question answered:

```text
Where may acceptable material come from?
```

Source Offer is semantic. It describes possible origins before transfer begins.

It owns:

- source declarations;
- mirror/local/git/direct URL families;
- expansion into concrete candidates;
- ordered/race/exhaustive candidate intent requested by the caller.

It must not own:

- HTTP client behavior;
- retry/backoff;
- checksum execution;
- cache freshness execution;
- store metadata.

Current owner:

```text
pulith-source
```

Current concrete objects:

```text
SourceDefinition
RemoteSource
SourceSet
SourceSpec
PlannedSources
ResolvedSourceCandidate
SelectionStrategy
```

Current interface into Materialized Evidence:

```rust
PlannedSources::candidates() -> &[ResolvedSourceCandidate]
PlannedSources::strategy() -> &SelectionStrategy
MultiSourceFetcher::fetch_planned_sources_with_receipt(...)
```

### Transfer Execution

Question answered:

```text
How do I execute a source candidate and place the resulting bytes at a destination?
```

Transfer Execution is a sub-concept of Materialized Evidence. It is not Source Offer.

It owns:

- HTTP/local transfer execution;
- retry/backoff mechanics;
- progress reporting;
- destination placement through `pulith-fs` workspace;
- fetch receipt generation.

It must not own:

- semantic source declaration vocabulary;
- caller ranking policy beyond executing a selected/planned strategy;
- long-term artifact memory;
- lifecycle state.

Current owner:

```text
pulith-fetch
```

Current concrete objects:

```text
Fetcher
MultiSourceFetcher
FetchOptions
RetryPolicy
FetchSource
FetchReceipt
HttpClient / ReqwestClient
```

### Integrity Evidence

Question answered:

```text
What digest/size was observed, and did it satisfy the caller's requirement?
```

Integrity Evidence is a proof primitive. It should have one canonical vocabulary.

Current intended owner:

```text
pulith-verify
```

Current concrete objects:

```text
Hasher
Sha256Hasher
VerifiedReader
VerificationReceipt
verify_stream(...)
```

Current duplicate/adapter objects in `pulith-fetch`:

```text
HashAlgorithm
ChecksumConfig
StreamVerifier
MultiVerifier
verify_checksum(...)
verify_multiple_checksums(...)
```

Evaluation:

- `pulith-fetch` already imports `pulith_verify::{Hasher, Sha256Hasher}` in both `fetcher.rs` and `codec/verify.rs`.
- `pulith-fetch::codec::verify` is not a separate implementation; it wraps the verify primitive and redefines a parallel checksum vocabulary.
- Several declared algorithms in `pulith-fetch::HashAlgorithm` are marked not yet implemented or only route to SHA-256 today.

Conclusion:

```text
Integrity vocabulary should move toward `pulith-verify` as canonical owner.
`pulith-fetch` should keep only fetch-specific checksum requirements/adapters if needed.
```

### Artifact Memory

Question answered:

```text
How do I preserve materialized artifacts/extracts and their provenance for reuse and explanation?
```

Artifact Memory owns local durable knowledge about materialized outputs.

It owns:

- store roots;
- store keys;
- stored artifacts;
- extracted artifacts;
- provenance records;
- metadata schema validation;
- metadata orphan/prune planning.

It must not own:

- source planning;
- transfer execution;
- archive extraction safety;
- lifecycle truth;
- install mutation.

Current owner:

```text
pulith-store
```

Current concrete objects:

```text
StoreReady
StoreRoots
StoreKey
StoredArtifact
ExtractedArtifact
StoreProvenance
StoreMetadataRecord
ArtifactRegistration / IntoArtifactRegistration
ExtractRegistration / IntoExtractRegistration
```

Current interfaces from Materialized Evidence:

```rust
impl IntoArtifactRegistration for &FetchReceipt
impl IntoExtractRegistration for (&Path, &ArchiveReport)
impl IntoExtractRegistration for (&FetchReceipt, &Path, &ArchiveReport)

StoreProvenance::from_fetch_receipt(...)
StoreProvenance::from_archive_report(...)
StoreProvenance::from_fetched_archive_extraction(...)
```

## Live API map

### `pulith-source`

Current source planning path:

```rust
ResourceLocator / RequestedResource / ResolvedResource
  -> SourceSpec::from_locator / from_requested_resource / from_resolved_resource
  -> SourceSpec::plan(SelectionStrategy)
  -> PlannedSources
  -> &[ResolvedSourceCandidate]
```

Candidate shape:

```rust
ResolvedSourceCandidate::Url(ValidUrl)
ResolvedSourceCandidate::LocalPath(PathBuf)
ResolvedSourceCandidate::Git { url, rev, subpath }
```

Planning strategy:

```rust
SelectionStrategy::OrderedFallback
SelectionStrategy::Race
SelectionStrategy::Exhaustive
```

Boundary reading:

- This is clean Source Offer vocabulary.
- It is pure and serializable where needed.
- It does not execute transfer.

### `pulith-fetch` base execution

Current base transfer path:

```rust
Fetcher::fetch_with_receipt(url, destination, FetchOptions)
  -> FetchReceipt
```

Receipt shape:

```rust
FetchReceipt {
    source: FetchSource,
    destination: PathBuf,
    bytes_downloaded: u64,
    total_bytes: Option<u64>,
    sha256_hex: Option<String>,
}
```

Fetch source receipt shape:

```rust
FetchSource::Url(String)
FetchSource::LocalPath(PathBuf)
```

Boundary reading:

- `FetchReceipt` is the correct Materialized Evidence output for transfer.
- It currently carries only SHA-256 as a named metadata field.
- It is suitable for store provenance but weaker than `VerificationReceipt` because it records digest text rather than a canonical verification receipt.

### `pulith-fetch` planned-source execution

Current integration with `pulith-source`:

```rust
MultiSourceFetcher::fetch_planned_sources_with_receipt(
    planned: &PlannedSources,
    destination: &Path,
    options: &FetchOptions,
) -> FetchReceipt
```

Also:

```rust
fetch_source_spec_with_receipt(...)
fetch_requested_resource_with_receipt(...)
fetch_resolved_resource_with_receipt(...)
```

Boundary reading:

- These methods already prove the desired Source Offer -> Transfer Execution interface exists.
- This path should become canonical.
- Direct `DownloadSource`/`MultiSourceOptions` should become legacy or internal execution options unless a distinct execution-only need is proven.

### `pulith-fetch` duplicate source vocabulary

Current duplicate-ish objects:

```rust
DownloadSource {
    url: String,
    priority: u32,
    checksum: Option<[u8; 32]>,
    source_type: SourceType,
    region: Option<String>,
}

SourceType::{Primary, Mirror, Cdn, Fallback}

MultiSourceOptions {
    sources: Vec<DownloadSource>,
    strategy: SourceSelectionStrategy,
    verify_consistency: bool,
    per_source_timeout: Option<Duration>,
}

SourceSelectionStrategy::{Priority, FastestFirst, Geographic, RaceAll}
```

Evaluation:

- `DownloadSource.url` duplicates `ResolvedSourceCandidate::Url` for HTTP candidates.
- `SourceType::{Primary, Mirror, Cdn, Fallback}` duplicates semantic source family/priority ideas that belong closer to Source Offer if they are domain meaning.
- `priority`, `region`, `per_source_timeout`, and `verify_consistency` are execution hints, but currently bundled with source declaration.
- `SourceSelectionStrategy::Priority` duplicates `SelectionStrategy::OrderedFallback`.
- `SourceSelectionStrategy::RaceAll` duplicates `SelectionStrategy::Race`.
- `FastestFirst` and `Geographic` are currently comments/fallback-to-priority behaviors in `multi_source.rs`, not implemented strategy semantics.

Conclusion:

```text
`pulith-source::PlannedSources` should be the canonical source list.
`pulith-fetch::DownloadSource` should not remain a peer source model long-term.
```

Possible target:

```rust
// Source owns semantic candidates.
pub enum ResolvedSourceCandidate { Url(ValidUrl), LocalPath(PathBuf), Git { ... } }

// Fetch owns execution knobs, not source identity.
pub struct FetchExecutionOptions {
    pub per_candidate_timeout: Option<Duration>,
    pub verify_consistency: bool,
    pub retry_policy: RetryPolicy,
}
```

Do not implement this yet. It requires caller/test migration.

### `pulith-verify`

Current canonical verification primitive:

```rust
verify_stream(reader, hasher, expected, expected_bytes) -> VerificationReceipt
```

Receipt shape:

```rust
VerificationReceipt {
    expected_digest: Vec<u8>,
    actual_digest: Vec<u8>,
    bytes_processed: u64,
}
```

Boundary reading:

- This is the right canonical owner for digest/size proof.
- It lacks an explicit algorithm field in `VerificationReceipt`; algorithm is encoded by the hasher type/caller context.
- If fetch/store provenance needs stable serialized algorithm metadata, add it deliberately to verify or translate at fetch/store boundaries.

### `pulith-store`

Current evidence absorption:

```rust
&FetchReceipt -> ArtifactRegistration { source: receipt.destination, provenance: StoreProvenance::from_fetch_receipt(receipt) }

(&Path, &ArchiveReport) -> ExtractRegistration { source_dir, provenance: from_archive_report(report) }

(&FetchReceipt, &Path, &ArchiveReport) -> ExtractRegistration { source_dir, provenance: from_fetched_archive_extraction(receipt, report) }
```

Current provenance shape:

```rust
StoreProvenance {
    origin: Option<String>,
    metadata: Metadata,
}
```

Current stored metadata from fetch/archive:

```text
fetch.sha256
archive.format
archive.entry_count
archive.total_bytes
```

Boundary reading:

- The trait absorption pattern is good: callers can pass evidence objects instead of manually converting to metadata.
- `StoreProvenance` is intentionally simple, but current string metadata may become too weak if provenance must preserve typed digest algorithm/bytes/verified status.
- Store is the correct place to persist provenance, but not to compute verification.

## Boundary decisions

### Decision 1: canonical source list is `pulith-source::PlannedSources`

`PlannedSources` should be the only semantic source-offer list crossing into fetch.

Keep:

```rust
MultiSourceFetcher::fetch_planned_sources_with_receipt(...)
```

Demote/deprecate later:

```rust
fetch_multi_source_with_receipt(Vec<DownloadSource>, MultiSourceOptions)
DownloadSource
SourceType
SourceSelectionStrategy
```

Reason:

- Source Offer belongs to `pulith-source`.
- Fetch should execute candidates, not maintain a competing source taxonomy.

### Decision 2: fetch can own execution strategy, but not source semantics

Allowed fetch execution knobs:

```text
retry policy
per-candidate timeout
race vs sequence execution selected from PlannedSources strategy
progress callback
resume/conditional/cache execution mechanics
consistency verification option if it is proven by receipts
```

Forbidden fetch source semantics:

```text
primary/mirror/cdn/fallback as source ontology
geographic source meaning unless source offers carry region metadata explicitly
source priority as independent semantic list when source planning already ordered candidates
```

### Decision 3: canonical integrity primitive is `pulith-verify`

`pulith-fetch::codec::verify` should not grow a parallel verification domain.

Target direction:

- Keep `pulith-verify` as owner of hasher/verified-reader/verification receipt.
- Let fetch call verify primitives or streaming hash primitives during transfer.
- Fetch receipt may include verification evidence by embedding or translating a canonical receipt.

Potential future shape:

```rust
pub struct FetchReceipt {
    pub source: FetchSource,
    pub destination: PathBuf,
    pub bytes_downloaded: u64,
    pub total_bytes: Option<u64>,
    pub verification: Option<TransferVerificationEvidence>,
}

pub struct TransferVerificationEvidence {
    pub algorithm: DigestAlgorithm,
    pub actual_hex: String,
    pub expected_hex: Option<String>,
    pub bytes_processed: u64,
}
```

The exact type belongs in the next implementation plan, not this report.

### Decision 4: store provenance should absorb evidence, not construct it

Store should keep trait absorption APIs:

```rust
IntoArtifactRegistration
IntoExtractRegistration
StoreProvenance::from_fetch_receipt
StoreProvenance::from_archive_report
```

But provenance fields should be assessed for typedness.

Current metadata keys are simple strings. That is acceptable for now, but if provenance becomes a durable user-facing schema, prefer typed fields over ad hoc metadata strings.

### Decision 5: `pulith-source` crate fate is blocked on this cleanup

Do not fold `pulith-source` into `pulith-resource` yet.

Reason:

- It currently owns real Source Offer objects.
- But fetch still has duplicate source vocabulary.
- Fold decision should happen after fetch stops exposing competing `DownloadSource`/`SourceSelectionStrategy` as public peer concepts.

If, after cleanup, `pulith-source` remains a small pure planner with one consumer, then it becomes a better fold candidate into `pulith-resource::source` or `pulith-resource::offer`.

## Proposed future API graph

Desired semantic path:

```rust
let planned = PlannedSources::from_requested_resource(
    &requested,
    SelectionStrategy::OrderedFallback,
)?;

let receipt = multi_fetcher
    .fetch_planned_sources_with_receipt(&planned, &destination, &fetch_options)
    .await?;

let artifact = store.register_artifact(&key, &receipt)?;
```

Archive path:

```rust
let fetch_receipt = multi_fetcher
    .fetch_planned_sources_with_receipt(&planned, &archive_path, &fetch_options)
    .await?;

let archive_report = pulith_archive::extract_to_workspace(...)?;

let extract = store.register_extract(
    &key,
    (&fetch_receipt, extracted_root.as_path(), &archive_report),
)?;
```

Key property:

```text
The caller composes steps, but does not manually reconstruct provenance glue.
```

## Delete/fold candidates after approval

These are candidates, not approved implementation steps.

### Candidate A: retire fetch `DownloadSource` public path

Target:

- migrate tests/examples/callers to `pulith-source::PlannedSources`;
- keep only `fetch_planned_sources_with_receipt` as multi-source public source path;
- delete or make private `DownloadSource`, `SourceType`, `SourceSelectionStrategy`, `MultiSourceOptions` if no remaining real caller needs them.

Risk:

- Some tests may cover only these types. They should be replaced with behavior tests over planned sources, not kept as layout guards.

### Candidate B: reduce fetch-local checksum vocabulary

Target:

- identify callers of `ChecksumConfig`, `StreamVerifier`, `MultiVerifier`, `verify_checksum`, and `verify_multiple_checksums`;
- if only tests/internal examples use them, delete or move required parse helpers to `pulith-verify`;
- keep fetch receipt evidence production tied to `pulith-verify` primitives.

Risk:

- Signature verification module may be speculative. It needs separate audit before removal because it may express future trust semantics.

### Candidate C: strengthen fetch-to-store provenance evidence

Target:

- keep `StoreProvenance` absorption traits;
- consider typed provenance fields for fetch/archive evidence instead of only string metadata if user-facing schema stability matters;
- at minimum document current string keys as schema-owned by store.

Risk:

- Over-typing provenance too early can recreate a backend/codec-style abstraction.

## Crate layout implication

After this boundary cleanup, re-evaluate:

```text
pulith-source as standalone crate vs pulith-resource::source module
pulith-verify as standalone crate vs materialization-internal module
```

Current recommendation remains:

```text
Keep `pulith-source` and `pulith-verify` until duplicate fetch vocabulary is resolved.
```

Reason:

- Folding now would hide the real overlap instead of fixing it.
- The overlap is mostly inside `pulith-fetch`, not in `pulith-source`.

## Proposed implementation plan after user approval

If this report is accepted, the next coding plan should be narrow and delete-first:

1. Search all active callers of `DownloadSource`, `SourceType`, `SourceSelectionStrategy`, and `MultiSourceOptions`.
2. Search all active callers of `ChecksumConfig`, `StreamVerifier`, `MultiVerifier`, `verify_checksum`, and `verify_multiple_checksums`.
3. If callers are only tests/examples, rewrite tests to use `PlannedSources` and `fetch_planned_sources_with_receipt`.
4. Remove public re-exports for duplicate source vocabulary.
5. Delete duplicate fetch source vocabulary if no internal caller remains.
6. Evaluate fetch-local checksum vocabulary separately; either delete, make private, or move parse-only pieces to `pulith-verify`.
7. Run focused tests:

```bash
cargo test -p pulith-source --all-features
cargo test -p pulith-fetch --all-features planned
cargo test -p pulith-store --all-features provenance
```

8. Run full gates only after the focused slice is green:

```bash
cargo fmt --all --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps
```

## Non-goals

- Do not rename crates in this slice.
- Do not fold `pulith-source` before fetch duplicate source vocabulary is resolved.
- Do not fold `pulith-verify` before fetch-local checksum/signature surfaces are audited.
- Do not introduce a facade crate.
- Do not merge fetch/archive/store into a single materialization crate.
- Do not make store execute verification or fetch execute store registration.

## Implemented duplicate cleanup slice

Status: implemented after approval.

Deleted/demoted duplicate public source vocabulary from `pulith-fetch`:

```text
DownloadSource
SourceType
SourceSelectionStrategy
MultiSourceOptions
fetch_multi_source_with_receipt(...)
Fetcher::try_source(...)
```

Code now uses this path for multi-source execution:

```text
pulith-source::SourceSpec / PlannedSources
  -> pulith-fetch::MultiSourceFetcher::fetch_planned_sources_with_receipt
  -> pulith-fetch::FetchReceipt
  -> pulith-store registration/provenance handoff
```

Updated implementation surfaces:

- `crates/pulith-fetch/src/config.rs` now only exports fetch execution options.
- `crates/pulith-fetch/src/config/sources.rs` was deleted.
- `crates/pulith-fetch/src/fetch/multi_source.rs` now accepts only `PlannedSources`/`SourceSpec`/resource-derived planned sources.
- `crates/pulith-fetch/src/fetch/batch.rs` executes URL jobs directly with `Fetcher::fetch_with_receipt` instead of constructing a duplicate source object.
- `crates/pulith-fetch/benches/multi_source.rs` now benchmarks planned source candidates.
- `docs/architecture/fetch.md` no longer lists the retired source vocabulary as active API.

## Pattern experience extracted from the code slice

The reusable pattern is:

```text
When two adjacent concepts both model the same noun, keep the noun in the upstream semantic owner and let the downstream owner accept that semantic object plus its own execution options.
```

Applied here:

- `pulith-source` owns source-offer nouns: candidate, mirror/local/git source, selection strategy.
- `pulith-fetch` owns verbs and receipts: execute candidate, place bytes, report progress, retry, return fetch receipt.
- A downstream execution crate should not invent a peer `DownloadSource` unless it represents a real execution-only object unavailable from the upstream concept.

The concrete reduction test is:

```text
Can every behavior still be expressed as PlannedSources + FetchOptions?
```

For this slice the answer was yes for active code/tests/benchmarks, so the duplicate fetch source vocabulary was removed.

## Summary recommendation

The first-principles boundary is:

```text
pulith-source owns Source Offer.
pulith-fetch owns Transfer Execution and transfer receipts.
pulith-verify owns canonical Integrity Evidence.
pulith-store owns Artifact Memory and provenance absorption.
```

After the implemented cleanup, the next duplicate section is fetch-local checksum verification vocabulary:

```text
ChecksumConfig / HashAlgorithm / StreamVerifier / MultiVerifier / verify_checksum / verify_multiple_checksums
```

That should be evaluated against `pulith-verify` in a separate slice, because signature/decompression helpers are nearby but not necessarily the same concept.

Only after the checksum slice should we decide whether `pulith-source` or `pulith-verify` still earn standalone crates.
