# Pulith engineering reference

This document is a concise reference for contributors. Development workflow belongs in
[`CONTRIBUTING.md`](../CONTRIBUTING.md); public behavior belongs in rustdoc.

## Project goal

Pulith provides composable, typed Rust behaviors for managing external resources. It owns narrow
resource operations and their evidence while callers own application identity, desired state,
trust and admission policy, orchestration, retention, and rollback.

The crate must remain useful as a library rather than becoming one package manager, deployment
system, service ecosystem, or global workflow engine.

## Principles

- Keep behavior law, resource semantics, and concrete adapter separate.
- Make effects explicit; inspection and reconciliation remain non-mutating.
- Return typed evidence from the operation that observed or changed the resource.
- Keep sync and async semantics equal; only execution modality may differ.
- Preserve private staging custody until an explicit publication boundary.
- Admit filesystem paths, URLs, environment entries, and process inputs before effects.
- Reject traversal, collision, unsafe-link, unbounded-resource, and overwrite hazards where the API
  promises protection.
- Prefer small behavior methods and restricted values over registries, factories, global context,
  wrappers without additional law, or vague workflow objects.
- Use mature implementations for protocols, compression, hashes, and platform APIs; Pulith owns
  composition and safety policy around them.
- Treat Linux and Windows as required semantic peers. Platform adapters may differ internally but
  may not change the declared meaning.

## Technical stack

| Role | Technology |
| --- | --- |
| Language | Rust 2024 |
| Development toolchain | Rust 1.98 |
| MSRV | Rust 1.88 |
| Synchronous HTTP | ureq with rustls |
| Asynchronous HTTP | reqwest with rustls on Tokio |
| Admission and pacing | governor |
| Local staging and traversal | tempfile, walkdir, same-file |
| Archives and codecs | zip, tar, flate2, xz2, zstd |
| Digests | blake3, sha2, hex |
| URL and HTTP dates | url, httpdate |
| Platform APIs | rustix on Unix, windows-sys on Windows |
| Serialization | serde |
| Automation | GitHub Actions, rustfmt, Clippy, rustdoc, cargo-deny |
