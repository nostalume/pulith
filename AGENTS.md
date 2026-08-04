# Pulith Engineering Guide

## Purpose

Pulith is one feature-gated Rust crate ecosystem for composing typed external-resource management
behaviors. Artifact materialization is one concrete behavior family:

```text
Materialize -> Acquired -> Applied
                        -> Verified -> Applied
                        -> Prepared -> Applied
                        -> Verified -> Prepared -> Applied
Forget -----------------------------> Applied
LocalTarget -> Inspected -> Reconciled
RemoteUrl  --> Inspected
```

Each behavior passes policy `Need` as a trait type parameter where required and declares associated
`Error` and `Output` contracts. The typed `Output` carries the behavior's evidence. Callers compose
policy and effects; there is no global `App`, `Context`, registry, or factory.

Keep three axes orthogonal:

```text
behavior law != resource-specific semantics != concrete adapter
```

Callers own desired state, application identity, trust/admission policy, durable aggregates,
orchestration, and rollback/retention policy. Do not promote one package-manager, filesystem,
database, trust, or deployment implementation into Pulith's universal domain model.

## Tech stack

| Role | Technology |
| --- | --- |
| Language | Rust 2024 |
| MSRV | Rust 1.88 |
| Async runtime | Tokio |
| Sync HTTP | ureq + rustls |
| Async HTTP | reqwest + rustls |
| Admission/pacing | governor |
| Local staging | tempfile |
| Tree walking/same-file | walkdir, same-file |
| Archives | zip, tar |
| Codecs | flate2, xz2, zstd |
| Hashes | blake3, sha2, hex |
| URL/HTTP dates | url, httpdate |
| CI/security | rustfmt, clippy, rustdoc, cargo-deny |

## Feature graph

```text
default = local
net
http-sync -> net + local
http-async -> net + local + tokio
blake3 -> hash
sha2 -> hash
zip -> local
tar -> local
gzip/xz/zstd -> tar
```

Every public feature must enable real behavior or shared vocabulary, compile in its smallest supported combination, and own only dependencies it uses. Do not add empty compatibility features.

## Design rules

- Design behavior and evidence laws before code.
- For every public behavior, document input, `Need` where required, output-carried `Evidence`,
  `Error`, `Output`, effect/failure law, authority owner, resource semantics, and concrete adapter
  or caller.
- Preserve the typed transition chain, generic `Need`, and associated `Error`/`Output` contracts.
- Treat state types as composition vocabulary, not proof that every transition has a built-in
  concrete behavior; document implemented and caller-owned boundaries explicitly.
- Target-only operations such as `Forget` branch directly to apply and must not require
  a synthetic source, acquisition, verification, or preparation path.
- Prefer enum/ZST/associated-type identity over strings.
- Keep sync and async semantics aligned; only execution modality may differ.
- Keep request admission, concurrency, and decoded-body pacing distinct.
- Use mature crates for codecs and container parsing; Pulith owns policy, evidence, staging, and composition.
- Delete speculative abstractions and compatibility shells.
- Keep observation read-only, reconciliation non-mutating, and repair as a separate explicit behavior.
- Keep metadata-only `LocalInspect` O(metadata); hash-backed exact local inspection is opt-in at
  `local + blake3` or `local + sha2`, reads only regular-file handles, and counts bytes in the digest
  loop. It protects only the final component, assumes trusted parents, and is not an atomic snapshot.
- Classify Windows symbolic links separately from other reparse points; never hash either kind.
- Treat canonical evidence as the caller-selected adapter's attestation, not provenance,
  authorization, or an unforgeable capability. State records are open for external composition;
  invariant-bearing resource outputs may restrict construction separately.
- Keep validated values and invariant-bearing resource identities crate-constructed when built-in
  behavior relies on them; expose only the read access callers need. Do not close requests,
  policies, canonical records, or evidence merely to simulate authenticity.
