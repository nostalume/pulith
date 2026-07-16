# Phase 3 Platform Freeze/Fold Plan

## Status

Plan only. Do not edit Rust code from this report alone.

This is the next reduction phase after:

1. `pulith-shim` was folded into `pulith-install`.
2. `pulith-serde-backend` was folded into owner-local JSON boundaries.
3. `pulith-lock` was folded into `pulith-state` as state-owned lock export/diff.

## Goal

Decide and implement the smallest honest boundary for `pulith-platform`.

Expected target, if audit remains unchanged:

```text
pulith-platform stops being a public core crate.
It is either removed from the workspace/public crate set, or frozen as publish=false/internal-only until real Pulith-specific platform semantics appear.
No active workflow crate depends on it today.
```

## Why this phase next

The previous phases removed thin one-consumer or dead abstraction crates. `pulith-platform` is the remaining Phase 0 concern.

Current evidence says it is different from `pulith-shim` and `pulith-serde-backend`:

- it is not currently a one-consumer dependency;
- it has no active internal dependents at all;
- it wraps generic OS/shell/directory/command helpers;
- many of those jobs already have mature wheels/native APIs (`std::env`, `std::process`, `home`, `query-shell`, `target-lexicon`, `directories`, `which`, `sysinfo`, etc.);
- keeping it public implies Pulith has a platform API product, but no current workflow uses that API.

Cargo metadata evidence at plan time:

```text
pulith-platform deps = home,once_cell,pulith-fs,query-shell,thiserror
```

No package printed as depending on `pulith-platform`.

Source audit evidence at plan time:

```text
crates/pulith-platform/src/lib.rs exports:
  arch
  command
  dir
  env
  os
  shell
```

## Product-intent decision to make

Before coding, answer this concrete product question:

```text
Is pulith-platform part of Pulith's public product surface, or is it dormant generic utility code?
```

Recommended decision if no new consumer appears:

```text
Treat pulith-platform as dormant generic utility code and remove it from the active public core wave.
```

Two implementation choices are acceptable:

1. **Remove** `crates/pulith-platform/` entirely.
2. **Freeze internal** by setting `publish = false`, removing it from public docs/publish wave, and leaving it only as future internal scratch if the user wants to keep the source for now.

Preferred ponytail choice:

```text
Delete it.
```

Reason: no active internal consumer means there is no code migration cost. Freezing keeps a dormant crate that can drift.

## Scope

In scope:

- Audit active consumers of `pulith_platform` / `pulith-platform`.
- Verify no active crates/examples depend on it.
- Delete `crates/pulith-platform/` if no consumer appears.
- Remove it from explicit workspace members.
- Update README, architecture, publish docs, crate lists, and active docs.
- Run active-surface absence checks and canonical verification.

Out of scope:

- Do not redesign platform helpers into another crate.
- Do not create a compatibility re-export module.
- Do not add new dependencies just to preserve old functionality.
- Do not migrate generic helpers into unrelated crates unless a current caller proves the exact needed function.
- Do not alter install/state/fetch behavior unless absence checks reveal a real dependency.

## Stop condition

Stop and redesign before deleting if the audit finds an active non-test workflow consumer of `pulith-platform` that cannot be replaced with local `std`/existing wheel usage in one small owner-local edit.

Examples of stop-condition evidence:

- `pulith-install` actively needs `Shell` or `Command` semantics;
- `pulith-fetch` actively needs platform path/env normalization;
- examples use `pulith-platform` as part of the intended public composition story.

If that happens, the phase should become a targeted fold into the actual owner, not a blind delete.

## Implementation plan after approval

### Step 1 — Active usage audit

Run:

```bash
grep -R "pulith_platform\|pulith-platform" -n crates examples Cargo.toml Cargo.lock README.md docs/architecture.md docs/AGENT.md docs/publish
```

Also run metadata fan-in check:

```bash
python - <<'PY'
import json, subprocess
p = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], text=True))
for pkg in p['packages']:
    deps = [d['name'] for d in pkg['dependencies']]
    if 'pulith-platform' in deps or pkg['name'] == 'pulith-platform':
        print(pkg['name'], 'deps=', ','.join(deps))
PY
```

Expected current output:

```text
pulith-platform deps= home,once_cell,pulith-fs,query-shell,thiserror
```

No other dependent package should print.

### Step 2 — Read exact crate modules

Read:

- `crates/pulith-platform/src/lib.rs`
- `crates/pulith-platform/src/arch.rs`
- `crates/pulith-platform/src/command.rs`
- `crates/pulith-platform/src/dir.rs`
- `crates/pulith-platform/src/env.rs`
- `crates/pulith-platform/src/os.rs`
- `crates/pulith-platform/src/shell.rs`
- `crates/pulith-platform/src/error.rs`
- `crates/pulith-platform/Cargo.toml`
- `crates/pulith-platform/README.md`
- `docs/architecture/platform.md`

Classify each module as:

- generic wrapper over existing crate/std API;
- Pulith-specific contract;
- unused test-only helper.

### Step 3 — Choose delete vs freeze

If no active consumer and no Pulith-specific contract appears, choose delete.

If the user prefers preserving dormant source, choose freeze:

```toml
publish = false
```

and remove it from publish docs only.

Preferred implementation for this phase is delete.

### Step 4 — Delete platform crate if chosen

After audit passes:

```bash
git rm -r crates/pulith-platform
```

Then remove from explicit workspace members in root `Cargo.toml`:

```toml
"crates/pulith-platform",
```

### Step 5 — Update active docs

Update:

- `README.md`
- `docs/AGENT.md`
- `docs/architecture.md`
- `docs/architecture/platform.md` or delete it after merging any useful note into `docs/architecture.md`
- `docs/publish/overview.md`
- `docs/publish/checklist.md`
- `docs/publish/readiness-matrix.md`
- any crate README mentioning `pulith-platform`

Historical `docs/report/` mentions can remain as evidence.

### Step 6 — Active absence checks

Run:

```bash
python - <<'PY'
from pathlib import Path
root = Path('.')
needles = ['pulith-platform', 'pulith_platform']
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
print('no active crates/examples/manifest/lock pulith-platform references')
PY
```

Verify metadata:

```bash
python - <<'PY'
import json, subprocess
p = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], text=True))
print('pulith-platform in metadata:', 'pulith-platform' in [pkg['name'] for pkg in p['packages']])
PY
```

Expected after delete:

```text
pulith-platform in metadata: False
```

### Step 7 — Canonical verification

Run:

```bash
cargo fmt --all --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps
```

If Hermes stale-verification guard triggers afterward, run a focused temp `F:/Stratum/TEMP/hermes-verify-*.py` script proving:

- `crates/pulith-platform` is absent or `publish = false` is present, depending on chosen path;
- no active crates/examples/manifests import `pulith_platform`;
- `cargo metadata` matches the chosen outcome;
- active docs no longer list it as a public core crate.

## Expected final report format

After implementation, report:

- delete vs freeze decision;
- exact files deleted or changed;
- whether any owner-local replacement was needed;
- active absence check result;
- exact verification command outputs;
- next reduction recommendation.

## Likely next phase after platform

After `pulith-platform`, stop deleting crates by default and switch from crate-boundary deletion to **monolith/internal module reduction**.

Likely next design target:

```text
pulith-install/src/lib.rs split/fold cleanup
```

or:

```text
pulith-state/src/lib.rs internal module split after lock moved in
```

Do not code either without a fresh report, because those are internal API-shape refactors rather than obvious dead-crate deletions.
