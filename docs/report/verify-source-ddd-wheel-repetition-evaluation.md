# Verify / Source DDD Wheel-Repetition Evaluation

## Status

Design/evaluation report only. Do not remove crates, move code, rename public APIs, or edit Rust implementation from this report alone.

This report answers the current design question:

```text
Is `pulith-verify` a real DDD boundary, or wheel repetition that should be removed/folded?
How should `pulith-source` be judged relative to resource/fetch/install after the source-vocabulary cleanup?
What crate layout follows from the dependency and concept analysis?
```

It follows the Phase A DDD chain:

```text
Resource Intent -> Source Offer -> Materialized Evidence -> Artifact Memory -> Mutation Workflow -> Lifecycle State
```

and the cleanup rule proven by the previous source/fetch slice:

```text
When two adjacent concepts both model the same noun,
keep the noun in the upstream semantic owner and let the downstream owner consume it.
```

## Evidence read

Live files inspected:

```text
crates/pulith-verify/src/lib.rs
crates/pulith-verify/src/reader.rs
crates/pulith-verify/src/hasher.rs
crates/pulith-verify/src/error.rs
crates/pulith-verify/Cargo.toml
crates/pulith-source/src/lib.rs
crates/pulith-fetch/src/codec/verify.rs
crates/pulith-fetch/src/config/fetch_options.rs
crates/pulith-fetch/src/fetch/fetcher.rs
crates/pulith-fetch/src/error.rs
crates/pulith-fetch/src/lib.rs
crates/pulith-archive/src/options.rs
docs/architecture/verify.md
docs/architecture/source.md
docs/report/source-fetch-verify-store-first-principles-boundary.md
```

Dependency evidence from `cargo metadata --no-deps`:

```text
DEPENDENTS pulith-verify
  pulith-fetch

DEPENDENTS pulith-source
  pulith-backend-example
  pulith-fetch
  pulith-install
  runtime-manager-example
```

Source inventory:

```text
pulith-verify: files=4 loc=424
pulith-source: files=1 loc=442
pulith-fetch:  files=31 loc=9264
pulith-store:  files=1 loc=1280
pulith-resource: files=1 loc=1117
```

## DDD quality test

A crate earns a DDD boundary only if it owns a domain question that cannot be reduced to a mature library call or to a caller-local workflow step.

For each crate, ask:

1. What product question does it answer?
2. Is the answer Pulith-specific domain language?
3. Does more than one owner consume it as a semantic boundary?
4. Does it reduce dependency/capability coupling?
5. Would deleting it force duplicated domain logic elsewhere, or only direct use of mature libraries?
6. Are its public types stable contracts, or implementation mechanics?

## What is Integrity Evidence from first principles?

Before deciding `pulith-verify`, define the concept.

The product question is:

```text
After bytes are materialized, can Pulith prove they match the expected identity/integrity constraints?
```

A correct domain object would contain:

```text
IntegrityRequirement:
  expected digest algorithm and value
  optional expected byte length
  maybe future signature/trust anchor

IntegrityEvidence:
  actual digest algorithm and value
  bytes observed
  pass/fail reason
  evidence source / when checked
```

It must not decide:

```text
where bytes came from
where bytes are stored
whether an install should proceed
which source candidate to choose
how archive entries are extracted
```

Adjacent interface should be:

```text
Transfer Execution produces bytes/receipt
Integrity Verification judges bytes against requirement
Artifact Memory stores resulting evidence/provenance
```

## `pulith-verify` current state

Current public surface:

```text
Hasher
DigestHasher<D>
Sha256Hasher
Blake3Hasher
Sha3_256Hasher
VerifiedReader<R, H>
VerificationReceipt
verify_stream(...)
VerifyError
```

Current implementation shape:

- wraps `digest::Digest` behind a local `Hasher` trait;
- provides concrete type aliases/constructors over `sha2`, `sha3`, and `blake3`;
- `VerifiedReader` hashes a blocking `Read` stream while it is consumed;
- `verify_stream(...)` reads an entire blocking reader and returns a receipt;
- `VerificationReceipt` stores expected digest bytes, actual digest bytes, and bytes processed.

Current dependency shape:

```text
pulith-fetch -> pulith-verify
```

No other active crate depends on `pulith-verify`.

## `pulith-verify` wheel-repetition score

### Strong wheel-repetition signals

1. **Thin wrapper over mature libraries**