- Do not treat evidence automatically as a domain event; introduce aggregates and DDD repositories
  only for demonstrated durable consistency boundaries.
- No registries, factories, middleware layers, or global singletons without a demonstrated caller.

## Filesystem and archive invariants

- Stage before publishing a final destination.
- For local regular files, `MaterializeMode::CreateNew` means expected predecessor `Missing` and
  `persist_noclobber` is the authoritative execution-time commit check. `AlreadyExists` is
  `ApplyWouldOverwrite`, not generic I/O; conflict must not change the winner target.
- Do not generalize that conditional-file law to directories, replacement modes, `Forget`, or
  digest-based compare-and-swap.
- `LocalMaterial::File` and `Directory` are caller-owned; `StagedFile` owns a `TempPath` and removes
  it on drop. Do not make staged custody cloneable.
- Reject same-file, directory-cycle, traversal, and symlink hazards where promised.
- `ExtractWorkspace` is exclusive, destructive scratch; it is not a final destination.
- `ArchiveTree` is crate-constructed and exposes its prepared root by shared reference; callers
  cannot fabricate one from an arbitrary path or write the root field directly. Open canonical
  records may still replace whole resource values; this is not a permanent input/evidence binding,
  immutable scratch, or concurrency-safe filesystem capability.
- Archive symlinks, hardlinks, devices, and unsafe paths are rejected by default.
- Archive path collisions use portable case-folded identity rather than the host volume's case-sensitivity.
- Archive entry/total limits are enforced against observed materialized bytes, not metadata alone; decoded-container limits also bound TAR headers, padding, extensions, and stripped entries.
- Extraction failures surface cleanup failures instead of silently leaving a contaminated `ExtractWorkspace`.
- Never implement ZIP/TAR/DEFLATE/gzip/xz/zstd algorithms in Pulith.
- Never report failure after successful publication merely because best-effort cleanup failed.
- Local path safety assumes trusted parent directories; Pulith is not a hostile concurrent-filesystem sandbox.

Required runtime evidence covers Windows and Linux. macOS is best-effort and unverified; it is not a
phase-admission or completion gate.

## Network invariants

- Admit each outbound attempt separately, including retries.
- HTTP inspection issues HEAD only; it never falls back to GET, copies a body, or publishes a destination.
- Every received final HTTP inspection status is an observation, including 4xx and exhausted 5xx.
- HTTP inspection records requested and final URLs; declared content length is metadata, not observed bytes or artifact identity.
- Enforce `max_bytes` before pacing and staged writes.
- HTTP Acquire returns adapter-owned staged material and never publishes `Materialize.target` or
  creates its parent. Only Apply publishes the final target.
- Resume partial paths remain caller-owned and are never removed implicitly.
- Record attempt outcome, resume, admission wait, and pacing wait.
- Attempt rate is not concurrency.
- Decoded-body pacing is not socket-level bandwidth control.

## Required gates

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps
cargo package --allow-dirty --no-verify
cargo deny check advisories bans sources
```

For a changed feature, also run its smallest combination:

```bash
cargo check --no-default-features --features local
cargo check --no-default-features --features net
cargo check --no-default-features --features http-sync
cargo check --no-default-features --features http-async
cargo check --no-default-features --features hash
cargo check --no-default-features --features blake3
cargo check --no-default-features --features sha2
cargo check --no-default-features --features zip
cargo check --no-default-features --features tar
cargo check --no-default-features --features gzip
cargo check --no-default-features --features xz
cargo check --no-default-features --features zstd
```

Behavior changes require contract tests. Prefer evidence assertions over exact timing.

## Repository discipline

- `README.md` is the user-facing summary.
- Crate and module rustdoc are the architecture/API authority.
- Git history is the historical design record; do not recreate `docs/report/`.
- Do not commit `.hermes/`, temporary verification scripts, `target/`, or credentials.
- Inspect staged diffs and run `git diff --cached --check` before committing.
- Do not push or rewrite history unless explicitly requested.
