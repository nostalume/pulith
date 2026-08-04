# Pulith architecture

Pulith is a composable resource-management state machine. It gives callers typed behavior
transitions for external resources while leaving application policy and orchestration outside the
crate.

This document describes the concepts present in the current repository. It is not a roadmap and
does not treat unused state names as implemented capabilities.

## Architectural center

Pulith keeps three independent axes:

```text
behavior law × resource semantics × adapter
```

- A **behavior law** defines a legal transition, its input, required policy, output, evidence,
  errors, effects, and failure boundary.
- **Resource semantics** define what acquisition, identity, observation, publication, or failure
  means for one kind of resource.
- An **adapter** implements one demonstrated intersection of a behavior and a resource.

The caller composes those intersections into a workflow. Pulith has no global application object,
context, registry, factory, middleware pipeline, or universal resource lifecycle.

## State graph

```text
Materialize -> Acquired -> Applied
                        -> Verified -> Applied
                        -> Prepared -> Applied
                        -> Verified -> Prepared -> Applied
Forget -----------------------------> Applied
LocalTarget -> Inspected -> Reconciled
RemoteUrl  --> Inspected
```

The graph is an inductive composition vocabulary rather than one mandatory sequence.

- **Materialize** is a caller-owned request naming an item, source, target, and publication mode.
- **Forget** is a caller-authorized target-only removal request. It deliberately bypasses source,
  acquisition, verification, and preparation.
- **Acquired** carries material and acquisition evidence, but grants no final-target authority.
- **Verified** preserves material custody while adding a factual proof.
- **Prepared** carries transformed, resource-specific material ready for a later effect.
- **Applied** records completion of one target effect.
- **Inspected** carries read-only resource facts.
- **Reconciled** classifies an observation against caller-owned expected state without mutating the
  resource.

Verification and preparation are optional branches selected by the caller. Inspection and
reconciliation form an observation path, not a hidden repair loop.

## Behavior contracts

The kernel exposes `Acquire`, `Verify`, `Prepare`, `Apply`, `Inspect`, and `Reconcile`, plus concrete
asynchronous forms where an asynchronous adapter exists. Each behavior declares associated
`Error` and `Output` types. Policy required by a transition is passed as the generic `Need` value;
it is not read from shared global state.

Every behavior owns a narrow authority boundary:

| Behavior | May do | Must not imply |
| --- | --- | --- |
| Acquire | Read a source and produce resource-specific material | Publish the final target |
| Verify | Establish a caller-requested factual property | Authenticate the expectation or authorize publication |
| Prepare | Transform material in isolated custody | Publish the final target |
| Apply | Perform the exact caller-authorized target effect | Claim package ownership or a multi-target transaction |
| Inspect | Observe resource-specific facts without mutation | Decide desired state |
| Reconcile | Compare observation with caller-owned expectation | Repair, adopt, delete, or persist |

Synchronous and asynchronous adapters share these semantic contracts. Execution modality does not
change authority or evidence laws.

## Evidence model

Transition outputs carry typed evidence. `EvidenceChain` preserves upstream evidence and appends
the current adapter's evidence, so later stages can retain the factual path that produced them.

Canonical state records are intentionally open. This lets third-party adapters create a canonical
state and continue through built-in behaviors, or consume a built-in state and implement a later
transition. The selected adapter attests to the evidence; record construction does not make that
evidence provenance, authorization, or an unforgeable capability.

Resource values may enforce stronger construction invariants when a built-in behavior depends on
them. For example, prepared archive custody is crate-constructed even though the surrounding
canonical record remains open. Openness is decided at the boundary that owns the invariant.

Evidence is also not automatically a domain event. The state records are composition products, not
entities, aggregates, or durable lifecycle records.

## Resource boundaries

The current implementation demonstrates four resource contexts while keeping their meanings
separate:

| Context | Owns | Does not establish |
| --- | --- | --- |
| Local filesystem | Entry kind, staging, publication, removal, and read-only observation | Package ownership or durable desired state |
| HTTP representation | URL, status, validators, retries, admission, pacing, and transfer attempts | Artifact identity, publisher trust, or final publication |
| Artifact identity | Algorithm-typed digest and exact byte size | Provenance, authorization, or trust in the supplied expectation |
| Archive | Guarded decoding and prepared tree custody | Final-destination publication |