Most hashing behavior is already provided by:

```text
digest::Digest
sha2::Sha256
sha3::Sha3_256
blake3::Hasher
hex
```

`DigestHasher<D>` and `Hasher` mostly re-express the `digest` ecosystem in Pulith vocabulary.

2. **Only one active internal dependent**

`pulith-fetch` is the only active internal dependent.

That means `pulith-verify` is not currently a shared boundary across fetch/archive/store/install. It is a helper crate for fetch.

3. **Fetch already bypasses the higher-level verify API**

`pulith-fetch::fetcher` imports:

```rust
use pulith_verify::{Hasher, Sha256Hasher};
```

but performs the actual verification inline:

```text
hasher.update(chunk)
actual_checksum = hasher.finalize()
compare options.checksum
return Error::ChecksumMismatch
```

It does not use `VerifiedReader` or `verify_stream` for the primary async transfer path.

4. **Fetch has a second verification vocabulary**

`pulith-fetch::codec::verify` defines:

```text
HashAlgorithm
ChecksumConfig
StreamVerifier
MultiVerifier
verify_checksum
verify_multiple_checksums
parse_multiple_checksums
```

This duplicates and partially contradicts `pulith-verify` instead of making `pulith-verify` the canonical owner.

5. **Archive already uses mature hashing crates directly**

`pulith-archive::HashStrategy` computes SHA-256 and Blake3 directly using `sha2` and `blake3`.

So the workspace already treats hashing as owner-local workflow mechanics in at least one major materialization crate.

### Boundary-strength signals

`pulith-verify` does contain two potentially useful domain ideas:

```text
VerificationReceipt
VerifyError::{HashMismatch, SizeMismatch}
```

Those are closer to Pulith domain evidence than the hasher wrappers are.

However, they are currently too small and underused to justify a standalone crate:

- `VerificationReceipt` is not the canonical evidence stored by `pulith-store`.
- `FetchReceipt` stores `sha256_hex: Option<String>` directly.
- `StoreProvenance` absorbs fetch/archive receipts, not verification receipts.
- primary fetch verification does not return `VerificationReceipt`.

## `pulith-verify` decision

Current recommendation:

```text
`pulith-verify` is likely wheel repetition in its current form.
Do not keep it as a standalone crate unless we first promote a real Integrity Evidence contract that multiple workflows consume.
```

More directly:

```text
Remove/fold candidate: YES.
Immediate delete without replacement design: NO.
```

Why not delete immediately?

Because the concept of integrity evidence is real. The crate may be wrong, but the boundary question is not fake.

The correct move is to separate:

```text
Domain evidence worth preserving:
  IntegrityRequirement / IntegrityEvidence / mismatch reason / byte count

Wheel mechanics to delete:
  local Hasher trait
  DigestHasher wrapper
  algorithm constructors duplicating sha2/blake3/sha3
  fetch-local ChecksumConfig/StreamVerifier/MultiVerifier if they only wrap hashing
```

## Proposed `pulith-verify` outcomes

### Option A — Delete `pulith-verify`, make verification owner-local

Layout:

```text
pulith-fetch:
  uses sha2 directly for transfer checksum
  owns FetchOptions::checksum and FetchReceipt::sha256_hex

pulith-archive:
  keeps HashStrategy owner-local

pulith-store:
  stores digest/provenance fields from fetch/archive receipts
```

Pros:

- Removes one crate.
- Deletes local `Hasher`/`DigestHasher` wheel.
- Matches current actual use: fetch/archive already do owner-local hashing.
- Minimal concept overhead.

Cons:

- No canonical Integrity Evidence type.
- Hash/digest vocabulary remains duplicated in fetch and archive.
- Store provenance may stay stringly typed.

Use this if we conclude integrity is just workflow-local validation, not a reusable Pulith concept.

### Option B — Keep/fold only a small domain evidence module, delete hasher wrappers

Layout if inside fetch or a materialization owner:

```text
integrity.rs:
  IntegrityRequirement
  IntegrityEvidence
  IntegrityMismatch
  verify_sha256_bytes(...)
  verify_sha256_reader(...)
```

Implementation uses mature libraries directly:

```text
sha2::Sha256
blake3::Hasher only where caller asks for Blake3
hex encode/decode at boundary
```

Pros:

- Keeps domain evidence explicit.
- Deletes wheel wrappers.
- Gives store a typed evidence object later.
- Avoids standalone crate until multiple owners need it.

