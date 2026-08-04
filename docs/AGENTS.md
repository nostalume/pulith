# Pulith goal and technical stack

## Goal

Pulith is a feature-gated Rust crate ecosystem for composing typed external-resource management
behaviors. A behavior accepts caller-selected policy, performs one resource-specific effect or
observation, and returns typed output that carries its evidence.

Pulith supplies composable behavior laws and concrete adapters; callers retain desired state,
application identity, trust/admission policy, durable aggregates, orchestration, and recovery. It
does not provide a global application context, registry, factory, or universal resource lifecycle.

## Technical stack

| Role | Technology |
| --- | --- |
| Language / MSRV | Rust 2024 / Rust 1.88 |
| Async runtime | Tokio |
| Sync HTTP | ureq + rustls |
| Async HTTP | reqwest + rustls |
| Admission and pacing | governor |
| Local staging | tempfile |
| Tree walking / same-file | walkdir, same-file |
| Archives | zip, tar |
| Codecs | flate2, xz2, zstd |
| Hashes | blake3, sha2, hex |
| URL / HTTP dates | url, httpdate |
| Platform filesystem APIs | rustix (Unix), windows-sys (Windows) |
| CI and security | GitHub Actions, rustfmt, clippy, rustdoc, cargo-deny |

## Dependency policy

Run `cargo deny check advisories bans sources` as a separate dependency-policy gate. It audits the
resolved `Cargo.lock` against `deny.toml`; it does not compile or test Pulith behavior.

- `advisories` rejects dependencies with known security advisories.
- `bans` rejects wildcard dependency versions and reports duplicate versions under the configured policy.
- `sources` rejects unknown registries and unapproved Git sources.

This gate complements formatting, Clippy, rustdoc, and test evidence: those tools validate source
and behavior, while cargo-deny validates dependency security and supply-chain policy. Current
duplicate-transitive-version messages are warnings; advisory, banned-version, or source failures
block the gate.