These contexts meet only through typed states. For example, an HTTP adapter can acquire staged
local material, a digest adapter can verify it, an archive adapter can prepare it, and a local
adapter can publish it. No participating adapter becomes the universal owner of the workflow.

## Custody and effects

Custody is carried by resource types rather than hidden orchestration:

```text
caller request
  -> acquired source or adapter-owned stage
  -> optional verified custody
  -> optional prepared custody
  -> explicit final-target apply
```

Disposable stages are owned by their state and are removed when abandoned. Caller-owned local
sources and resume partials remain caller-owned. Preparation uses exclusive destructive scratch;
successful preparation yields guarded prepared custody, not a published destination.

Final publication is a distinct commit boundary. Local regular-file `Create` expects a missing
predecessor and uses a no-clobber commit as the authoritative execution-time check. A late existing
target is a typed conflict and the winner remains unchanged. That conditional law is specific to
regular-file creation; it is not generalized to directories, replacement, forgetting, or
digest-based compare-and-swap.

## Observation and convergence

Observation preserves resource-specific meaning:

- Cheap local inspection observes no-follow metadata only.
- Exact local artifact inspection is an explicit full-read behavior that reports a typed digest and
  the bytes counted by the digest loop. It reads a regular-file handle, rejects links and other
  special entries, trusts parent directories, and is not an atomic snapshot under concurrent
  writes.
- HTTP inspection uses HEAD only. Every received final status is an observation; declared content
  length is metadata rather than observed bytes or identity.

Reconciliation consumes caller-owned expectations and produces a classification with preserved
evidence. Mutation requires a separate explicit behavior and authority grant.

## Failure domains

Failures stay attached to the behavior and resource that owns them:

- Acquisition failure cannot publish or create the final destination.
- Verification mismatch leaves the final target untouched.
- Preparation failure leaves the final destination untouched and reports cleanup failure when the
  exclusive workspace cannot be reset.
- Application reports target conflicts as application conflicts rather than generic I/O when the
  publication law can classify them.
- Inspection errors mean no valid observation was produced; negative resource facts such as local
  absence or an HTTP error status remain observations where the resource contract says so.
- Reconciliation is pure and grants no mutation authority.

Bounded work is part of the contract. Network acquisition bounds materialized bytes before pacing
and staging. Archive preparation rejects unsafe paths and entry kinds and enforces limits against
observed decoded/materialized work, including container overhead.

## Caller-owned concerns

Pulith deliberately leaves these concerns to the composing application:

- application and item identity;
- desired state and dependency solving;
- source discovery and adapter selection;
- trust, provenance, authorization, and admission decisions;
- orchestration, cancellation, and cross-resource concurrency;
- durable aggregates, repositories, and recovery protocols;
- multi-target transactions, rollback, retention, and garbage collection;
- repair and adoption policy.

Those concerns may use Pulith's typed behaviors, but none is promoted into a universal domain model
without a demonstrated consistency boundary and concrete adapter.

## Extension law

A new capability belongs in Pulith only when it adds a real behavior/resource intersection:

1. Define the behavior's input, policy need, output evidence, error, effect, and failure law.
2. Name the owner of every authority and resource invariant.
3. Preserve upstream identity, custody, and evidence through the typed output.
4. Keep resource-specific semantics out of the universal behavior vocabulary.
5. Supply a concrete adapter and contract tests for the claimed boundary.

External adapters can implement the existing public traits and compose through the open canonical
records. New global registries, factories, compatibility shells, or state names without concrete
behavior are outside the architecture.

## Current boundary

The repository currently implements local and HTTP acquisition, digest and exact-descriptor
verification, guarded archive preparation, staged local publication, direct local forgetting,
local and HTTP inspection, and pure local reconciliation. Asynchronous behavior exists for HTTP
acquisition and inspection only.

It does not implement a package manager, source discovery, trust selection, installation database,
dependency solver, automatic repair, multi-target transaction, or system package-manager adapter.
