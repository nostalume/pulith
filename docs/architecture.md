# Architecture

Pulith is a library of composable contracts for external-resource work. It owns narrow operations,
resource-specific safety laws, and evidence. The application owns the workflow and its policy.

This separation allows an installer, tool manager, updater, or deployment utility to reuse safe
mechanisms without inheriting a universal package model.

## Contract model

Every Pulith operation belongs to one of four layers:

| Layer | Defines | Owns |
| --- | --- | --- |
| Behavior | one operation and its result shape | crate-root traits such as `Acquire`, `Inspect`, and `Reconcile` |
| Resource | valid inputs and resource-specific laws | `local`, `net`, `archive`, `hash`, and `process` types |
| Adapter | the concrete effect mechanism | filesystem, HTTP, codec, process, or platform implementation |
| Application | why and when operations compose | identity, desired state, trust, orchestration, retention, and recovery |

A restricted resource value is useful only when it adds a law: for example, an admitted URL,
target path, worktree, or exact environment. Pulith avoids wrappers that merely rename an existing
type, global registries, and context objects whose authority is difficult to see.

## Behavioral laws

Four rules govern the API:

1. **Admit before effect.** Inputs are parsed and restricted before the operation can mutate or
   execute.
2. **One method, one declared effect.** Inspection does not repair; reconciliation does not mutate;
   acquisition does not publish.
3. **Return evidence at the observation point.** The adapter that observed or changed a resource
   reports the corresponding typed evidence.
4. **Preserve custody until commit.** Temporary material remains privately owned until an explicit
   publication method consumes it.

Evidence is audit data. It records an observation or effect, but is not proof of authorization,
provenance, or an unforgeable capability.

## Artifact lifecycle

A typical application may compose this flow:

```text
admit source
    -> acquire private material
    -> verify an explicit expectation
    -> prepare into a private tree
    -> publish the completed tree
    -> inspect current state
    -> reconcile with caller-owned intent
    -> optionally link a consumer-visible view
```

This is not a built-in pipeline. Every step is independently callable, and the application may
omit, reorder, or branch between operations where their contracts permit it.

### Acquisition

`Acquire` obtains material from an admitted source. Local and HTTP adapters differ in transport but
return staged material under the same custody rule. Neither receives authority to select or
publish the final application destination.

### Verification and preparation

`Verify` proves one explicit expectation, such as an exact digest. Digest algorithms are separate
features because selecting BLAKE3 is not equivalent to selecting SHA-256.

Archive formats prepare into caller-selected private workspace. Preparation applies decoded-size,
entry-count, path, link, collision, and file-kind rules to the material actually encountered. ZIP,
TAR, and compressed TAR support share a safety contract while retaining format-specific decoding.

### Staging and publication

A destination-oriented `StagedTree` is created beside its target. Its methods assemble a complete
tree while that tree is private. `publish` consumes the stage and is the commit boundary: readers
must not observe a partially assembled destination.

Process scratch output uses separate custody. A scratch worktree cannot accidentally acquire the
authority of a destination-oriented stage.

### Observation and reconciliation

`Inspect` reports current facts and does not mutate. `Reconcile` compares an observation with an
explicit caller-owned expectation and also does not mutate. Repair is application code that selects
subsequent effects based on that difference.

Views are similarly explicit: `Link` exposes a published tree and `Unlink` withdraws the selected
view. Platform mechanisms may differ, but the declared effect may not.

## Process lifecycle

Process resources separate four concerns:

| Concern | Contract |
| --- | --- |
| Command and worktree admission | resolve executable identity and restrict working-directory scope |
| Environment construction | build an exact environment rather than inheriting ambient state implicitly |
| Execution | run to completion with explicit time and diagnostic bounds |
| Managed lifetime | acquire a running session, then wait, cancel, or terminate through separate methods |

Configuration choices that change behavior use separate constructors or methods rather than an
`Option<T>` switch. Synchronous and asynchronous adapters preserve the same outcomes, limits, and
evidence; only waiting and I/O modality change.

## Ownership by module

| Module or feature | Responsibility |
| --- | --- |
| `local` | path admission, staging, publication, views, records, metadata inspection |
| `hash` | digest vocabulary |
| `blake3`, `sha2` | concrete exact-digest verification |
| archive format features | safe preparation through a selected decoder |
| `net` | URL admission, retry, pacing, limits, and transport-neutral evidence |
| `http-ureq`, `http-reqwest` | synchronous and asynchronous HTTP transport |
| `process` | admitted worktrees, exact environments, bounded execution, managed sessions |
| `process-tokio` | asynchronous process execution with the same process contract |

Parent features do not silently choose unrelated algorithms or runtimes. Consumers select only the
adapter, codec, digest, or runtime they need.

## Failure and recovery

- Admission failures occur before the associated external effect.
- Failure evidence preserves bounded diagnostics rather than collecting unbounded process output.
- Network retry obeys admitted policy and pacing; it is not hidden in the application workflow.
- Archive limits apply during preparation, before publication.
- Publication success is not rewritten as failure because later best-effort cleanup failed.
- Durable records use bounded streaming custody and atomic replacement rather than loading an
  arbitrary journal into one buffer.
- Recovery and rollback remain explicit application decisions based on retained material and
  evidence.

## Platform contract

Linux and Windows are required semantic peers. Each may use its native filesystem, process, or
service-manager mechanism, but both must implement the same declared operation. macOS remains
best-effort until equivalent runtime evidence is continuously available.

Systemd and Windows SCM integration stays in the `toolhost` example. Their service ecosystems,
privilege models, and lifecycle policy are application adapters, not universal Pulith behavior.
The example uses Pulith for the reusable pieces: admitted paths and environments, process
execution, staged publication, inspection, and evidence.

## Extension test

A proposed abstraction belongs in Pulith only when all of these are true:

- more than one application can use the same resource law;
- the contract has a narrow, nameable effect;
- the library can return meaningful typed evidence;
- authority and cleanup custody remain visible;
- platform adapters can provide equal semantics;
- the abstraction does not force application identity or orchestration policy into the crate.

Otherwise, keep the behavior in the application or example until repeated use reveals a smaller
general contract.
