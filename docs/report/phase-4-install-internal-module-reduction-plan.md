# Phase 4 Install Internal Module Reduction Plan

## Status

Plan only. Do not edit Rust code from this report alone.

This is the next phase after the crate-boundary cleanup sequence:

1. `pulith-shim` folded into `pulith-install`.
2. `pulith-serde-backend` folded into owner-local JSON boundaries.
3. `pulith-lock` folded into `pulith-state` as state-owned lock export/diff.
4. `pulith-platform` removed as dormant generic utility surface.

## Goal

Reduce `pulith-install/src/lib.rs` from one large mixed module into owner-local modules without changing the public crate API.

Target outcome:

```text
pulith-install remains one crate.
Public imports from `pulith_install::{...}` keep working.
Internal files are split by install responsibility, not by aesthetic line count.
No compatibility shims, duplicate types, or wrapper modules are introduced.
```

## Why this phase next

The obvious dead/thin crates are gone. The next risk is not crate count; it is large-file drift.

Current file-size audit:

```text
crates/pulith-install/src/lib.rs 2496 lines
crates/pulith-state/src/lib.rs   2590 lines
```

Both are large, but `pulith-install` is the better next target because it contains several separable workflow concerns already visible from the public type groups:

- readiness + backup/restore/uninstall composition;
- install input/spec/planning;
- type-state flow: planned -> staged -> installed -> activated;
- activation traits and activators;
- lifecycle receipt envelope;
- filesystem helpers and tests.

`pulith-state` just absorbed lock export and should be left stable for one phase before more movement.

## Non-goal

This is **not** another crate deletion phase.

Do not delete `pulith-install`. Do not split it into new crates. Do not move install behavior into state/store/fs. The goal is internal module ownership only.

## Current API groups

From `crates/pulith-install/src/lib.rs`:

```text
error / result:
  InstallError
  Result

ready + recovery + uninstall:
  InstallReady
  BackupReceipt
  RestoreReceipt
  UninstallOptions
  UninstallDisposition
  UninstallReceipt

activation:
  ActivationTarget
  ActivationReceipt
  Activator
  SymlinkActivator
  CopyFileActivator
  ShimCommand
  InstalledShimResolver
  ShimLinkActivator
  ShimCopyActivator
  ActivationRequest

planning + input + spec:
  InstallMode
  InstallWorkflowVariant
  InstallWritableScope
  ConnectivityMode
  ActivationSupport
  RollbackSupport
  InstallCapabilities
  InstallPlanningRequest
  InstallPlanLimitation
  InstallPlanReport
  InstallInput
  IntoInstallInput
  InstallSpec

flow + receipts:
  Planned / Staged / Installed / Activated markers
  StagingArea
  InstallFlow<S>
  PlannedInstall
  StagedInstall
  InstalledInstall
  ActivatedInstall
  InstallReceipt
  RollbackReceipt

lifecycle receipt envelope:
  LifecycleOperationPhase
  LifecycleOperationDetails
  LifecycleOperationReceipt
```

## Target module layout

Preferred layout:

```text
crates/pulith-install/src/lib.rs
crates/pulith-install/src/error.rs
crates/pulith-install/src/ready.rs
crates/pulith-install/src/input.rs
crates/pulith-install/src/plan.rs
crates/pulith-install/src/flow.rs
crates/pulith-install/src/activation.rs
crates/pulith-install/src/receipt.rs
crates/pulith-install/src/fs_ops.rs
```

Responsibilities:

### `error.rs`

Owns:

```rust
pub type Result<T> = std::result::Result<T, InstallError>;
pub enum InstallError { ... }
```

No behavior besides error definitions.

### `ready.rs`

Owns installed-resource composition around state:

```rust
InstallReady
BackupReceipt
RestoreReceipt
UninstallOptions
UninstallDisposition
UninstallReceipt
```

This is where backup/restore/uninstall methods live.

### `input.rs`

Owns materialized install input boundaries:

```rust
InstallInput
IntoInstallInput
```

This module should not import fetch/archive transport types directly. It may depend on `pulith_store::{StoredArtifact, ExtractedArtifact}` because those are materialized semantic handles.

### `plan.rs`

Owns read-only install planning contracts:

```rust
InstallMode
InstallWorkflowVariant
InstallWritableScope
ConnectivityMode
ActivationSupport
RollbackSupport
InstallCapabilities
InstallPlanningRequest
InstallPlanLimitation
InstallPlanReport
InstallSpec::plan(...) if cleaner
```

No filesystem mutation.

### `flow.rs`

Owns type-state workflow:

```rust
Planned
Staged
Installed
Activated
StagingArea
InstallFlow<S>
PlannedInstall
StagedInstall
InstalledInstall
ActivatedInstall
InstallReceipt
RollbackReceipt
```

This module composes `input`, `plan`, `activation`, `ready`, and `fs_ops`.

### `activation.rs`

Owns activation contracts and built-ins:

```rust
ActivationTarget
ActivationReceipt
ActivationRequest
Activator
SymlinkActivator
CopyFileActivator
ShimCommand
InstalledShimResolver
ShimLinkActivator
ShimCopyActivator
```

Keep platform behavior explicit here. Do not introduce a platform crate or fallback magic.

### `receipt.rs`

Owns lifecycle receipt envelope:

```rust
LifecycleOperationPhase
LifecycleOperationDetails
LifecycleOperationReceipt
```

Conversions from operation-specific receipts can live here.

### `fs_ops.rs`

Private helper module for filesystem mechanics:

```rust
remove_existing_target
path_entry_exists
sanitize_component
copy/restore/stage helpers
resolve_shim_target if not better in activation.rs
```

