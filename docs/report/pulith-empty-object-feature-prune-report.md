# Pulith Empty Object Feature Prune Report

## Status

Completed implementation; final verification recorded below.

Plan:

```text
.hermes/plans/2026-07-15_215410-prune-empty-object-feature.md
```

## Decision

Pulith removed the advertised but behaviorless `object` feature and optional `object_store` dependency. Reintroduction now requires a real typed object-source contract rather than a reserved manifest name.

## Baseline evidence

Before deletion, the supported Cargo command succeeded:

```text
cargo check -p pulith --no-default-features --features object
```

It compiled `object_store 0.14.0` and Pulith, but repository searches found zero active Rust ownership:

```text
object_store imports: 0
cfg(feature = "object"): 0
ObjectSource: 0
ObjectResource: 0
ObjectAcquire: 0
```

CI/script/config searches also found no `--features object` caller.

The pre-delete feature therefore changed dependency resolution only:

```text
object -> net + async + dep:object_store
```

It did not add a source, resource, morphism, error, evidence, or behavior test.

## Manifest deletion

Removed from workspace `Cargo.toml`:

```toml
object_store = "0.14"
```

Removed from `crates/pulith/Cargo.toml`:

```toml
object = ["net", "async", "dep:object_store"]
object_store = { workspace = true, optional = true }
```

No compatibility shell was retained:

```toml
# deliberately absent
object = []
```

## Cargo.lock result

Cargo updated the lockfile through a normal workspace all-features check; it was not hand-edited.

A controlled temporary baseline copied the current lockfile, restored exactly `object_store = "=0.14.0"` and the deleted feature, then resolved that feature. Comparing this baseline with the post-delete lockfile attributes the following exact package delta to the removed object graph:

```text
android_system_properties 0.1.5
async-trait 0.1.89
autocfg 1.5.1
chrono 0.4.45
either 1.16.0
futures-macro 0.3.32
humantime 2.4.0
iana-time-zone 0.1.65
iana-time-zone-haiku 0.1.2
itertools 0.15.0
nix 0.31.3
num-traits 0.2.19
object_store 0.14.0
tokio-macros 2.7.0
tracing-attributes 0.1.31
windows-core 0.62.2
windows-implement 0.60.2
windows-interface 0.59.3
windows-result 0.4.1
windows-strings 0.5.1
```

No package existed only in the post-delete lockfile during that controlled comparison.

Post-delete `cargo metadata --no-deps` confirms:

```text
has object feature: false
has object_store dependency: false
```

The post-delete lockfile has no `name = "object_store"` package.

## SemVer note

The Cargo Book classifies removal of a feature or optional dependency as usually SemVer-incompatible. A crates.io API lookup found no published `pulith` package, and repository searches found no active caller. The current workspace is therefore the observed migration boundary.

If Pulith is distributed through a private registry or consumed by an external repository, this deletion must be communicated as an intentional breaking feature-surface change.

## Why implementation was not substituted for deletion

`object_store` is a substantial async storage API with local/cloud stores, runtime configuration, conditional fetch/write, range reads, listing, vectored I/O, and multipart operations. Pulith has not yet defined the corresponding owned semantics:

```text
store identity and construction
object path identity
provider credential/config ownership
version/generation validator
range/resume continuity
provider error and retry classification
staging and destination safety
evidence shape
```

Wrapping `ObjectStore::get` without those decisions would add mechanism while leaving the behavior contract undefined.

## Reintroduction gate

Restore object-store support only when all of the following exist:

1. A real caller and provider/store use case.
2. A typed `ObjectSource` identity, not provider/path strings in a generic map.
3. Explicit resource ownership for store handles and resolved credentials.
4. Validator, range, resume, retry, staging, max-byte, admission, and pacing laws.
5. Narrow `object_store` features selected with `default-features = false` unless the default filesystem backend is explicitly required.
6. RED behavior tests preceding production implementation.

Only then should a new feature and dependency be added.

## Verification

Fresh focused ad-hoc script:

```text
F:\Stratum\TEMP\hermes-verify-e081x_iz.py
```

Cleanup:

```text
AD_HOC_SCRIPT_CLEANED=F:\Stratum\TEMP\hermes-verify-e081x_iz.py
```

Structural marker:

```text
STRUCTURAL_ASSERTIONS_PASS object feature/dependency/lock absent
```

Final marker:

```text
AD_HOC_VERIFY_PASS pulith empty object feature pruned
```

Commands covered:

```text
cargo fmt --all --check
cargo check -p pulith --no-default-features
cargo check -p pulith --features net
cargo check -p pulith --features "sync local net ureq hash blake3"
cargo check -p pulith --features "async net reqwest hash blake3"
cargo test -p pulith --features "sync local net ureq hash blake3" admission
cargo test -p pulith --features "async net reqwest hash blake3" admission
cargo test -p pulith --features "sync local net ureq hash blake3" byte_rate_pacer
cargo test -p pulith --features "async net reqwest hash blake3" byte_rate_pacer
cargo test --workspace --all-features
git diff --check -- Cargo.toml Cargo.lock crates/pulith/Cargo.toml docs/report/pulith-empty-object-feature-prune-report.md
```

Observed:

```text
sync admission: 6 passed
async admission: 5 passed
sync byte-rate pacer: 4 passed
async byte-rate pacer: 4 passed
workspace all-features: 90 passed
manifest/metadata/lockfile absence: passed
fmt/check/diff-check: passed
```

The `net`-only build still reports existing backend-less dead-code warnings; supported sync/ureq, async/reqwest, and all-features paths introduced no new warning or failure. This is focused ad-hoc verification, not an external canonical suite claim.
