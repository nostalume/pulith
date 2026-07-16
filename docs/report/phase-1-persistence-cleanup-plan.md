# Phase 1 Persistence Cleanup Plan

## Status

Plan only. Do not edit Rust code from this report alone.

This is the next reduction phase after `pulith-shim` was folded into `pulith-install`.

## Goal

Delete the dead serialization backend abstraction by folding its tiny JSON helper surface into the crates that actually own durable schemas.

Target outcome:

```text
pulith-serde-backend crate disappears.
state/store/install/lock use direct schema-versioned JSON at their own boundaries.
No public TextCodec / JsonTextCodec / CompactJsonTextCodec abstraction remains.
```

## Why this phase next

The Phase 0 feedback queue was:

1. `pulith-install` activation/shim reduction — done.
2. Persistence cleanup — delete/fold `pulith-serde-backend`, choose direct schema-versioned JSON.
3. State lock/export cleanup — demote `pulith-lock` into state export or delete if not needed.
4. Platform cleanup — freeze/non-publish/remove `pulith-platform` unless active Pulith-specific semantics appear.

`pulith-serde-backend` should go before `pulith-lock` because `pulith-lock`, `pulith-state`, `pulith-store`, and `pulith-install` currently depend on the backend helper. Removing it first clarifies whether `pulith-lock` is a product format or just a state export shape.

## Current evidence

Current source-space audit at plan time:

```text
pulith-serde-backend     files=1 loc=143
pulith-lock              files=1 loc=308
pulith-state             files=1 loc=2590
pulith-store             files=1 loc=1286
pulith-install           files=1 loc=2500
```

Phase 0 conclusions already recorded:

- `TextCodec` is not earned by real backend polymorphism.
- `JsonTextCodec` vs `CompactJsonTextCodec` is formatting, not architecture.
- Existing crates (`serde_json`, `postcard`, `ciborium`) already own generic serialization formats.
- Pulith needs owner-local schema validation, deterministic JSON where diffs matter, and typed decode errors.

## Scope

In scope:

- Read and classify every active use of `pulith_serde_backend`.
- Replace backend helper usage with owner-local JSON functions or direct `serde_json` calls.
- Remove `pulith-serde-backend` from workspace membership and dependency manifests.
- Delete `crates/pulith-serde-backend/`.
- Update active docs/readmes/publish notes.
- Verify all persistence behavior still passes.

Out of scope:

- Do not fold `pulith-lock` yet.
- Do not redesign state/store schemas beyond removing the backend abstraction.
- Do not introduce Postcard/CBOR/TOML.
- Do not split `pulith-state`/`pulith-install` monoliths in this phase.
- Do not create a new `pulith-persistence` crate.

## Target architecture

### Principle

Persistence format is an owner boundary, not a public backend plugin boundary.

Each durable owner should expose typed load/save behavior through its own domain API:

```text
pulith-state: StateSnapshot JSON with schema version validation
pulith-store: Store metadata JSON with schema version validation
pulith-lock: LockFile JSON while it still exists
pulith-install: backup/restore payload JSON where install owns the receipt/file
```

The shared implementation should be either:

1. direct `serde_json::{to_vec_pretty, to_vec, from_slice}` at the local call site, or
2. a tiny private module inside the owning crate if the owner repeats the same encode/decode/error wrapping more than once.

No crate should export generic `TextCodec` vocabulary.

### Error ownership

Each owner maps JSON failures into its existing crate-local error enum:

```text
pulith-state::StateError
pulith-store::StoreError
pulith-lock::LockError
pulith-install::InstallError
```

Do not replace these with a new shared persistence error crate.

### Formatting rule

Use pretty JSON where files are user-facing or diff-facing.
Use compact JSON only if an existing test or storage contract requires compact output.
If no compact contract is proven, prefer pretty JSON for durable files.

## Implementation plan after approval

### Step 1 — Use audit

Run searches:

```bash
grep -R "pulith_serde_backend\|JsonTextCodec\|CompactJsonTextCodec\|TextCodec\|encode_pretty_vec\|encode_vec\|decode_slice" -n crates examples Cargo.toml Cargo.lock
```

Record every active usage and classify it by owner crate.

Expected likely owners:

- `pulith-state`
- `pulith-store`
- `pulith-lock`
- `pulith-install`

### Step 2 — Read owner error and persistence code

Read the relevant sections before editing:

