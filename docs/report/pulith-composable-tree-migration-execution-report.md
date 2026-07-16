# Pulith Composable Tree Migration Execution Report

## Status

The App-monolith removal plan has been executed in the active `pulith` crate.

This report covers the implementation slice:

```text
Delete App monolith.
Introduce typed Intent/WithSource leaves.
Represent behavior states as inductive typed nodes.
Move behavior requests/evidence/output to associated types.
Replace stringly hash Verify with Blake3/Sha256 ZST semantics.
Rebuild local proof through methods and typed behavior traits.
```

## Executed changes

### App monolith removed from main path

Deleted from active semantic model:

```text
App { item, sources, target, op, need, evidence }
VerifyNeed::Digest { algorithm: String, value: String }
Source enum as primary static source path
OpMode enum as primary static operation path
EvidenceEvent/EvidenceKind dynamic event bag in main proof
```

New declaration leaf:

```rust
Intent<I, T, O = CreateOrReplace>
```

Source is composed as a child node:

```rust
WithSource<I, S>
```

Current static leaves:

```rust
Item
LocalPath
LocalTarget
Create
Replace
CreateOrReplace
Forget
```

### Inductive behavior nodes introduced

New node shapes:

```rust
Offered<I, O>
Chosen<I, S>
Acquired<I, M, E>
Verified<I, M, E>
Prepared<I, P, E>
Applied<I, R, E>
Remembered<I, R, E>
Observed<I, R, E>
EvidenceChain<A, B>
NoEvidence
```

Important design shift:

```text
input is the previous typed subtree, not a universal App.
```

### Behavior traits now own associated output shape

New trait pattern:

```rust
trait VerifyNode<N> {
    type Need;
    type Evidence;
    type Error;
    type Output;

    fn verify_node(&self, node: N, need: Self::Need) -> Result<Self::Output, Self::Error>;
}
```

Analogous traits exist for:

```text
OfferNode
SelectNode
AcquireNode
AsyncAcquireNode
VerifyNode
AsyncVerifyNode
PrepareNode
AsyncPrepareNode
ApplyNode
AsyncApplyNode
RememberNode
AsyncRememberNode
```

The caller no longer constructs result fields by hand; each behavior impl chooses `Output`.

### Static typed hash Verify implemented

New typed digest surface:

```rust
DigestAlgorithm
DigestValue<A>
DigestNeed<A>
DigestEvidence<A>
HashVerify<A, R = NoHashResource>
NoHashResource
```

Feature-gated algorithms:

```rust
#[cfg(feature = "blake3")]
Blake3

#[cfg(feature = "sha2")]
Sha256
```

Static use:

```rust
HashVerify::<Blake3>::new().verify_node(acquired, DigestNeed::<Blake3>::new(expected))
HashVerify::<Sha256>::new().verify_node(acquired, DigestNeed::<Sha256>::new(expected))
```

Removed from static path:

```text
algorithm: String
normalize_algorithm
runtime algorithm match
UnsupportedDigestAlgorithm for static hash Verify
```

Remaining string value is digest bytes in hex:

```rust
DigestValue<A> { value: String, _algorithm: PhantomData<A> }
```

This is acceptable as value representation; behavior identity is no longer a string.

### Local proof rebuilt as typed tree

New local behavior implementations:

```rust
LocalAcquire
IdentityVerify
IdentityPrepare
LocalApply<Create>
LocalApply<Replace>
LocalApply<CreateOrReplace>
LocalApply<Forget>
MemoryRemember
```

Composition example in tests:

```rust
Intent::new(Item::new("demo"), LocalTarget::new(&target))
    .with_source(LocalPath::new(&source))
    .select_first()?;
```

Then:

```rust
LocalAcquire.acquire_node(chosen)?
IdentityVerify.verify_node(acquired, Identity)?
IdentityPrepare.prepare_node(verified, Identity)?
LocalApply::<CreateOrReplace>::new().apply_node(prepared)?
MemoryRemember.remember_node(applied)?
```

No `App` is passed through the graph.

## Current acceptance status

Focused test already passed after the rewrite:

```text
cargo test -p pulith --features 'sync local hash blake3 sha2'
```

Result:

```text
running 4 tests
hash::tests::sha256_verify_rejects_mismatch_before_apply ... ok
hash::tests::blake3_verify_is_typed_and_does_not_apply ... ok
local::tests::create_and_replace_are_typed_apply_laws ... ok
local::tests::local_tree_runs_create_or_replace_file ... ok
```

Full feature matrix verification is still required after this report.

## Next migration analysis

### Immediate next: run verification matrix

Required before claiming this slice fully verified:

```text
cargo fmt --all --check
cargo check -p pulith --no-default-features
cargo check -p pulith --features 'sync local'
cargo check -p pulith --features 'sync local hash blake3'
cargo check -p pulith --features 'sync local hash sha2'
cargo check -p pulith --features 'sync local hash blake3 sha2'
cargo check -p pulith --features async
cargo check --workspace --all-features
cargo test --workspace --all-features
```

### Next implementation migration: typed Prepare/archive

Recommended next migration after verification:

```text
Zip/Tar Prepare typed tree implementation
```

Why not `reqwest` next:

```text
reqwest introduces async runtime and shared network controls.
archive Prepare validates feature-gated typed nodes with less global resource-control pressure.
```

Required archive design before code:

```text
Zip / Tar ZST marker types behind features.
ArchiveNeed<A> typed by archive kind or safety policy.
ArchiveEvidence<A> typed evidence for safe extraction.
ArchivePrepare<A, R> implementation with resource annotation for temp/staging control.
```

Cargo/crate survey must be refreshed immediately before implementation:

```text
cargo search --registry crates-io --limit 5 zip
cargo info --registry crates-io zip
cargo search --registry crates-io --limit 5 tar
cargo info --registry crates-io tar
cargo search --registry crates-io --limit 5 safe_unzip
cargo info --registry crates-io safe_unzip
```

Disposition rules:

```text
Use zip/tar crates for parsing/extraction mechanism.
Do not port custom archive parser code.
Pulith owns path-safety policy, extraction evidence, and prepared tree semantics.
```

### Later: net Acquire

Do after archive or after resource-control tree types are clearer.

Typed target shape:

```rust
#[cfg(feature = "reqwest")]
pub struct ReqwestAcquire<R = SharedClient> { ... }

#[cfg(feature = "ureq")]
pub struct UreqAcquire<R = BlockingClient> { ... }
```

Source types:

```rust
LocalPath
HttpUrl
GitSource, only if feature exists later
```

Async rule remains:

```text
reqwest -> AsyncAcquireNode<HttpUrl>
ureq -> AcquireNode<HttpUrl>
```

No hidden runtime.

## Remaining cleanup debt

Current old proof debt to remove/check:

```text
Any docs still referencing App as active main path.
Any old examples that rely on Source enum / App / VerifyNeed.
Any stale `local_application_pipeline` language.
```

Current implementation still uses some simple private helper functions for mechanisms:

```text
copy_dir_all
copy_into_hasher
normalize_hex
```

These are acceptable if private and mechanism-only; do not expose them as public behavior.

## Summary

```text
The App monolith has been removed from the active main path.
Behavior is now represented as a typed inductive tree.
Hash Verify is typed with Blake3/Sha256 ZSTs instead of string algorithm dispatch.
Behavior traits now own associated Need/Evidence/Output.
Local proof composes through methods and behavior traits.
Next migration should be typed archive Prepare after full verification.
```
