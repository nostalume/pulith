# Pulith Design and Execution Reports

This directory is an append-only historical record of the analysis, plans, experiments, and execution slices that led to the current single-crate design.

## Authority

These reports are **not authoritative API documentation**. Current behavior is defined by:

1. source and tests in `crates/pulith`;
2. `docs/architecture.md`;
3. `README.md` and `docs/AGENT.md`.

A report may describe a superseded crate layout, feature, type name, test count, dependency version, temporary path, or next step. Read its title and internal status in chronological context. Do not execute historical commands blindly.

## Reading order

### Reduction and single-crate decision

Start with:

- `top-down-architecture-reduction.md`
- `pulith-single-crate-migration-plan.md`
- `single-crate-feature-composable-traits-redesign.md`
- `pulith-composable-tree-behavior-design.md`
- `pulith-behavior-morphism-spec.md`

### Typed local, hash, and archive behavior

Then read:

- `pulith-file-io-dependency-and-behavior-design.md`
- `pulith-local-file-apply-hardening-execution-report.md`
- `pulith-typed-archive-prepare-migration-plan.md`
- `pulith-zip-prepare-execution-report.md`
- `pulith-tar-prepare-execution-report.md`
- `pulith-compressed-tar-prepare-execution-report.md`

### Network behavior

The network sequence is captured across:

- `pulith-net-acquire-execution-detail-plan.md`
- `pulith-net-acquire-sync-ureq-execution-report.md`
- `pulith-reqwest-tokio-backed-acquire-execution-report.md`
- `pulith-net-retry-execution-report.md`
- `pulith-resume-range-execution-report.md`
- `pulith-net-owned-error-execution-report.md`
- `pulith-net-prefix-removal-byte-pacing-execution-report.md`
- `pulith-net-request-admission-execution-report.md`

### Capability pruning

- `pulith-crate-prune-net-and-file-io-assessment.md`
- `pulith-empty-object-feature-prune-report.md`

## Historical-path note

Some plans and execution reports mention machine-specific temporary verification locations. Those strings record the verification convention used during execution; they are not repository requirements or portable commands.

## Preservation rule

Preserve report text as historical evidence. Correct current guidance in the authoritative documents rather than silently rewriting old decisions to look current.