- `crates/pulith-serde-backend/src/lib.rs`
- `crates/pulith-state/src/lib.rs`
- `crates/pulith-store/src/lib.rs`
- `crates/pulith-lock/src/lib.rs`
- `crates/pulith-install/src/lib.rs`
- all four `Cargo.toml` files

For each owner, identify:

- current `CodecError` conversion path;
- whether writes are pretty or compact;
- schema version checks;
- tests that assert JSON determinism or roundtrip behavior.

### Step 3 — Replace `pulith-lock` usage first

Reason: `pulith-lock` is small and its serialization behavior is explicit/deterministic.

Likely edit:

- remove `pulith-serde-backend` dependency from `crates/pulith-lock/Cargo.toml`;
- replace helper calls with direct `serde_json`;
- map serde errors into `LockError` directly;
- preserve deterministic `BTreeMap` behavior.

Focused verification:

```bash
cargo test -p pulith-lock --all-features
```

### Step 4 — Replace `pulith-store` usage

Likely edit:

- remove backend dependency from `crates/pulith-store/Cargo.toml`;
- localize JSON read/write helper inside `pulith-store` only if repeated;
- keep schema validation in store-owned code;
- preserve provenance/metadata roundtrip tests.

Focused verification:

```bash
cargo test -p pulith-store --all-features
```

### Step 5 — Replace `pulith-state` usage

Likely edit:

- remove backend dependency from `crates/pulith-state/Cargo.toml`;
- localize JSON snapshot helpers if needed;
- keep unsupported schema-version errors owner-local;
- keep lock export behavior unchanged for now.

Focused verification:

```bash
cargo test -p pulith-state --all-features
```

### Step 6 — Replace `pulith-install` usage

Likely edit:

- remove backend dependency from `crates/pulith-install/Cargo.toml`;
- replace backup/restore payload encode/decode with direct JSON;
- keep install receipt/lifecycle behavior unchanged.

Focused verification:

```bash
cargo test -p pulith-install --all-features
```

### Step 7 — Delete retired crate

After all importers pass focused tests:

```bash
git rm -r crates/pulith-serde-backend
```

Then remove workspace/dependency references from:

- root `Cargo.toml`
- `Cargo.lock`
- README crate lists
- crate metadata/readmes if they list or link the backend crate
- `docs/architecture.md`
- `docs/architecture/serialization.md` or delete/retire that doc if it only describes the removed crate
- `docs/AGENT.md`
- `docs/publish/*`

Historical `docs/report/` mentions can remain as evidence.

### Step 8 — Active-surface absence check

Run a layout/reference check over active code and manifests:

```bash
python - <<'PY'
from pathlib import Path
root = Path('.')
needles = [
    'pulith-serde-backend',
    'pulith_serde_backend',
    'TextCodec',
    'JsonTextCodec',
    'CompactJsonTextCodec',
    'CodecError',
]
fail = []
for p in list((root/'crates').rglob('*')) + list((root/'examples').rglob('*')) + [root/'Cargo.toml', root/'Cargo.lock']:
    if p.is_file() and p.suffix.lower() in {'.rs', '.toml', '.lock'}:
        text = p.read_text(encoding='utf-8', errors='ignore')
        for needle in needles:
            if needle in text:
                fail.append((str(p), needle))
if fail:
    for path, needle in fail:
        print(f'{path}: {needle}')
    raise SystemExit(1)
print('no active code/manifest/lock serde-backend references')
PY
```

### Step 9 — Workspace verification

Run canonical gates:

```bash
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps
```

## Risk controls

- Do not change schema names, field names, or version numbers unless a test requires it.
- Do not make lock/state/store share a new public persistence module; that would recreate the deleted crate.
- Do not delete `pulith-lock` in this phase, even if it becomes obviously foldable. Record that as the next phase.
- Do not leave compatibility re-exports or aliases for the deleted backend crate.
- Preserve deterministic JSON behavior in lock/store tests.

## Expected final report format

After implementation, report:

- deleted crate and files;
- dependency edges removed;
- owner-local JSON behavior preserved;
- active-surface absence check result;
- exact verification commands and pass/fail outputs;
- next reduction recommendation, likely `pulith-lock` demotion into `pulith-state` export.

## Stop condition

Stop and ask before coding if the use audit shows any real caller-selected backend behavior or non-JSON backend implementation. That would invalidate the premise of deleting `pulith-serde-backend` as a dead abstraction.
