# Phase 0 Product Intent and Existing-Tool Evaluation

## Status

Design/evaluation only. No crate fold, dependency change, or API move is authorized by this report.

This report answers the Phase 0 questions from `docs/report/top-down-architecture-reduction.md` and adds the rule requested in review:

> If a mature crate, standard-library facility, operating-system primitive, or native tool already does the job, Pulith should not recreate it unless Pulith needs a narrower semantic contract around it.

## Inputs checked

Repository evidence:

- `docs/architecture.md`
- `docs/AGENT.md`
- `docs/architecture/shim.md`
- `docs/architecture/lock.md`
- `docs/architecture/serialization.md`
- `docs/architecture/platform.md`
- candidate crate sources for `pulith-shim`, `pulith-lock`, `pulith-serde-backend`
- internal usage searches for `pulith_shim`, `pulith_lock`, `pulith_serde_backend`, `pulith_platform`
- `cargo metadata --no-deps --format-version 1`

External/native wheel check via `cargo search --registry crates-io`:

- executable/path discovery: `which 8.0.4`, `directories 6.0.0`, `dirs 6.0.0`, `path-absolutize 3.1.1`
- file/process locks: `fslock 0.2.1`, `lockfile 0.4.0`, `fd-lock 4.0.4`
- serialization: `serde_json 1.0.150`, `postcard 1.1.3`, `ciborium 0.2.2`
- platform/system info: `os_info 3.15.0`, `sysinfo 0.39.5`, `target-lexicon 0.13.5`

Local `Cargo.lock` already includes `serde_json`, `ciborium` via benchmark dependencies, and `sysinfo` via benchmark tooling, but current workspace package dependencies use `serde_json` directly only through `pulith-serde-backend` and related crates.

## First principle: what should Pulith own?

Pulith should own **resource-manager semantics**, not generic utilities.

Own when:

- the type carries Pulith-specific meaning in the canonical pipeline;
- the behavior is a security/integrity/platform contract Pulith must guarantee;
- the abstraction reduces repeated caller glue across real Pulith call paths;
- the abstraction makes side effects explicit and testable.

Do not own when:

- the API is a generic utility with no Pulith semantics;
- a mature crate or standard library/native API already provides the mechanism;
- only one current caller exists and the abstraction merely forwards generic behavior;
- the API exists for hypothetical future backend flexibility only.

## Candidate 1: `pulith-shim`

### Current functionality

Evidence from `docs/architecture/shim.md` and source:

- Defines `TargetResolver`:
  - `fn resolve(&self, command: &str) -> Option<PathBuf>`
- Defines `PairResolver` and `TripleResolver` fallback combinators.
- Has a small error module, but current public API is mostly resolver vocabulary.
- Describes a possible shim binary, but the crate itself does not implement a real cross-platform shim executable generator.
- Only real internal consumer found: `pulith-install/src/lib.rs`, where `InstalledShimResolver` implements `TargetResolver`.

### Existing wheel/native check

Existing mechanisms already cover most generic pieces:

- Rust trait composition does not need a crate.
- Fallback resolution can be expressed with `Iterator::find_map`, `Option::or_else`, or a small owner-local helper.
- Executable discovery has existing crate support: `which 8.0.4`.
- Native activation mechanisms already exist by platform:
  - symlink/hardlink/copy for filesystem activation;
  - shell wrapper scripts / `.cmd` / `.ps1` / executable launchers for command shims;
  - OS path/env conventions handled by platform/native APIs.

Pulith may still need **shim semantics** if it means "activation target is resolved at invocation time from current state", but the current `pulith-shim` crate only defines a generic resolver trait and two small combinators.

### Product-intent answer

Is `pulith-shim` meant to be used without `pulith-install`?

**Current evidence says no.** It has one internal consumer and does not provide a standalone shim generator/runtime.

### Recommendation

Fold `pulith-shim` into `pulith-install` during the install reduction slice, unless the project explicitly commits to a standalone shim product.

Target ownership:

- `pulith-install::activation` or `pulith-install::shim` owns invocation-time activation resolution.
- Replace `PairResolver`/`TripleResolver` with owner-local composition or concrete functions.
- Do not add a new generic shim framework.
- If executable lookup is needed, prefer the existing `which` crate or platform-native PATH handling rather than recreating it.

### Required abstraction after fold

Keep only a Pulith-specific concept, not generic resolver machinery:

