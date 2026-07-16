# Phase 2 Lock State Export Reduction Plan

## Status

Plan only. Do not edit Rust code from this report alone.

This is the next reduction phase after:

1. `pulith-shim` was folded into `pulith-install`.
2. `pulith-serde-backend` was folded into owner-local JSON boundaries.

## Goal

Demote `pulith-lock` from an independent workspace crate into a `pulith-state` owned export/diff module, unless implementation audit discovers real standalone product usage.

Expected target:

```text
pulith-state owns lock export/report types.
pulith-lock crate disappears.
No pulith_lock import path remains in active code.
No compatibility crate, alias crate, or re-export shim remains.
```

## Why this phase next

After the persistence cleanup, `pulith-lock` no longer depends on the dead backend abstraction. Its active shape is now clear:

- one source file, about 300 LOC;
- deterministic `BTreeMap`-backed data model;
- direct `serde_json` helpers;
- only active internal consumer found through metadata/search: `pulith-state`;
- `pulith-state::export_lock_file()` creates the `LockFile` from state records.

The current evidence says lock is a state export/report shape, not a standalone solver or package-manager lock product.

## Current evidence

Active metadata/dependency evidence at plan time:

```text
pulith-state deps = pulith-fs,pulith-lock,pulith-resource,pulith-store,serde,serde_json,thiserror,criterion,tempfile
pulith-lock  deps = serde,serde_json,thiserror,criterion
```

Active source evidence:

```text
crates/pulith-state/src/lib.rs imports:
  use pulith_lock::{LockFile, LockedResource};

crates/pulith-state/src/lib.rs exposes:
  pub fn export_lock_file(&self) -> Result<LockFile>
```

`crates/pulith-lock/` contents:

```text
README.md
Cargo.toml
src/lib.rs
benches/lock_diff.rs
```

`pulith-lock` owns only:

- `LOCK_SCHEMA_VERSION`
- `Metadata = BTreeMap<String, String>`
- `LockError`
- `LockedResource`
- `LockFile`
- `LockResourceChange`
- `LockDiff`
- JSON encode/decode helpers
- deterministic diff tests
- one diff benchmark

## Product-intent decision

Decision for this phase:

```text
Treat pulith-lock as state-owned export/report functionality.
```

Reason:

- It does not solve dependencies.
- It does not own source/version resolution.
- It does not explain why a version/source/digest was selected beyond copied state fields.
- It is produced from lifecycle state today.
- Existing solver/lockfile crates already cover package-manager lockfile concerns if Pulith later needs them.

## Scope

In scope:

- Move `pulith-lock` types and tests into `pulith-state` as owner-local lock export code.
- Remove `pulith-lock` dependency from `pulith-state`.
- Delete `crates/pulith-lock/` after importers pass.
- Update active docs/readmes/publish notes.
- Move or retire the `lock_diff` benchmark depending on whether the benchmark still earns its keep under `pulith-state`.
- Verify all lock export/diff behavior remains passing.

Out of scope:

- Do not add dependency solving.
- Do not add external ecosystem lock parsing.
- Do not redesign resource/source/version semantics.
- Do not split the whole `pulith-state` monolith in this phase beyond a minimal state-owned lock module if needed.
- Do not change lock JSON schema field names or version unless a test forces it.
- Do not keep `pulith-lock` as a compatibility shim crate.

## Target architecture

### Module shape

Preferred owner-local layout:

```text
crates/pulith-state/src/lib.rs
crates/pulith-state/src/lock.rs
```

`lock.rs` owns:

```rust
pub const LOCK_SCHEMA_VERSION: u32 = 1;
pub type LockMetadata = BTreeMap<String, String>;
pub enum LockError { ... }
pub struct LockedResource { ... }
pub struct LockFile { ... }
pub struct LockResourceChange { ... }
pub struct LockDiff { ... }
```

`lib.rs` should expose these through the normal `pulith-state` public API, not through a deprecated old crate path:

```rust
pub mod lock;
pub use lock::{LockDiff, LockError, LockFile, LockResourceChange, LockedResource};
```

This is not a compatibility shim. It is `pulith-state` declaring that lock export is part of state API.

### Error shape

Two acceptable options:

1. Keep a state-owned `LockError` and preserve precise lock validation errors.
2. Fold lock validation errors into `StateError` only if that deletes code and does not make tests less precise.

Preferred first implementation: keep `pulith_state::LockError` inside `lock.rs`, because it isolates lock JSON/schema validation without preserving the old crate.

Then update `StateError`:

```rust
Lock(#[from] LockError)
```

or inline where cleaner.

### Data model

Keep the existing JSON/data shape unchanged:

```rust
LockFile {
    schema_version: u32,
    resources: BTreeMap<String, LockedResource>,
    metadata: BTreeMap<String, String>,
}

LockedResource {
    version: String,
    source: String,
    digest: Option<String>,
    metadata: BTreeMap<String, String>,
}
```

Do not add new fields. Do not rename `LockedResource` just to make the module feel new.

### Benchmark handling

Move benchmark from:

```text
crates/pulith-lock/benches/lock_diff.rs
```

to:

```text
crates/pulith-state/benches/lock_diff.rs
```

