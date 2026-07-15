# Pulith Architecture

## Authority

This document describes the current source tree. Files in `docs/report/` are historical design and execution evidence; they do not override this contract.

Pulith is one feature-gated crate:

```text
Cargo.toml
crates/pulith/
  Cargo.toml
  src/
    application.rs
    behavior.rs
    evidence.rs
    error.rs
    local.rs
    hash.rs
    archive.rs
    net.rs
```

No legacy `pulith-*` side crate is an active source owner.

## Design law

Pulith models artifact work as typed morphisms:

```text
WithSource<I, S>
  -> Chosen<I, S>
  -> Acquired<I, M, E>
  -> Verified<I, M, E>
  -> Prepared<I, P, E>
  -> Applied<I, R, E>
  -> Remembered<I, R, E>
```

The transition traits are:

```text
SelectNode
AcquireNode
VerifyNode
PrepareNode
ApplyNode
RememberNode
```

Every transition declares associated:

```text
Need
Evidence
Error
Output
```

This keeps behavior identity in Rust types and feature gates rather than strings, registries, factories, or a global application context.

## Ownership

### `application`

Owns the semantic nouns used by behavior:

```text
Intent
Item
LocalTarget
Create
Replace
CreateOrReplace
Forget
Receipt
```

It does not execute effects.

### `behavior`

Owns transition traits and typed state nodes. It does not choose concrete policy or resources.

### `evidence`

Owns `EvidenceChain` and transition evidence composition. Evidence is carried forward instead of reconstructed from paths after the fact.

### `local`

Owns local filesystem mechanisms:

- select a local path;
- acquire file/directory material;
- identity prepare/verify where explicitly chosen;
- stage file and directory copies before publication;
- apply create/replace/create-or-replace/forget;
- record placement statistics and receipts.

Directory replacement renames the old target to a backup, publishes the staged tree, and then cleans the backup best-effort. Once publication succeeds, cleanup failure is not reported as an apply failure. Parent directories are assumed trusted against adversarial concurrent mutation.

### `hash`

Owns typed digest verification. `blake3` and `sha2` enable concrete algorithms; bare `hash` provides shared vocabulary only.

### `archive`

Owns typed ZIP/TAR preparation and extraction evidence. The module is present only when `zip` or `tar` is enabled. Compression codecs extend `tar`.

Order of concern:

```text
verified local file
-> clear exclusive extraction root
-> validate/sanitize each archive path
-> enforce entry and byte limits
-> reject unsupported links
-> materialize archive tree
-> emit ArchiveEvidence
```

`ExistingExtractRoot` is an exclusive destructive capability. The caller must never point it at shared or independently managed content.

### `net`

Owns planned HTTP acquire behavior for both blocking `ureq` and async `reqwest` resources.

```text
attempt admission
-> outbound request
-> status/retry/resume decision
-> decoded response chunk
-> max_bytes guard
-> byte pacing
-> staged write
-> stage finalization
-> destination persist
```

Two shared controls are orthogonal:

```text
RateAdmission
  unit: outbound request attempts
  boundary: before send
  evidence: admission_wait

ByteRatePacer
  unit: decoded response-body bytes
  boundary: after max guard, before staged write
  evidence: pacing_wait
```

The same governor decision model backs sync and async implementations; only the wait effect differs. Attempt rate is not maximum in-flight concurrency. Body pacing does not claim control over socket reads, TLS records, client buffering, or HTTP/2/3 flow control.

## Feature graph

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
gzip -> tar
xz -> tar
zstd -> tar
```

Features must correspond to real source-owned behavior. Empty dependency-only capability features are rejected. This is why `object`, `archive`, `compress`, `fs-extra`, and `json` are absent.

## Error ownership

`PulithError` owns cross-module composition errors. `AcquireError` remains net-owned because it carries complete attempt, retry, resume, admission, and pacing evidence. `PulithError::NetAcquire` boxes it to keep unrelated result types compact.

Targeted `result_large_err` allowances exist only on APIs that intentionally return the evidence-rich net error directly.

## Safety model

Guaranteed by covered behavior:

- destination remains untouched on pre-publication network failure;
- status, max-byte, admission, pacing, resume, and staging failures are explicit;
- archive parent traversal and symlink entries are rejected;
- local source/target identity and directory-cycle conflicts are rejected;
- archive and local effects return typed evidence.

Not guaranteed:

- hostile concurrent filesystem safety under attacker-writable parents;
- atomic replacement across filesystems or every operating-system failure mode;
- raw network bandwidth control;
- dependency solving or package-manager policy;
- durable store/state/install/activation workflows;
- transparent compatibility with deleted side-crate APIs.

## Quality gates

Before a behavior commit:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
cargo package -p pulith --allow-dirty --no-verify
```

Also check every smallest public feature combination with `--no-default-features`. Timing tests assert semantic evidence such as a non-zero wait rather than exact wall-clock duration.

## Next design questions

The next work must harden existing behavior before restoring removed domains:

1. enforce archive byte limits using observed copied bytes, not only declared entry sizes;
2. make backup-cleanup residue observable without reporting false apply failure;
3. split `net.rs` internally only where typed behavior boundaries remain intact;
4. decide whether a separate concurrency permit with an end-of-attempt lifetime is required;
5. restore store/state/install/object behavior only when a real caller and complete typed contract exist.