```text
ActivationTarget / ShimActivation / InstalledCommandBinding
```

The required contract is:

```text
installed command activation can resolve current active target at invocation time
```

## Candidate 2: `pulith-lock`

### Current functionality

Evidence from `docs/architecture/lock.md` and source:

- Defines deterministic resource lock representation:
  - `LockedResource`
  - `LockFile`
  - `LockDiff`
  - `LockResourceChange`
- Uses `BTreeMap` for stable key order.
- Serializes through `pulith-serde-backend` JSON adapter.
- Supports explicit diff: added/removed/changed.
- Current internal consumer found: `pulith-state::export_lock_file()`.
- Benchmark exists under `crates/pulith-lock/benches/lock_diff.rs`.

### Existing wheel/native check

Search results such as `fslock`, `lockfile`, and `fd-lock` are **file/process lock** crates. They do not solve Pulith's lock-file meaning.

Pulith's lock is not an OS mutex or file guard. It is a deterministic resource snapshot/diff model. Existing package-manager lockfile parsers are ecosystem-specific and would be the wrong owner for Pulith's resource semantics.

### Product-intent answer

Is `pulith-lock` meant to be used without `pulith-state`?

**Current evidence is ambiguous but leans no for the active workflow.** The only runtime owner found is `pulith-state::export_lock_file()`. However, the concept itself is a plausible standalone artifact if Pulith wants external reproducibility tooling.

### Recommendation

Do not fold immediately. Decide product intent explicitly:

- If lock files are a public artifact users edit/diff independently, keep `pulith-lock`.
- If lock files are only an export view of lifecycle state, fold into `pulith-state::lock` or `pulith-state::export`.

Given current usage, the better next step is a **state/lock design slice**, not direct folding.

### Required abstraction if kept

Keep:

```text
LockFile
LockedResource
LockDiff
LockResourceChange
```

But tighten semantics:

- derive `LockedResource` from state/resource records without duplicating fields by hand;
- keep deterministic ordering and schema version;
- keep diff policy-free.

### Required abstraction if folded

Fold as:

```text
pulith_state::lock::{LockFile, LockedResource, LockDiff}
```

and keep it as an export/report type, not an independent state backend.

## Candidate 3: `pulith-serde-backend`

### Current functionality

Evidence from `docs/architecture/serialization.md` and source:

- Defines `TextCodec` trait:
  - `encode_pretty<T: Serialize>() -> String`
  - `decode_str<T: DeserializeOwned>() -> T`
- Provides `JsonTextCodec` and `CompactJsonTextCodec`.
- Provides helpers `encode_pretty_vec` and `decode_slice`.
- Used by `pulith-lock`, `pulith-state`, `pulith-store`, and `pulith-install` tests/backup paths.
- Current docs say future backend evolution may include binary codec, SQLite blob column, alternate JSON adapter, postcard, or similar.

### Existing wheel/native check

Mature existing serialization crates already exist:

- `serde_json 1.0.150` for JSON.
- `postcard 1.1.3` for compact binary serde.
- `ciborium 0.2.2` for CBOR.

Pulith should not invent serialization formats. It should only own schema/version validation and persistence-boundary semantics.

Current `pulith-serde-backend` is not a format implementation; it is a thin trait over serde JSON. The only second implementation is compact JSON for parity tests, not a fundamentally different backend.

### Product-intent answer

Is `pulith-serde-backend` meant to support real non-JSON backends now?

**Current evidence says not yet.** The active implementation is JSON and compact JSON. There is no active postcard/CBOR/sqlite backend in the workspace.

### Recommendation

Treat `pulith-serde-backend` as a likely dead/hypothetical abstraction unless the next slice adds a real alternate backend with production use.

Prefer one of two directions:

1. **Delete/fold now during a persistence reduction slice**:
   - Owners call `serde_json` directly through small private helpers.
   - Schema/version validation stays in `pulith-state`, `pulith-store`, and lock export owners.
   - Compact JSON parity tests become owner-local tests.

2. **Keep only if a real backend is introduced now**:
   - Add actual `postcard` or `ciborium` adapter.
   - Add cross-backend semantic parity tests.
   - Add explicit API selection by caller.

No third option: do not keep a public backend crate purely for someday.

### Required abstraction

Pulith needs:

```text
schema version validation
stable/deterministic persistence representation
clear typed decode errors
```

Pulith does **not** need a public codec trait unless at least two real backend families are active.

## Candidate 4: `pulith-platform`

