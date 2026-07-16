# Top-Down Architecture Evaluation and Reduction Brainstorm

## Status

Design/evaluation report only. Do not decompose modules or fold crates from this report alone.

This report is the current top-down checkpoint after reading:

- `docs/architecture.md`
- every file under `docs/architecture/`
- current `cargo metadata --no-deps --format-version 1`
- current crate source/file-size inventory

It supersedes the earlier crate-count-driven reduction order. The next work should clarify architecture first, then choose one behavior/crate slice.

## User correction captured

The current direction is:

```text
Top-down evaluation first.
Clarify architecture first.
Then focus on each specific behavior or crate.
Do not start direct file decomposition just because a file is large.
```

That means the previously drafted `phase-4-install-internal-module-reduction-plan.md` is not the active next implementation contract. It remains a possible later slice only after the architecture evaluation chooses install as the right behavior owner.

## Current implemented workspace state

The active workspace now has these core crates:

```text
pulith-version   files=  2 loc=  634
pulith-resource  files=  1 loc= 1117
pulith-source    files=  1 loc=  442
pulith-fs        files= 14 loc= 1493
pulith-verify    files=  4 loc=  424
pulith-archive   files=  9 loc= 2275
pulith-fetch     files= 32 loc= 9819
pulith-store     files=  1 loc= 1280
pulith-state     files=  2 loc= 2887
pulith-install   files=  1 loc= 2496
```

Removed/folded satellites:

```text
pulith-shim          removed; install owns shim activation/resolution
pulith-serde-backend removed; owners use direct schema-versioned JSON
pulith-lock          removed; state owns lock export/diff views
pulith-platform      removed; no active internal consumer/product boundary
```

Current internal dependency map from `cargo metadata`:

```text
pulith-version:        -
pulith-fs:             -
pulith-verify:         -
pulith-resource:       pulith-version
pulith-source:         pulith-resource
pulith-archive:        pulith-fs
pulith-fetch:          pulith-fs, pulith-resource, pulith-source, pulith-verify
pulith-store:          pulith-archive, pulith-fetch, pulith-fs, pulith-resource
pulith-state:          pulith-fs, pulith-resource, pulith-store
pulith-install:        pulith-archive, pulith-fetch, pulith-fs, pulith-resource, pulith-state, pulith-store, pulith-source
examples:              compose the above crates
```

Interpretation:

- The dead satellite crates are already gone.
- The current graph is not bloated by tiny utility crates anymore.
- The main design question has shifted from “which crate should be deleted?” to “which behavior boundaries are real, and which public surfaces still encode policy/glue/duplicate concepts?”

## Current architecture docs state

`docs/architecture/` currently contains:

```text
archive.md        archive extraction, path safety, transactional extraction
fetch.md          HTTP/materialization primitives and advanced fetcher maturity
fs.md             atomic filesystem primitives and workspace/transaction basics
install.md        install/activation/rollback/uninstall workflow boundary
resource.md       resource identity/version/trust/behavior semantics
serialization.md  owner-local JSON decision after serde-backend removal
source.md         source planning and candidate expansion
stabilization.md  cross-cutting stabilization decisions
state.md          lifecycle state, inspection, repair, retention, lock export
store.md          artifact storage/provenance/metadata/prune planning
verify.md         streaming verification primitives
version.md        version parsing/selection primitives
```

Overall, the docs now mostly reflect the post-fold world. The major stale signals are small and not architectural blockers:

- `docs/architecture/fs.md` still says “future higher-level crates such as `pulith-store` and `pulith-install`” even though those crates now exist.
- `docs/architecture/serialization.md` intentionally mentions the former `pulith-serde-backend` as historical context.
- `docs/architecture.md` still references `docs/roadmap.md`, which is currently deleted in the working tree. That reference should be fixed in a later doc cleanup slice.

## Current top-down architecture reading

Pulith’s current architecture is a mechanism-first resource-manager toolkit, not a package manager and not a hidden orchestrator.

The canonical pipeline remains:

```text
resource -> source plan -> fetch -> verify -> extract/register -> install -> activate -> state
```