Cons:

- Requires careful API design.
- May still become abstraction if only fetch consumes it.

Use this if we want DDD concepts but not a crate per concept.

### Option C — Rebuild `pulith-verify` as real Integrity Evidence owner

Layout:

```text
pulith-verify:
  IntegrityRequirement
  IntegrityEvidence
  IntegrityAlgorithm
  verify_reader / verify_bytes
  no local generic Hasher trait unless proven necessary
```

Then fetch/archive/store must consume it:

```text
fetch returns FetchReceipt { integrity: Option<IntegrityEvidence>, ... }
archive returns entry/report integrity evidence using same type
store persists typed evidence
```

Pros:

- Real DDD boundary.
- Shared evidence vocabulary.
- Better provenance consistency.

Cons:

- Larger migration.
- Could still be overkill if only fetch uses it.
- Must not recreate generic crypto framework.

Use this only if at least two materialization workflows need the same durable evidence contract.

## Recommendation for `pulith-verify`

Recommended next implementation direction, after this report is accepted:

```text
Choose Option A or B, not Option C yet.
```

More specific recommendation:

```text
Fold/remove `pulith-verify` as a standalone crate.
Keep SHA-256 transfer verification owner-local in `pulith-fetch` for now.
Delete `pulith-fetch::codec::verify` public checksum vocabulary.
Preserve only the evidence facts already needed by `FetchReceipt` and StoreProvenance.
```

Then evaluate whether an `IntegrityEvidence` type is needed by both fetch and archive. If yes, add it as a small module in the owning materialization crate or a later dedicated crate. Do not keep the current `Hasher` wrapper crate as the answer.

## Proposed first code slice if approved

Slice name:

```text
fetch-verify-wheel-cleanup
```

Steps:

1. Remove public re-exports from `pulith-fetch`:

```text
ChecksumConfig
StreamVerifier
MultiVerifier
verify_checksum
```

2. Delete or make private `crates/pulith-fetch/src/codec/verify.rs` if active callers are only its own tests.

3. Replace `pulith_verify::{Hasher, Sha256Hasher}` in fetch with direct `sha2::Sha256` and `sha2::Digest`.

4. Remove `pulith-verify` dependency from `pulith-fetch/Cargo.toml`; add direct `sha2` if not already present.

5. If no other active dependent remains, delete `crates/pulith-verify` from workspace.

6. Update active docs/readmes/publish docs to mark `pulith-verify` folded/removed as wheel repetition.

7. Keep historical `docs/report/` references as evidence; active docs should not advertise it as current crate.

Focused verification:

```bash
cargo test -p pulith-fetch --all-features codec::verify
cargo test -p pulith-fetch --all-features checksum
cargo test -p pulith-archive --all-features hash
cargo test -p pulith-store --all-features provenance
cargo metadata --no-deps --format-version 1
```

Full gates after focused green:

```bash
cargo fmt --all --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps
```

## `pulith-source` current state

Current public surface:

```text
HttpAssetSource
MirrorSource
LocalSource
GitSource
RemoteSource
SourceDefinition
SourceSet
SourceSpec
PlannedSources
ResolvedSourceCandidate
SelectionStrategy
SourceAdapter
PassthroughAdapter
```

Current dependency shape:

```text
pulith-source -> pulith-resource
pulith-fetch -> pulith-source
pulith-install -> pulith-source
examples -> pulith-source
```

After the previous cleanup, `pulith-fetch` no longer exposes a competing source-offer model. It consumes `PlannedSources` / `ResolvedSourceCandidate`.

## What is Source Offer from first principles?

The product question is:

```text
Given a resource intent, what acceptable origins may produce its material bytes, and in what candidate order/strategy should they be tried?
```

A correct domain object contains:

```text
SourceDefinition:
  semantic source family: HTTP asset, mirror set, local path, git reference

SourceSet:
  non-empty declared sources

PlannedSources:
  resolved candidate list plus selection strategy

ResolvedSourceCandidate:
  concrete executable source candidate for fetch/install
```

It must not decide:

```text
HTTP retry/backoff
cache/resume behavior
checksum verification
store location
installation target
manager-specific package policy
```

Adjacent interface:

```text
Resource Intent -> SourceSpec / PlannedSources -> Transfer Execution
```

## `pulith-source` wheel-repetition score

### Signals that it is not just a wheel

