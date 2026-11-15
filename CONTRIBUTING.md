# Contributing

Pulith welcomes focused fixes, documentation, tests, and behavior additions that preserve its
library boundaries. Read [`docs/AGENTS.md`](docs/AGENTS.md) for the project goal, principles, and
technical stack, and [`docs/architecture.md`](docs/architecture.md) for current ownership.

## Development setup

The crate supports Rust 1.88 and newer. The repository selects Rust 1.99 with rustfmt and Clippy
through `rust-toolchain.toml`; rustup installs that development toolchain on first use. Clone the
canonical repository:

```text
git clone https://github.com/nostalume/pulith.git
cd pulith
```

The repository honors the Cargo source configuration of your environment. Dependency downloads may
therefore fail when a configured mirror is unavailable; that is distinct from a Pulith test failure.

## Checks

Run the focused test for the area you changed, then the canonical gates:

```text
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo doc --locked --all-features --no-deps
cargo deny check advisories bans sources
```

Tests are self-contained. A test that needs a hostile network condition builds its fixture in
process (see `tests/ambient_proxy.rs`); no external service, CI-only environment setup, or host
state is required. `cargo test --locked --all-features` is the whole verification path.

The process-tree tests share Windows job-object and child-process resources; run the full suite
serially on Windows to avoid scheduler-dependent contention:

```text
cargo test --locked --all-features -- --test-threads=1
```

Public APIs must also pass strict documentation linting:

```text
RUSTDOCFLAGS="-D warnings -D missing-docs" cargo doc --locked --all-features --no-deps
```

For feature work, check the smallest feature combination that owns the behavior. The CI workflow is
the authoritative matrix.

## Windows private-file acceptance

Use two ordinary Windows users with distinct SIDs and an empty local NTFS fixture directory.
Give the publishing user write access and both users read access to the fixture and inherited
read access to its ordinary files. The reader must open the public control before each denied
open is counted. Set `PULITH_PRIVATE_TEST_ROOT` to its full path. Run the following twice,
with `PULITH_PRIVATE_TEST_PHASE=create` as the publisher and `PULITH_PRIVATE_TEST_PHASE=read`
as the reader. The test binaries can also be run directly under the corresponding account.

```text
cargo test --locked --all-features --test local windows_private_publication_two_accounts -- --ignored --nocapture --test-threads=1
cargo test --locked --all-features --lib windows_private_ -- --ignored --nocapture --test-threads=1
```

Cleanup uses phase `remove` as the publisher, with the two commands in reverse order. Repeat
with a restrictive inherited ACL that still lets the reader open the public control. To check
an unsupported volume, set `PULITH_TEST_UNSUPPORTED_VOLUME` to a writable directory on that
volume and run the following. It creates a disposable control and exercises the actual publish
operation.

```text
cargo test --locked --all-features --test local windows_private_volume_refuses_before_creating_parent -- --ignored --nocapture
```

Record the control read, restricted file opens, and cleanup state for each account. A run of
`create` alone records publisher behavior. Use the second account's local token for the reader
phase. Keep file publication and editor-session results separate when recording support.

## Change requirements

- Keep observable behavior covered by contract tests.
- Put example tests under `tests/examples/`; example source may reference those modules.
- Prefer evidence assertions over exact timing.
- Do not add a CI-only verification path that `cargo test` cannot reproduce locally.
- Document public contracts, errors, safety boundaries, and feature requirements.
- Do not combine unrelated formatting or refactoring with a behavioral fix.
- Do not commit build output, credentials, local agent state, or temporary verification scripts.

Before committing, inspect `git diff`, run `git diff --check`, and use the commit form
`[prefix]: concise content`, such as `[fix]: reject escaped archive paths`.

## Pull requests

Explain the problem, contract change, and evidence. Identify any platform behavior that was not
observed. A change to Windows or Linux semantics requires evidence on that platform; macOS remains
best-effort until promoted by project policy.
