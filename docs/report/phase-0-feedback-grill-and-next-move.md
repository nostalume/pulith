# Phase 0 Feedback Grill and Next Move

## Status

Design/evaluation only. No code moves, crate folds, dependency changes, or API changes are authorized by this report.

This report incorporates the review feedback:

1. If a crate is only a collection of existing wheels, remove the crate instead of maintaining a wrapper package.
2. Remove dead backend abstractions; prefer one optimal persistence choice over confusing branches.
3. A lock file like `uv.lock` / `Cargo.lock` locks resolved versions and sources. Do not design a vague Pulith lock crate if existing solver/lockfile wheels or a simpler state export is enough.

## Feedback translated into architecture rules

### Rule 1: no wrapper crates around wheels

A crate must not survive only because it groups generic utilities. If its API is just a friendlier front for `std`, operating-system behavior, or mature crates, it should be removed or folded into the real owner.

Keep a wrapper only when Pulith adds a **narrow semantic contract** that existing wheels do not provide.

Examples:

- `directories`, `dirs`, `home`: already know user dirs.
- `which`: already finds executables on PATH.
- `os_info`, `sysinfo`, `target-lexicon`: already model OS/system/target facts.
- `serde_json`, `postcard`, `ciborium`, `toml_edit`: already encode/decode common durable formats.
- `cargo-lock`, `lockfiles`, `aube-lockfile`: already parse existing ecosystem lockfiles.
- `pubgrub`, `semver`, `version-ranges`: already solve/range-check versions.

Pulith should not recreate these as broad utility crates.

### Rule 2: one optimal persistence path first

A public backend abstraction is worse than direct JSON/TOML if there is no real caller-selected backend.

Current Pulith persistence needs:

- explicit schema version;
- deterministic ordering where diff/export cares;
- typed decode errors;
- stable tests.

It does **not** currently need:

- public `TextCodec` trait;
- alternate backend selection;
- JSON-vs-compact-JSON branch as architecture.

Choose the simplest stable baseline first. Current repo already uses JSON; keep JSON unless a user-facing reason exists to prefer TOML for lock-style files.

### Rule 3: lockfile is a product format or nothing

A lock file is not just a deterministic map. It must answer:

```text
what resolved version/source/digest is pinned, and why can this reproduce later?
```

If Pulith cannot answer that better than state export, then `pulith-lock` is not a product. It is only a report/export shape and should live under `pulith-state` or be removed.

If Pulith later needs real dependency solving or version conflict resolution, use an existing solver wheel such as `pubgrub` / `semver` / `version-ranges`; do not create a Pulith solver.

## Existing-wheel evidence gathered

### Lock/version ecosystem

`cargo search --registry crates-io` found:

- `cargo-lock 11.0.1` — self-contained `Cargo.lock` parser with optional dependency graph analysis.
- `lockfiles 0.0.1` — multi-ecosystem lockfile parser normalized by PURL.
- `aube-lockfile 1.25.2` — multi-format lockfile reader/writer for Aube, pnpm, package-lock, yarn.
- `openvet-lockfile 0.6.0` — per-registry lockfile parsing into audit subject tuples.
- `toml_edit 0.25.12+spec-1.1.0` — format-preserving TOML parser.
- `pubgrub 0.4.0` / `astral-pubgrub 0.5.0` — version solving algorithm.
- `semver 1.0.28` — semantic version parsing/evaluation.
- `version-ranges 0.1.3` — version range operations.

Conclusion: if Pulith needs ecosystem lockfile parsing or dependency solving, use existing crates. If Pulith only exports its own installed-state pins, keep that export minimal and owner-local.

### Serialization ecosystem

`cargo search --registry crates-io` found:

- `serde_json 1.0.150`
- `postcard 1.1.3`
- `ciborium 0.2.2`

Conclusion: Pulith should not own a serialization backend crate unless it adds real user-selected backend semantics. Current `pulith-serde-backend` is a dead abstraction over JSON.

### Platform/generic utility ecosystem

`cargo search --registry crates-io` found:

- `which 8.0.4`
- `directories 6.0.0`
- `dirs 6.0.0`
- `path-absolutize 3.1.1`
- `os_info 3.15.0`
- `sysinfo 0.39.5`
- `target-lexicon 0.13.5`

Conclusion: `pulith-platform` should not remain a public crate if it only collects generic OS/dir/shell/target helpers.

## Candidate grill

### `pulith-serde-backend`: reject current abstraction

Current state:

- `TextCodec` trait.
- `JsonTextCodec` and `CompactJsonTextCodec`.
- Helpers around serde JSON.
- No active Postcard/CBOR/SQLite backend.
- No caller-selected persistence backend.

Grill:

- This is not an abstraction earned by use. It is a future option encoded as public API.
- It creates a confusing branch: pretty JSON vs compact JSON looks like architecture, but it is only formatting/parity.
- It obscures the real owner: state/store/lock schema validation.

Decision:

```text
Delete/fold pulith-serde-backend.
```

Preferred replacement:

- owner-local private JSON helpers in `pulith-state` and `pulith-store`;
- direct `serde_json` usage with schema validation at the owner boundary;
- if lock export survives, it uses the same direct format helper or a tiny state-local module.

Do not introduce Postcard/CBOR now. That would preserve the dead abstraction by feeding it a token implementation. Use the optimal current choice: schema-versioned JSON.

### `pulith-lock`: demote from crate unless product semantics are proven

Current state:

- `LockFile` is a deterministic map of resource key to version/source/digest/metadata.
- Only active runtime owner is `pulith-state::export_lock_file()`.
- It does not solve dependencies.
- It does not explain source resolution beyond copied strings.
- It depends on dead backend abstraction.