But a more precise top-down reading is:

```text
Intent layer:
  resource identity, version intent, trust/integrity requirement, behavior axes

Source layer:
  declared origins and planned candidates, without transfer policy

Materialization layer:
  fetch remote/local bytes, verify integrity, extract contained trees

Memory layer:
  store materialized artifacts and provenance; persist lifecycle state and lock/export views

Mutation layer:
  install roots, activation targets, rollback/backup/uninstall receipts

Adapter layer:
  examples and future managers compose policies outside the core crates
```

This is healthier than evaluating by crate count or line count.

## Architecture invariants that should now be explicit

### 1. Resource semantics before mechanics

`pulith-resource` and `pulith-version` define user intent:

- identity;
- version/selection semantics;
- locator/trust/digest semantics;
- behavior axes.

They should not fetch, cache, install, or persist lifecycle state.

### 2. Source planning is not transfer execution

`pulith-source` describes source candidates. `pulith-fetch` executes transfer.

Healthy boundary:

```text
pulith-resource -> pulith-source -> pulith-fetch
```

Risk to watch:

- `pulith-fetch` already exposes source options and multi-source strategy vocabulary.
- `pulith-source` also owns source planning strategy vocabulary.
- The boundary is probably correct, but the exact behavior split needs a focused source/fetch evaluation before any module-level refactor.

### 3. Verification is an integrity primitive, not fetch codec clutter

`pulith-verify` has an earned role if it stays the canonical streaming verification primitive.

Risk to watch:

- `pulith-fetch/codec` also contains checksum/signature/decompression helpers.
- The next high-value top-down behavior evaluation may be: where does “verification” end and “fetch codec/decompression/signature convenience” begin?

### 4. Archive extraction is a security boundary

`pulith-archive` should remain separate because path sanitization, symlink escape prevention, and extraction limits are durable security semantics.

Do not fold this crate for line-count reasons.

### 5. Store and state are adjacent but not the same product

`pulith-store` owns artifact memory/provenance.

`pulith-state` owns lifecycle memory, activation facts, inspection, repair, retention, and lock export views.

This split is still healthy. The top-down issue is not “merge store and state”; it is ensuring cleanup/retention/repair semantics have one owner and do not duplicate report payloads.

### 6. Install owns mutation workflow, not source policy

`pulith-install` should keep install-root mutation, activation, rollback, backup/restore, and uninstall flows.

It should not become:

- source resolver;
- dependency solver;
- fetch retry policy owner;
- store retention policy owner;
- external service/registry/env side-effect orchestrator.

The file is large, but decomposition should wait until the architecture says which behavior slice needs clarity.

### 7. Platform behavior is now owner-local

After removing `pulith-platform`, platform details should appear where contracts require them:

- filesystem behavior in `pulith-fs`;
- archive path semantics in `pulith-archive`;
- Windows symlink privilege and activation target behavior in `pulith-install`;
- user-level manager policy outside core crates.

Do not recreate a platform helper crate unless an actual cross-owner Pulith-specific contract appears.

### 8. Serialization is an owner boundary

The current JSON decision is clean:

- state owns state snapshot JSON and lock export JSON;
- store owns metadata JSON;
- install owns backup/restore state payload JSON where it writes typed state snapshots.

No public backend trait until at least two owner crates need the same real non-JSON backend.

## Brainstorm: current architecture options

### Option A — Continue direct internal decomposition now

Shape:

- Split `pulith-install/src/lib.rs` into modules because it is 2496 LOC.
- Then split `pulith-state/src/lib.rs`, `pulith-store/src/lib.rs`, etc.

Pros:

- Reduces immediate large-file pain.
- Low external API churn if crate-root re-exports are preserved.

Cons:

- Violates the user’s current correction: this starts from file size, not architecture.
- Can ossify current accidental behavior groupings.
- Risks moving code without answering whether the behavior surface itself is right.

Evaluation: pause. Keep as a later implementation slice, not the next step.

### Option B — Write a top-down behavior architecture map before more code

Shape:

- Update `docs/architecture.md` into the canonical behavior-layer map.
- For each behavior layer, name owner, inputs, outputs, side effects, forbidden dependencies, and open pressure points.
- Then choose one behavior slice for a focused plan.

Pros:

- Aligns with user correction.
- Prevents line-count/aesthetic refactors.
- Creates a reusable decision frame for every later crate/module move.

Cons:

- Slower than immediate code movement.
- Requires discipline not to turn architecture docs into implementation plans.

Evaluation: recommended next step.

### Option C — Focus on source/fetch/verify as the next behavior slice

Shape:

Evaluate the materialization path:

```text
Resource locator / SourceSpec / PlannedSources
  -> Fetcher / MultiSourceFetcher
  -> verify receipt / checksum/signature/decompression helpers
  -> store provenance absorption
```

Questions:

- Does `pulith-source` own all planning strategy vocabulary, or does `pulith-fetch` own some execution strategy vocabulary?
- Are `DownloadSource`, `MultiSourceOptions`, and `SourceSelectionStrategy` in fetch policy-like duplicates of source planning, or are they execution knobs?
- Should checksum/signature helpers in fetch collapse toward `pulith-verify`, or are they fetch-specific receipt adapters?
- What exact receipt/provenance fields cross from fetch into store?

Pros:

- High value: `pulith-fetch` is largest and most mature-risk-heavy crate.
- Addresses behavior boundaries, not file boundaries.
- Likely reveals real public API simplifications.

Cons:

- Harder than install decomposition.
- May require reading many fetch modules and tests.

Evaluation: best first behavior-specific evaluation after the top-down architecture map.

### Option D — Focus on lifecycle cleanup semantics next

Shape:

Evaluate:

```text
store orphan metadata
state store-key references
state retention policy/plans
install uninstall backup/restore rollback receipts
lifecycle receipt envelope
```

Questions:

- Which receipt is canonical for lifecycle operations?
- Which cleanup decision belongs in state vs store vs install?
- Are backup/restore payloads duplicating state-owned snapshots, or are they install operation receipts only?
- Should repair/retention/ownership reports have a shared preview/apply pattern?

Pros:

- Directly targets current duplication pressure in `install.md`, `state.md`, `store.md`.
- Can produce a clean behavior API graph before any module split.

Cons:

- May be more subtle than source/fetch because many semantics are already partly right.
- Risk of inventing a top-level orchestrator if not kept mechanism-first.

Evaluation: strong candidate, but probably second after source/fetch/verify because materialization is currently the wider dependency fan-in path.

### Option E — Rebuild docs spine: architecture first, graph/reference next

Shape:

- Keep `docs/architecture.md` as top-down behavior authority.
- Keep `docs/architecture/*.md` as per-crate boundary docs.
- Add or update a `docs/reference.md` / graph-style doc for current crate API surfaces and dependency graph.
- Move stale roadmap/development status out of architecture.

Pros:

- Makes future refactors easier to evaluate.
- Keeps architecture docs from becoming implementation checklists.

Cons:

- Documentation work only; no code simplification yet.

Evaluation: useful as part of Option B, not a separate long phase.

## Recommended next sequence

### Phase A — DDD concept model and architecture authority cleanup

Update docs only.

The important shift is from abstract layers to domain concepts:

```text
Resource Intent -> Source Offer -> Materialized Evidence -> Artifact Memory -> Mutation Workflow -> Lifecycle State -> Inspection/Repair
```

Phase A should make `docs/architecture.md` the canonical domain concept map:

1. Start from Pulith's goal: mechanism-first resource-manager construction, not a package manager or hidden orchestrator.
2. Define domain concepts before crate/module names.
3. Map each concept to its current owner crate only after the concept is clear.
4. State each concept's responsibilities, non-responsibilities, side effects, and forbidden ownership.
5. Remove stale references such as deleted `docs/roadmap.md`.
6. Ensure crate-level architecture docs remain boundary docs, not sprint/status reports.
7. Do not touch Rust code.

The concept map is now:

