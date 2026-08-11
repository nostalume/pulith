# Architecture

Pulith separates an external-resource workflow into independently composable behavior contracts.
The caller chooses which contracts form a workflow and retains the resulting evidence.

## Layers

| Layer | Owner | Responsibility |
| --- | --- | --- |
| behavior | crate root traits | name one operation and its error/output contract |
| resource semantics | `local`, `net`, `archive`, `hash`, `process` | validate inputs and define resource-specific laws |
| adapter | feature-selected implementation | perform filesystem, HTTP, process, codec, or platform I/O |
| application | caller | desired state, identity, orchestration, trust, rollback, and retention |

No global object coordinates these layers. Behavior methods consume the restricted value that owns
the relevant input law.

## Dataflow

One common artifact flow is:

```text
source -> acquire -> optional verify -> optional archive preparation
       -> private staged tree -> publish -> inspect -> reconcile
                                      `-> optional link into a consumer view
```

Each arrow is a real method or trait operation. The shape is descriptive, not a mandatory pipeline:
removal, inspection, reconciliation, linking, and process execution remain independently callable.

## Custody and publication

`LocalSource` and `LocalTarget` admit non-empty filesystem identities. Caller-owned local files and
directories survive value drop. Adapter-owned staged files and `StagedTree` values retain cleanup
custody until consumed or dropped.

A destination-oriented stage is created beside its target. `StagedTree::publish` is the commit
boundary: it publishes a new tree without exposing a partially assembled destination. Process
scratch stages are separate custody and cannot be published as destination-oriented stages.

The library assumes trusted parent directories. Its final-component checks and staging laws are not
a capability sandbox against hostile concurrent mutation of ancestors.

## Observation and evidence

Inspection reports facts and does not mutate. Reconciliation compares those facts with an explicit
expectation and also does not mutate. Repair is application composition of observation plus selected
effects, never an implicit branch inside reconciliation.

Evidence records what a concrete operation observed or changed. It is useful audit data, not proof
of authorization, provenance, or an unforgeable capability.

## Feature ownership

- `local` owns filesystem admission, staging, publication, views, records, and metadata inspection.
- `hash` owns digest vocabulary; `blake3` and `sha2` add concrete local verification.
- `archive` formats prepare into a caller-provided extraction workspace under explicit limits.
- `net` owns URL, retry, admission, pacing, and evidence vocabulary; HTTP adapters own transport.
- `process` owns admitted worktrees, exact environments, bounded diagnostics, managed sessions, and
  staged outputs. Tokio adapters preserve the same result semantics.

Parent features do not hide unrelated algorithms. A consumer can select one HTTP adapter, digest,
or archive format without enabling every implementation.

## Failure domains

- Admission errors occur before the associated effect.
- Acquisition never grants authority to publish a caller's final target.
- Archive limits are checked against observed decoded/materialized data.
- Publication does not report failure merely because later best-effort cleanup failed.
- Process cancellation and timeout retain their own explicit methods and evidence.
- Record edits use bounded streaming custody and atomic replacement rather than loading arbitrary
  state into one unbounded buffer.

## Platform semantics

Linux and Windows adapters must implement equal declared meaning even when mechanisms differ.
macOS currently receives best-effort build and test observation. Service-manager adapters are kept
in the `toolhost` example because systemd and Windows SCM are ecosystem policy, not universal Pulith
behavior.