1. It uses Pulith-specific domain nouns.

`SourceSpec`, `PlannedSources`, and `ResolvedSourceCandidate` are not mature library wrappers. They encode the boundary between Resource Intent and Transfer Execution.

2. It has multiple active consumers.

Dependents include fetch, install, and examples.

3. It has already absorbed a duplicated peer model.

The previous slice deleted fetch-side `DownloadSource`/`SourceSelectionStrategy` and made `pulith-source` canonical.

4. It prevents fetch from becoming policy owner.

Without `pulith-source`, `pulith-fetch` would likely re-grow source planning and candidate semantics.

### Suspicious / needs quality improvement

1. It may still be small enough to live inside `pulith-resource`.

`pulith-source` is only one file and depends directly on `pulith-resource`.

A possible layout is:

```text
pulith-resource::source
```

if Source Offer never becomes reusable outside Resource Intent.

2. `SourceAdapter` may be premature framework vocabulary.

Current `SourceAdapter` + `PassthroughAdapter` are small and not yet clearly proven by multiple real backend adapters.

They should be watched as possible abstraction creep.

3. `SelectionStrategy` has policy flavor.

`OrderedFallback`, `Race`, and `Exhaustive` are candidate execution strategies. They are currently attached to source planning because they decide candidate order/semantics before fetch execution.

This is acceptable after the fetch cleanup, but it should stay minimal.

## `pulith-source` decision

Current recommendation:

```text
Keep `pulith-source` for now.
```

But with a review condition:

```text
After verify cleanup, evaluate whether `pulith-source` should remain a crate or become `pulith-resource::source`.
```

Why keep now?

- It owns a real DDD concept: Source Offer.
- It has active consumers outside tests.
- The previous cleanup made it more canonical, not less.
- Folding it now would mix resource identity with source-planning mechanics before the verify/materialization boundary is stable.

Why not commit to keeping forever?

- It is small.
- It has a one-way dependency on `pulith-resource`.
- If the project wants fewer crates, `pulith-resource + source offer` is a plausible bounded context.

## Dependency-informed new layout proposal

### Near-term after `pulith-verify` cleanup

If `pulith-verify` is removed:

```text
Resource Intent / Source Offer
  pulith-version
  pulith-resource
  pulith-source          # keep for one more phase

Materialized Evidence
  pulith-fs
  pulith-archive
  pulith-fetch           # owns transfer + direct SHA-256 checksum mechanics

Artifact Memory
  pulith-store

Lifecycle State / Inspection
  pulith-state

Mutation Workflow
  pulith-install
```

### Possible next reduction after source review

If `pulith-source` stays small and remains tightly bound to resource intent:

```text
pulith-resource
  version intent?       # only if pulith-version later fails standalone value test
  resource identity
  source offer module

pulith-fs
pulith-archive
pulith-fetch
pulith-store
pulith-state
pulith-install
```

This would reduce one more crate while preserving DDD concepts as modules rather than crates.

### Do not collapse these yet

```text
pulith-fs
pulith-archive
pulith-fetch
pulith-store
pulith-state
pulith-install
```

Reason:

- `pulith-fs` owns filesystem safety primitives.
- `pulith-archive` owns archive entry safety and extraction semantics.
- `pulith-fetch` owns network/local transfer execution.
- `pulith-store` owns artifact memory/provenance.
- `pulith-state` owns lifecycle truth and inspection/repair views.
- `pulith-install` owns mutation workflow and rollback/activation.

These are not just wheels based on current evidence.

## DDD design quality judgment

### Good current design decisions

- Source/fetch split is now healthier after duplicate source vocabulary removal.
- Store absorbs receipts rather than executing fetch/archive itself.
- State owns lifecycle and lock/export views.
- Install composes resource/source/fetch/store/state without owning their internal evidence models.

### Weak current design decisions

- `pulith-verify` is a crate-shaped wrapper around existing hash crates, with too little project-specific domain contract.
- `pulith-fetch::codec::verify` duplicates verification vocabulary and should not stay public.
- `FetchOptions::checksum: Option<[u8; 32]>` is simple and useful, but names only SHA-256 and does not express broader integrity requirement.
- `FetchReceipt::sha256_hex: Option<String>` is pragmatic but not a typed evidence object.
- `pulith-archive::HashStrategy` separately implements hashing; this proves `pulith-verify` is not canonical.

## Recommended next design-to-code path