### Current functionality

Evidence from `docs/architecture/platform.md` and source usage search:

- API covers OS/distro, architecture/target triple, shell, user dirs, and PATH/env helpers.
- Depends on `query-shell` and `home`.
- Internal Rust usage search found no current `pulith_platform` consumers in workspace source.
- `cargo metadata` shows no internal package depends on `pulith-platform`.

### Existing wheel/native check

Existing crates cover most generic functionality:

- directories: `directories 6.0.0`, `dirs 6.0.0`
- OS info: `os_info 3.15.0`, `sysinfo 0.39.5`
- target triples: `target-lexicon 0.13.5`
- executable lookup: `which 8.0.4`
- path normalization: `path-absolutize 3.1.1`
- standard library and OS APIs cover env vars, PATH, process spawning, and many path operations.

Pulith should not recreate a generic platform crate if all it offers is wrapper names around existing crates/native APIs.

### Product-intent answer

Is `pulith-platform` a standalone product crate or future/core support crate?

**Current evidence says it is not active core support yet.** It has no current internal consumers. It may be a future product API, but that is not proven by active workflows.

### Recommendation

Do not fold into another crate yet, but do not expand it either.

Classify as **dormant/future-support** until a real core workflow consumes it.

Options:

1. Keep but mark as not currently on the canonical pipeline path.
2. If publishing pressure matters, set `publish = false` until it has active product use.
3. If a workflow needs platform behavior, first check existing crates/native APIs and use them directly or as private owner-local helpers.
4. Only keep `pulith-platform` public if it adds Pulith-specific normalization semantics beyond `directories`, `os_info`, `target-lexicon`, `which`, and `std`.

### Required abstraction

Only keep public abstractions if they encode Pulith-specific cross-platform contracts:

```text
activation path semantics
shell profile mutation target
resource manager config/data/cache roots
platform support limitations as typed values
```

Generic OS/arch/shell wrappers are not enough.

## Phase 0 decision table

| Crate | Active consumer evidence | Existing wheel/native overlap | Phase 0 decision | Next design slice |
| --- | --- | --- | --- | --- |
| `pulith-shim` | Only `pulith-install` implements/uses resolver | `which`, std/native symlink/script/launcher mechanics | Fold candidate; likely not standalone | `pulith-install` activation/shim reduction |
| `pulith-lock` | `pulith-state::export_lock_file`; own bench | OS lock crates do not replace semantic resource lock | Decide product intent; keep if public artifact, fold if state export only | `pulith-state` lock/export reduction |
| `pulith-serde-backend` | state/store/lock/install use JSON helpers | `serde_json`, `postcard`, `ciborium` already exist | Delete/fold unless real non-JSON backend is added now | persistence boundary reduction |
| `pulith-platform` | No internal source consumers found | `directories`, `dirs`, `os_info`, `sysinfo`, `target-lexicon`, `which`, std/native APIs | Dormant; do not expand; maybe publish=false | platform intent/API reduction |

## Recommended Phase 0 answers

1. `pulith-shim` should **not** remain a crate by default. Fold it into install activation unless standalone shim generation becomes a product.
2. `pulith-lock` should be decided by user/product intent. It is not replaceable by file-lock crates, but current workflow suggests it may belong under state.
3. `pulith-serde-backend` should be deleted/folded unless a real second backend is introduced immediately. Do not keep public codec abstraction for hypothetical flexibility.
4. `pulith-platform` should be frozen as dormant or marked non-publish until active workflows need Pulith-specific platform semantics. Prefer existing crates/native APIs for generic platform work.

## Recommended next move

Start with one focused design slice, not code:

```text
pulith-install activation/shim reduction
```

Reason:

- `pulith-shim` has the clearest fold evidence.
- The fold target is obvious: install activation.
- Existing wheels/native mechanisms prevent inventing a new generic shim framework.
- This slice can also clarify whether activation needs a real public abstraction or just concrete install-owned types.

Proposed design questions for that slice:

1. What activation modes are actually supported: symlink, copy, shim script, resolver-at-invocation?
2. Which are native filesystem operations vs generated wrapper files?
3. What public type should represent activation target and activation receipt?
4. Can `TargetResolver` disappear in favor of concrete `InstalledCommandBinding` resolution?
5. Should executable PATH discovery use `which` or stay caller-owned?

Do not start with `pulith-serde-backend` unless the team first decides whether alternate persistence backends are in scope now.
