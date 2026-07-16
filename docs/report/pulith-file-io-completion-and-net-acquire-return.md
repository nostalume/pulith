# Pulith File Operation Completion and Net Acquire Return Plan

## Status

This pass follows the decision to prefer rejection over preserving old filesystem mechanisms.

Completed:

```text
deleted crates/pulith-fs
updated active docs away from pulith-fs as a core dependency
kept file-operation behavior inside crates/pulith typed modules
returned the next implementation focus to net Acquire
```

Deletion command result:

```text
deleted=crates/pulith-fs
```

The recursive deletion required approval and was approved.

## What was rejected

The remaining `pulith-fs` mechanisms were rejected as public APIs for the current single-crate design.

### hardlink_or_copy

Decision:

```text
reject for now
```

Reason:

- Hardlinks share file identity and can surprise callers after placement.
- Cross-device fallback changes behavior from hardlink to copy.
- Current `LocalApply` default is clearer and safer as copy-only evidence.

Future reopen condition:

```text
Only reintroduce as explicit PlacementStrategy::HardlinkOrCopy with evidence proving copied/hardlinked/mixed outcomes.
```

### Durability / fsync policy

Decision:

```text
reject for current Apply
```

Reason:

- `tempfile::persist` gives atomic-ish placement, not full crash durability.
- Parent-directory sync is platform-specific and should not be implied.
- Current contract should stay honest: placement safety, not crash-consistency durability.

Future reopen condition:

```text
Only add as explicit DurabilityPolicy::FileSync / FileAndParentSync with platform notes and tests.
```

### permission preservation

Decision:

```text
reject for current Apply
```

Reason:

- Permission semantics differ across Unix/Windows.
- Preserve-source permission policy is not required for current local/archive Apply.
- Implicit permission copying would be surprising.

Future reopen condition:

```text
Only add as explicit PermissionPolicy.
```

### Windows retry/backoff around directory replacement

Decision:

```text
reject for current slice
```

Reason:

- Current staged directory replacement is backup-based and test-backed.
- Retry/backoff is operational policy, not core behavior identity.

Future reopen condition:

```text
Only add as explicit LocalApply resource policy if Windows open-handle failures appear in real tests.
```

### file-lock Transaction

Decision:

```text
reject for Apply
```

Reason:

- Locking is state/store behavior, not local placement behavior.
- Old transaction wrote by truncating in place; it is not staged placement.

Future reopen condition:

```text
Consider only for future Remember/state persistence.
```

### symlink creation

Decision:

```text
reject by default
```

Reason:

- Current LocalApply and Archive Prepare reject symlink/special entries.
- Symlink/junction creation has escape and platform-semantics risk.

Future reopen condition:

```text
Only add with explicit SymlinkPolicy and archive/local tests.
```

### aligned buffers

Decision:

```text
reject
```

Reason:

- No direct-I/O requirement exists.
- Unsafe allocation surface is unjustified.

## File-operation completion level

### Completed and verified

```text
local source acquire for files/directories
staged file Apply using NamedTempFile in target parent
Create via persist_noclobber
Replace/CreateOrReplace via persist
same-file guard with same-file crate
source-target containment guard for directories
staged directory Apply using TempDir + walkdir
symlink/special entry rejection by default
archive extraction path containment and symlink-entry rejection
archive extraction root reset to avoid stale file carryover
archive target path component symlink guard
ArchiveTree final placement delegates to hardened LocalApply
hash Verify rejects symlink/non-file material via symlink_metadata
ApplyEvidence includes files/directories/bytes/strategy
```

### Explicit non-goals / rejected for now

```text
hardlink placement
permission preservation
crash-durability fsync policy
Windows retry/backoff policy
symlink creation/preservation
file-lock transaction API
unsafe aligned/direct-I/O buffers
```

### Completion judgement

File-operation behavior is complete for the current Pulith single-crate baseline:

```text
safe default local material placement
safe default archive-to-local placement
safe default digest verification
copy-only, symlink-rejecting, typed-evidence behavior
```

It is not claiming:

```text
strict transactionality for directory replacement
crash consistency after power loss
permission/timestamp preservation
hardlink optimization
symlink preservation
```

Those are intentionally rejected or deferred as explicit policies.

## Documentation updated

Updated active docs:

```text
README.md
docs/AGENT.md
docs/architecture.md
```

Key direction:

```text
pulith-fs is no longer an active primitive crate
file behavior lives in crates/pulith local/archive/hash modules
public filesystem choreography is rejected
```

Some historical design/publish documents still mention `pulith-fs`; those are archival context, not active architecture authority.

## Net Acquire return plan

Now that file operation behavior is closed for the baseline, return to `net` Acquire.

Recommended next order:

### 1. Design net Acquire contract

Keep the same typed tree style:

```text
Intent<I, LocalTarget, O>
  -> WithSource<I, RemoteUrl>
  -> Chosen<I, RemoteUrl>
  -> Acquired<I, LocalMaterial, NetAcquireEvidence>
```

Net Acquire should produce local material, not install directly.

### 2. Sync HTTP first with `ureq`

Feature:

```text
ureq = ["sync", "dep:ureq"]
```

Why first:

```text
small sync surface
fits current sync typed tree
no runtime requirement
```

Expected behavior:

```text
download remote URL into staged tempfile/cache path
publish as LocalMaterial::File
record URL/status/final_path/bytes evidence
```

### 3. Async HTTP second with `reqwest`

Feature:

```text
reqwest = ["async", "dep:reqwest", "dep:tokio"]
```

Why second:

```text
requires async trait path and runtime policy
should mirror sync behavior after sync contract stabilizes
```

### 4. Defer object_store

Feature:

```text
object = ["async", "dep:object_store"]
```

Reason:

```text
object_store is high-quality for object backends but too broad for first URL Acquire
```

### 5. Verification requirements for net Acquire

Tests should prove:

```text
successful URL download produces LocalMaterial::File
non-2xx response errors before Apply
byte count evidence matches downloaded body
download uses staged placement, not partial target writes
hash Verify can consume acquired material
LocalApply can consume verified acquired material
```

Use a local test server or mock server; avoid external network dependency in tests.