Keep it private: no public helper API unless a caller already uses it.

## Re-export policy

`lib.rs` should be a thin public surface map:

```rust
pub mod activation;
mod error;
mod flow;
mod fs_ops;
mod input;
mod plan;
mod ready;
mod receipt;

pub use activation::{...};
pub use error::{InstallError, Result};
pub use flow::{...};
pub use input::{...};
pub use plan::{...};
pub use ready::{...};
pub use receipt::{...};
```

This is not a compatibility shim. It is the crate root preserving the existing public API while internals become owned modules.

## Implementation plan after approval

### Step 1 — Create module skeletons without behavior changes

Create empty/private modules and move only imports needed by each module as behavior moves.

Do not change function bodies yet.

### Step 2 — Move error definitions first

Move `InstallError` and `Result` to `error.rs`.

Run:

```bash
cargo test -p pulith-install --all-features
```

### Step 3 — Move activation contracts and activators

Move activation types and helper code into `activation.rs`.

Preserve names and semantics:

- `SymlinkActivator` still maps Windows file-symlink privilege errors.
- copy activation still only supports file targets.
- shim activation remains install-owned.

Run:

```bash
cargo test -p pulith-install --all-features shim_
cargo test -p pulith-install --all-features activation
cargo test -p pulith-install --test workspace_pipeline --all-features repeated_symlink_activation_replaces_existing_file_target
cargo test -p pulith-install --test workspace_pipeline --all-features repeated_copy_activation_replaces_existing_file_target
```

### Step 4 — Move input/spec/planning

Move `InstallInput`, `IntoInstallInput`, planning enums, `InstallCapabilities`, `InstallPlanningRequest`, `InstallPlanLimitation`, `InstallPlanReport`, and `InstallSpec` into `input.rs` / `plan.rs`.

Run:

```bash
cargo test -p pulith-install --all-features install_plan
cargo test -p pulith-install --all-features install_spec_new_with_input_absorbs
```

### Step 5 — Move type-state flow

Move type-state markers, `InstallFlow<S>`, aliases, staging/commit/activate/rollback/finish methods, and install/rollback receipts into `flow.rs`.

Run:

```bash
cargo test -p pulith-install --all-features rollback
cargo test -p pulith-install --all-features replace_existing
cargo test -p pulith-install --test workspace_pipeline --all-features archive_replace_activate_rollback_restores_previous_activation_snapshot
```

### Step 6 — Move ready/recovery/uninstall composition

Move `InstallReady` and backup/restore/uninstall receipts/options into `ready.rs`.

Run:

```bash
cargo test -p pulith-install --all-features backup_and_restore_round_trip_install_and_state
cargo test -p pulith-install --all-features uninstall_resource
cargo test -p pulith-install --test workspace_pipeline --all-features recovery_contract_backup_restore_recovers_install_and_state_facts
```

### Step 7 — Move lifecycle receipt envelope

Move `LifecycleOperation*` types and `From<...>` conversions into `receipt.rs`.

Run:

```bash
cargo test -p pulith-install --all-features lifecycle_receipt
```

### Step 8 — Private filesystem helpers

Move helper functions into `fs_ops.rs` only if they are shared across modules.

Do not create helper wrappers for one-use code. If a function is only used by one module, keep it in that module.

Run:

```bash
cargo test -p pulith-install --all-features
```

### Step 9 — Public API absence/regression check

Run a compile check against the examples and workspace tests:

```bash
cargo check --workspace --all-features
cargo test -p pulith-install --all-features
cargo test --workspace --all-features
```

Also run a simple source layout check:

```bash
python - <<'PY'
from pathlib import Path
lib = Path('crates/pulith-install/src/lib.rs').read_text(encoding='utf-8')
assert 'pub enum InstallError' not in lib
assert 'pub struct InstallReady' not in lib
assert 'pub struct InstallSpec' not in lib
assert 'pub trait Activator' not in lib
assert len(lib.splitlines()) < 250
for rel in ['error.rs','ready.rs','input.rs','plan.rs','flow.rs','activation.rs','receipt.rs']:
    assert (Path('crates/pulith-install/src') / rel).exists(), rel
print('pulith-install root is re-export surface, owner modules exist')
PY
```

### Step 10 — Canonical verification

Run:

```bash
cargo fmt --all --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps
```

If Hermes stale-verification guard triggers afterward, run a focused temp `F:/Stratum/TEMP/hermes-verify-*.py` script proving the module split shape and public imports.

## Risk controls

- Preserve every existing public type name.
- Preserve crate-root re-exports so examples do not change unless a test proves they should.
- Do not introduce new state machines or wrappers.
- Do not move install behavior into state/store/fs.
- Do not make `fs_ops.rs` a dumping ground; keep one-use helpers next to their owner.
- Stop if module moves start forcing semantic changes instead of import changes.

## Expected final report format

After implementation, report:

- files created and what each owns;
- `lib.rs` line count before/after;
- public API compatibility evidence from examples/workspace checks;
- focused `pulith-install` test results;
- canonical verification results;
- next phase recommendation.

## Likely next phase after this

After `pulith-install` is modularized, the next design target should be `pulith-state/src/lib.rs` internal module reduction.

Do not start it without a fresh report. `pulith-state` has several separable surfaces, but it is stateful and easy to over-split:

```text
state persistence
resource records/upsert/patch
resource inspection/repair
activation ownership
store retention planning
resource snapshot/restore
lock export
```

The plan should preserve state API semantics and split only where it deletes real ownership ambiguity.