Grill:

- A lock file like `uv.lock` is valuable because it captures the result of resolution for reproducibility.
- Pulith explicitly says dependency solving and lock orchestration are out of scope for core crates.
- Therefore a standalone `pulith-lock` crate is suspicious: it suggests package-manager-level product behavior without owning solving/resolution.
- Existing wheels already cover many lockfile and solver concerns. Pulith should not recreate them.

Decision:

```text
Do not keep pulith-lock as an independent crate by default.
```

Better model:

- If the current need is export: make it `pulith-state` export/report data.
- If the future need is dependency/version solving: design around `pubgrub`/`semver`/`version-ranges`, not a hand-made lock crate.
- If the future need is interoperating with existing ecosystem locks: use `cargo-lock`, `lockfiles`, or domain-specific parsers.

Minimum surviving abstraction:

```text
StateExportLock or ResolvedResourcePin
```

owned by `pulith-state`, not a crate.

### `pulith-platform`: likely wrapper crate, freeze/remove unless Pulith-specific contract appears

Current state:

- No internal source consumers found.
- API is broad: OS/distro, arch/target triple, shell, user dirs, env/PATH.
- Existing crates/native APIs cover most of it.

Grill:

- This is a classic collector crate risk.
- It groups wheels rather than expressing one Pulith-specific boundary.
- The fact that no core crate consumes it means it is not yet part of the canonical pipeline.

Decision:

```text
Freeze as dormant; mark non-publish or remove in a cleanup slice unless an active workflow needs Pulith-specific platform semantics.
```

What would make it valid:

- activation path semantics;
- shell profile mutation target chosen by Pulith's install activation model;
- resource-manager config/data/cache roots with explicit guarantees;
- platform limitations as typed values in install/state reports.

Generic OS/arch/shell wrappers do not justify a crate.

### `pulith-shim`: fold into install activation

Current state:

- Tiny resolver crate.
- Only real consumer: `pulith-install`.
- Does not provide a complete shim binary generator.

Grill:

- `TargetResolver` is a generic callback, not a product API.
- `PairResolver` and `TripleResolver` are `or_else`/`find_map` in crate form.
- If PATH discovery is needed, `which` exists.
- If wrapper scripts are needed, the install activation owner should generate them directly.

Decision:

```text
Fold pulith-shim into pulith-install activation.
```

Surviving concept should be concrete:

```text
InstalledCommandBinding
ActivationTarget
ShimScript / WrapperActivation if actually generated
```

No generic shim framework.

## Revised Phase 0 decision table

| Crate | Feedback-grilled decision | Why |
| --- | --- | --- |
| `pulith-serde-backend` | Delete/fold | Dead backend abstraction; choose schema-versioned JSON directly. |
| `pulith-lock` | Demote/fold into state export unless product semantics proven | Lockfile without solving/reproducibility semantics is just state export; wheels exist for lock parsing/solving. |
| `pulith-platform` | Freeze, non-publish, or remove unless Pulith-specific platform contract appears | Currently a collection of generic utility wheels with no internal consumers. |
| `pulith-shim` | Fold into install activation | One-consumer generic resolver crate; native/wheel mechanisms already exist. |

## Revised next move

Proceed with a focused design slice:

```text
pulith-install activation/shim reduction
```

Reason:

- Clearest one-consumer crate fold.
- Does not require deciding persistence format or lockfile product semantics first.
- Can remove generic resolver abstraction without touching wider state/store persistence.
- Keeps the next implementation small and verifiable.

But with feedback applied, the follow-up queue changes:

1. `pulith-install` activation/shim reduction — fold `pulith-shim` if design confirms no real standalone shim product.
2. Persistence cleanup — delete/fold `pulith-serde-backend`, choose direct schema-versioned JSON.
3. State lock/export cleanup — demote `pulith-lock` into state export or delete if not needed.
4. Platform cleanup — freeze/non-publish/remove `pulith-platform` unless an active workflow proves Pulith-specific platform semantics.

## Detailed next-slice questions: install activation/shim

Before code, answer these in a dedicated report:

1. What activation modes exist today in `pulith-install`?
   - symlink activation;
   - copy activation;
   - shim-link activation;
   - shim-copy activation;
   - installed resolver at invocation time.
2. Which modes are actual native filesystem operations?
3. Which modes generate wrapper files or executable scripts?
4. Does any public caller need to implement `TargetResolver`, or can install own all activation resolution?
5. Can `PairResolver`/`TripleResolver` be deleted with no replacement?
6. Does executable lookup need `which`, or should callers supply absolute installed executable paths?
7. What receipt fields prove activation without exposing internal staging/workspace details?

Expected target:

```text
pulith-install owns activation.
pulith-shim crate disappears.
No compatibility shim crate remains.
No generic resolver combinators remain.
```

## Non-goals for the next slice

- Do not touch lock/export yet.
- Do not touch serde backend yet.
- Do not touch platform crate yet.
- Do not add `which` unless activation design proves executable PATH lookup is Pulith-owned.
- Do not create a new `pulith-activation` crate.

## Verification expectation for implementation later

When implementation starts, require:

- import absence check for `pulith_shim` outside deleted docs/history;
- workspace manifest no longer includes `pulith-shim` if folded;
- `cargo check --workspace --all-features`;
- `cargo test -p pulith-install --all-features`;
- focused integration test for activation behavior still passing;
- docs updated in `docs/architecture/install.md`, `docs/architecture.md`, and crate READMEs.
