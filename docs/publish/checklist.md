# Publish Checklist

Use this checklist for the consolidated `pulith` crate only.

## Source gate

- [x] workspace contains only `crates/pulith`
- [x] package license matches repository `LICENSE` (`Apache-2.0`)
- [x] package description and repository metadata are present
- [x] dependency-only placeholder features are removed
- [ ] public API and SemVer review is complete
- [ ] release notes explain retirement of historical side crates

## Quality gate

- [ ] clean-commit `cargo fmt --all --check`
- [ ] clean-commit `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] clean-commit `cargo test --workspace --all-features`
- [ ] clean-commit rustdoc with `RUSTDOCFLAGS=-D warnings`
- [ ] every smallest public feature combination compiles
- [ ] Linux, Windows, and macOS CI pass

## Security and dependency gate

- [ ] `cargo audit`
- [ ] configured `cargo deny` checks
- [ ] duplicate dependency review
- [ ] archive observed-byte limit hardening is resolved or accepted as a documented release limitation
- [ ] filesystem trusted-parent/cleanup limits are reviewed

## Packaging gate

- [ ] `cargo package -p pulith --no-verify` from a clean commit
- [ ] `cargo publish -p pulith --dry-run --registry crates-io`
- [ ] packaged file list is reviewed
- [ ] crates.io name/version availability is confirmed immediately before release

## Decision

- [ ] readiness matrix says `GO`
- [ ] release commit/tag is identified
- [ ] publication is explicitly authorized

Historical side-crate publication events are audit history, not reusable completion evidence for this crate.
