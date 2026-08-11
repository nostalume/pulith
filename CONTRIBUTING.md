# Contributing

Pulith welcomes focused fixes, documentation, tests, and behavior additions that preserve its
library boundaries. Read [`docs/AGENTS.md`](docs/AGENTS.md) for the project goal, principles, and
technical stack, and [`docs/architecture.md`](docs/architecture.md) for current ownership.

## Development setup

Install Rust 1.88 or newer with rustfmt and Clippy. Clone the canonical repository:

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

Public APIs must also pass strict documentation linting:

```text
RUSTDOCFLAGS="-D warnings -D missing-docs" cargo doc --locked --all-features --no-deps
```

For feature work, check the smallest feature combination that owns the behavior. The CI workflow is
the authoritative matrix.

## Change requirements

- Keep observable behavior covered by contract tests.
- Put example tests under `tests/examples/`; example source may reference those modules.
- Prefer evidence assertions over exact timing.
- Document public contracts, errors, safety boundaries, and feature requirements.
- Do not combine unrelated formatting or refactoring with a behavioral fix.
- Do not commit build output, credentials, local agent state, or temporary verification scripts.

Before committing, inspect `git diff`, run `git diff --check`, and use the commit form
`[prefix]: concise content`, such as `[fix]: reject escaped archive paths`.

## Pull requests

Explain the problem, contract change, and evidence. Identify any platform behavior that was not
observed. A change to Windows or Linux semantics requires evidence on that platform; macOS remains
best-effort until promoted by project policy.