| Domain concept | Crate owner | Role |
| --- | --- | --- |
| Resource Intent | `pulith-version`, `pulith-resource` | user/manager declaration of what should exist |
| Source Offer | `pulith-source` | where it may come from; candidate planning without transfer execution |
| Materialized Evidence | `pulith-fetch`, `pulith-verify`, `pulith-archive`, `pulith-fs` | bytes/trees obtained, checked, and safely placed |
| Artifact Memory | `pulith-store` | local artifact/extract metadata and provenance |
| Lifecycle State | `pulith-state` | lifecycle facts, activation facts, inspection/repair/retention, lock export views |
| Mutation Workflow | `pulith-install` | install roots, activation targets, rollback/backup/uninstall operation boundaries |
| Caller Policy | examples/future managers | dependency solving, fallback choice, cleanup timing, product policy |

This DDD model is the gate before any direct decomposition. A large file is not a refactor target until its domain concept owner and behavior boundary are accepted.

Verification:

```bash
git diff --check -- docs
python - <<'PY'
from pathlib import Path
text = Path('docs/architecture.md').read_text(encoding='utf-8')
for phrase in [
    'Domain-Driven Concept Model',
    'Resource Intent',
    'Source Offer',
    'Materialized Evidence',
    'Artifact Memory',
    'Lifecycle State',
    'Mutation Workflow',
    'Caller Policy',
]:
    assert phrase in text, phrase
assert 'docs/roadmap.md' not in text
print('architecture DDD concept map exists and avoids deleted roadmap reference')
PY
```

### Phase B — Source/fetch/verify behavior evaluation

Write a focused report before code:

```text
docs/report/source-fetch-verify-boundary-evaluation.md
```

It should read live code and tests for:

- `pulith-source` public types and planning methods;
- `pulith-fetch` source/multi-source/retry/codec/receipt surfaces;
- `pulith-verify` streaming verification API;
- store provenance absorption from fetch receipts.

Output should be:

- current API/dependency map;
- duplicate/overlapping concepts;
- which behavior owner is canonical;
- proposed code slice, if any;
- exact verification gates.

### Phase C — Only then choose implementation slice

Possible outcomes after Phase B:

1. No code change; just clarify docs.
2. Move duplicate verification helpers toward `pulith-verify` or mark fetch-local helpers as receipt adapters.
3. Simplify `pulith-fetch` source strategy vocabulary if it duplicates `pulith-source` planning.
4. Tighten store provenance absorption from fetch/archive evidence.

No implementation should begin before that report is accepted.

## Current specific crate/behavior priorities

### Highest-priority evaluation: source/fetch/verify/store materialization path

Reason:

- `pulith-fetch` is the largest crate by far.
- It depends on `pulith-resource`, `pulith-source`, `pulith-verify`, and `pulith-fs`.
- It is the place most likely to mix planning, transfer execution, verification helpers, retry policy, source selection, and receipt shape.

This should be evaluated top-down before decomposing fetch modules or touching install.

### Second-priority evaluation: lifecycle cleanup/recovery path

Reason:

- `pulith-install`, `pulith-state`, and `pulith-store` all participate.
- Current docs already mention receipt/state duplication pressure.
- Needs architecture clarification before moving install modules.

### Later implementation: internal module decomposition

Only after a behavior evaluation says the owner boundary is right:

- split `pulith-install` if the install behavior API is accepted;
- split `pulith-state` if lifecycle/report/repair ownership is accepted;
- split `pulith-store` if provenance/metadata/prune ownership is accepted;
- split `pulith-resource` only if behavior axes need clearer submodules.

## Anti-goals

- Do not start module decomposition because of LOC alone.
- Do not recreate deleted satellite crates.
- Do not introduce a top-level Pulith orchestrator crate.
- Do not hide caller policy in primitive crates.
- Do not let architecture docs carry phase/task checklists as if they were durable design.
- Do not preserve compatibility aliases for deleted crate paths.

## Concrete next ask for approval

Recommended next action:

```text
Phase A: update architecture docs only, especially docs/architecture.md,
so the top-down behavior model becomes canonical and the direct-decomposition plan is explicitly deferred.
```

After that, write the focused source/fetch/verify boundary report before any Rust changes.