Only keep it if it compiles cleanly and still measures `LockFile::diff` directly. Otherwise retire it and note that state has enough broader benchmarks for now.

## Implementation plan after approval

### Step 1 — Read exact current files

Read before editing:

- `crates/pulith-lock/src/lib.rs`
- `crates/pulith-lock/Cargo.toml`
- `crates/pulith-lock/README.md`
- `crates/pulith-lock/benches/lock_diff.rs`
- `crates/pulith-state/src/lib.rs`
- `crates/pulith-state/Cargo.toml`
- `crates/pulith-state/README.md`
- `docs/architecture/lock.md`
- `docs/architecture/state.md`

### Step 2 — Create `pulith-state::lock`

Create:

```text
crates/pulith-state/src/lock.rs
```

Move lock model/diff/JSON code from `crates/pulith-lock/src/lib.rs` into it.

Adjust crate-local imports:

```rust
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;
```

Keep unit tests with the moved module if possible.

### Step 3 — Wire module into state

Modify:

```text
crates/pulith-state/src/lib.rs
```

Replace external import:

```rust
use pulith_lock::{LockFile, LockedResource};
```

with state-owned imports:

```rust
pub mod lock;
pub use lock::{LockDiff, LockError, LockFile, LockResourceChange, LockedResource};
```

Then adjust internal references if needed.

Focused verification:

```bash
cargo test -p pulith-state --all-features state_can_export_lock_file_from_resolved_records
cargo test -p pulith-state --all-features lock_
```

### Step 4 — Remove dependency edge

Modify:

```text
crates/pulith-state/Cargo.toml
```

Delete:

```toml
pulith-lock = { path = "../pulith-lock", version = "0.1.0" }
```

Focused verification:

```bash
cargo test -p pulith-state --all-features
```

### Step 5 — Migrate or retire lock benchmark

Read `crates/pulith-lock/benches/lock_diff.rs` and choose:

- if it directly benchmarks `LockFile::diff`, move it to `crates/pulith-state/benches/lock_diff.rs` and add a `[[bench]]` entry in `crates/pulith-state/Cargo.toml`;
- if it mostly duplicates unit-test scale behavior and adds maintenance surface, retire it with the crate deletion.

Preferred: move it, because there is already a benchmark note under `docs/benchmarks/block-t-2026-04.md` referencing lock diff scale behavior.

Verification if moved:

```bash
cargo bench -p pulith-state --bench lock_diff --no-run
```

### Step 6 — Delete retired crate

After state tests pass:

```bash
git rm -r crates/pulith-lock
```

Then update:

- root workspace lockfile through Cargo commands;
- `README.md` crate lists;
- `docs/architecture.md` crate roles;
- `docs/architecture/lock.md` or merge its active content into `docs/architecture/state.md` and delete it;
- `docs/AGENT.md` crate role list;
- `docs/publish/*` historical notes;
- `docs/benchmarks/block-t-2026-04.md` benchmark command;
- any crate README that links `pulith-lock` as a standalone crate.

Historical `docs/report/` mentions can remain as evidence.

### Step 7 — Active-surface absence check

Run:

```bash
python - <<'PY'
from pathlib import Path
root = Path('.')
needles = [
    'pulith-lock',
    'pulith_lock',
]
fail = []
for p in list((root/'crates').rglob('*')) + list((root/'examples').rglob('*')) + [root/'Cargo.toml', root/'Cargo.lock']:
    if p.is_file() and p.suffix.lower() in {'.rs', '.toml', '.lock', '.md'}:
        text = p.read_text(encoding='utf-8', errors='ignore')
        for needle in needles:
            if needle in text:
                fail.append((str(p), needle))
if fail:
    for path, needle in fail:
        print(f'{path}: {needle}')
    raise SystemExit(1)
print('no active crates/examples/manifest/lock pulith-lock references')
PY
```

Also verify metadata:

```bash
python - <<'PY'
import json, subprocess
p = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], text=True))
print('pulith-lock in metadata:', 'pulith-lock' in [pkg['name'] for pkg in p['packages']])
PY
```

Expected:

```text
pulith-lock in metadata: False
```

### Step 8 — Canonical verification

Run:

```bash
cargo fmt --all --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps
```

If Hermes stale-verification guard triggers afterward, run a focused temp `F:/Stratum/TEMP/hermes-verify-*.py` script that proves:

- `crates/pulith-lock` is absent;
- no active crates/examples/manifests import `pulith_lock`;
- `pulith-state` exposes lock export types;
- `cargo metadata` excludes `pulith-lock`.

## Expected final report format

After implementation, report:

- deleted crate/files;
- new state-owned lock module path;
- whether benchmark was moved or retired;
- dependency edge removed from `pulith-state`;
- active absence check result;
- exact verification command outputs;
- next reduction recommendation, likely `pulith-platform` freeze/remove evaluation or internal monolith splitting of `pulith-state`/`pulith-install`.

## Stop condition

Stop and ask before coding if the audit finds an active non-state consumer that uses `pulith-lock` as a standalone product API outside tests/docs/benchmarks.

That would mean we should keep the crate and instead only tighten its semantics/docs.