1. Accept this report or revise the decision.
2. If accepted, implement the `fetch-verify-wheel-cleanup` slice:
   - remove `pulith-fetch::codec::verify` public vocabulary;
   - replace `pulith_verify` hasher wrappers with direct `sha2` in fetch;
   - remove/delete `pulith-verify` if no active dependent remains.
3. Document pattern experience:

```text
If a crate only wraps mature algorithm crates and has one execution-crate consumer,
it is not a bounded context. Keep the evidence facts in the workflow receipt,
and use mature libraries directly.
```

4. Then re-evaluate `pulith-source`:
   - keep as crate if multiple real manager/source adapters emerge;
   - fold into `pulith-resource::source` if it remains a thin one-file extension of Resource Intent.

## Stop conditions

Do not remove `pulith-verify` if implementation audit finds:

- active non-fetch consumers using `VerificationReceipt` as a durable product API;
- store/state/install depending on `pulith-verify` as canonical evidence;
- public examples that cannot be expressed by direct `sha2` plus owner-local receipt fields;
- a near-term accepted design that upgrades `pulith-verify` into a true `IntegrityEvidence` owner consumed by fetch and archive.

Do not fold `pulith-source` until after verify cleanup and a separate source/resource evaluation.

## Implemented slice: delete `pulith-verify`

Status: implemented after user approval.

Deleted from the active workspace:

```text
crates/pulith-verify/
docs/architecture/verify.md
crates/pulith-fetch/src/codec/verify.rs
```

Removed active dependency edge:

```text
pulith-fetch -> pulith-verify
```

Fetch now uses owner-local SHA-256 mechanics directly:

```text
sha2::Sha256
sha2::Digest
FetchOptions::checksum: Option<[u8; 32]>
FetchReceipt::sha256_hex: Option<String>
```

Removed duplicate public fetch checksum vocabulary:

```text
ChecksumConfig
HashAlgorithm
StreamVerifier
MultiVerifier
verify_checksum
verify_multiple_checksums
```

Pattern experience extracted:

```text
If a crate only wraps mature algorithm crates and has one execution-crate consumer,
it is not a bounded context. Keep the evidence fact in the workflow receipt,
and use the mature library directly in the workflow owner.
```

## Optimized plan: archive/fetch continuity

The user's correction is important: download and extraction are often adjacent in product workflows, so the architecture must not split subordinate behavior just to make crate boundaries look symmetrical.

Revised boundary principle:

```text
Separate crates only when they own independently reusable safety contracts.
Do not separate merely sequential actions when the product operation is naturally continuous.
```

Current judgment after deleting `pulith-verify`:

```text
pulith-fetch:
  owns transfer execution, transfer checksum checks, retry/progress/range/cache mechanics,
  and FetchReceipt.

pulith-archive:
  owns archive format detection, path/symlink/zip-slip safety, extraction limits,
  entry hashing when requested, and ArchiveReport.
```

These two crates may stay separate for now because they protect different safety contracts:

```text
fetch safety: remote/local byte materialization and atomic placement
archive safety: untrusted tree expansion, path traversal, symlink escape, extraction limits
```

But the workflow boundary should be continuous at the higher layer:

```text
planned source -> fetch bytes -> optional extract -> store/register -> install
```

Next design artifact should therefore evaluate a **materialization workflow surface**, not another tiny helper crate:

```text
MaterializeArchive / MaterializedArtifact / MaterializedTree
```

Possible owner candidates:

1. `pulith-store` absorbs fetch/archive receipts into artifact memory, but should not execute network/extraction.
2. `pulith-install` composes fetch/archive/store when mutation needs a ready tree/file.
3. A small module inside `pulith-fetch` or `pulith-archive` is only acceptable if it deletes real caller glue and does not create a facade crate.
4. A new crate is explicitly rejected unless multiple independent callers require a durable materialization API.

Immediate next slice recommendation:

```text
Do not fold archive into fetch yet.
First inspect actual fetch->extract call paths and design one continuous materialization API at the existing workflow owner.
```

Acceptance test for any next design:

```text
Can a caller express "download this archive and extract/register it" without manual path/provenance glue,
while fetch and archive still keep their safety contracts independently testable?
```

## Final recommendation

```text
`pulith-verify` is removed from the active layout.
Keep `pulith-source` for now.
Next: evaluate fetch/archive/store/install call paths for a continuous materialization workflow surface.
```
