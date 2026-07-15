# Pulith Engineering Guide

This guide is subordinate to [`architecture.md`](architecture.md).

## Repository shape

The active Rust workspace contains exactly one package:

```text
crates/pulith
```

Do not reintroduce deleted side crates, compatibility shims, registries, factories, middleware layers, or empty feature flags without a demonstrated caller and a complete behavior contract.

## Design rules

- Design first; code only after the behavior boundary and evidence law are explicit.
- Preserve the typed transition chain and associated `Need`/`Evidence`/`Error`/`Output` types.
- Keep caller policy outside mechanisms.
- Prefer enum/ZST/associated-type identity over strings.
- Keep sync and async semantics aligned; only execution modality may differ.
- Keep request admission, concurrency, and byte pacing as distinct resources.
- Delete speculative abstraction rather than preserving compatibility shells.
- Document destructive capability types and non-guarantees honestly.

## Feature rules

Every public feature must:

1. enable real behavior or shared vocabulary;
2. compile in its smallest supported combination;
3. own only the dependencies it uses;
4. avoid parallel aliases for the same capability.

Current graph:

```text
default = local + sync
runtime-tokio -> async
net -> local
ureq -> net + sync
reqwest -> net + runtime-tokio
blake3 -> hash
sha2 -> hash
zip -> local
tar -> local
gzip/xz/zstd -> tar
```

## Error rules

- Use concrete owner-local error enums.
- Preserve source errors.
- Carry evidence needed to explain retries, resume, limits, and partial behavior.
- Box a large nested error at an outer boundary rather than inflating every unrelated result.
- Any lint allowance must be local and include a reason.

## Filesystem rules

- Use `Path`/`PathBuf`.
- Stage before publication.
- Reject same-file, directory-cycle, traversal, and symlink hazards where the contract promises it.
- Never report failure after successful publication merely because best-effort cleanup failed.
- Treat `ExistingExtractRoot` as exclusive and destructive.
- State trusted-parent and cross-filesystem limitations explicitly.

## Network rules

- Admit each outbound attempt separately, including retries.
- Enforce `max_bytes` before pacing and staged writes.
- Persist only after body copy and stage finalization succeed.
- Record attempt outcome, resume evidence, admission wait, and pacing wait.
- Do not call attempt rate concurrency.
- Do not describe decoded-body pacing as socket-level bandwidth control.

## Required local gates

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
cargo package -p pulith --allow-dirty --no-verify
```

Run smallest-combination checks for every changed feature, for example:

```bash
cargo check -p pulith --no-default-features --features local
cargo check -p pulith --no-default-features --features ureq
cargo check -p pulith --no-default-features --features reqwest
cargo check -p pulith --no-default-features --features blake3
cargo check -p pulith --no-default-features --features sha2
cargo check -p pulith --no-default-features --features zip
cargo check -p pulith --no-default-features --features tar
cargo check -p pulith --no-default-features --features gzip
cargo check -p pulith --no-default-features --features xz
cargo check -p pulith --no-default-features --features zstd
```

Behavior changes require contract-oriented tests. Avoid exact timing assertions when evidence can express the law.

## Documentation

- `docs/architecture.md` is authoritative.
- `README.md` is the user-facing summary.
- `docs/publish/` owns current release gates.
- `docs/report/` is historical analysis and execution evidence.
- Historical benchmark notes remain historical; do not rewrite them as current commands.

## Commit discipline

- Keep code, authoritative docs, and historical reports in separate commits when possible.
- Inspect staged diffs and run `git diff --cached --check` before committing.
- Do not commit `.hermes/`, temporary verification scripts, `target/`, or credentials.
- Do not commit or push unless explicitly requested.
