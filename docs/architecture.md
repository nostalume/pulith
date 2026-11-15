# Architecture

Pulith is a library of composable contracts for external-resource work. It owns narrow operations,
resource-specific safety laws, and evidence. The application owns the workflow and its policy.

This separation allows an installer, tool manager, updater, or deployment utility to reuse safe
mechanisms without inheriting a universal package model.

## Contract model

Every Pulith operation belongs to one of four layers:

| Layer | Defines | Owns |
| --- | --- | --- |
| Behavior | one operation and its result shape | resource methods named for their operation |
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
3. **Return facts at the operation boundary.** Reads return the observed value. Publication
   returns the effect that completed. An operation may also report facts it obtained along the way,
   such as HTTP attempts.
4. **Preserve custody until commit.** Temporary material remains privately owned until an explicit
   publication method consumes it.

Evidence is audit data. It records an observation or effect, but is not proof of authorization,
provenance, or an unforgeable capability.

## Artifact lifecycle

A typical application may compose this flow:

```text
admit source
    -> open a local file or capture remote bytes
    -> prepare private output from the bytes actually verified
    -> publish the completed tree
    -> inspect current state
    -> compare with caller-owned intent
    -> optionally link a consumer-visible view
```

This is not a built-in pipeline. Every step is independently callable, and the application may
omit, reorder, or branch between operations where their contracts permit it.

### Source access and execution

`LocalPath` carries a non-empty path. A read or a write admits the role at its own boundary.
`UreqResources::download` performs a GET and returns an owned `FileSnapshot` and
transport evidence. `UreqResources::inspect_headers` performs a HEAD and reports the declared HTTP
facts and attempts. `PreparedProcess::execute` runs a child to completion and takes custody of
its output stage.

### Verification and preparation

`OpenedFile::digest` describes the complete stream read through the admitted handle.
`OpenedFile::snapshot_verified` captures and verifies in one pass and returns owned bytes.
`FileSnapshot::verify_digest` verifies already captured bytes and rewinds them for the next
reader. `LocalPath::publish_verified_file` copies and hashes from the admitted handle in one pass
and publishes the private stage after the digest matches. This entry requires an absent target
and returns the publication result and the actual stream descriptor. BLAKE3 and SHA-256 remain
separate features.

Archive formats prepare into caller-selected private workspace. Preparation applies decoded-size,
entry-count, path, link, collision, and file-kind rules to the material actually encountered. ZIP,
TAR, and compressed TAR support share a safety contract while retaining format-specific decoding.
The extraction root must be absent or an empty regular directory. Admission leaves an occupied
root untouched. The caller keeps the admitted namespace exclusive until extraction completes.
After an extraction error, Pulith clears the children it produced and preserves the root for
retry. If cleanup also fails, both errors remain available to the caller.
Relative paths are constructed from admitted components. The empty exclusive root and the
refusal of link entries establish where their descendants can be created throughout extraction.

### Staging and publication

A destination-oriented `StagedTree` is created beside its target. Its methods assemble a complete
tree while that tree is private. Copying a file requires a free final stage name. `publish` consumes the stage and is the commit boundary: readers
must not observe a partially assembled destination. The commit takes an explicit `TreeAdmission`:
`Absent` renames only onto a free name, `ReplaceRegularFile` also replaces a regular file in place
and, on POSIX, carries the predecessor's permission bits onto the new file. `LocalPath::inspect_entry` reports the entry read-only. The publisher reclassifies
at commit time and refuses a directory, symlink, reparse point, or other entry it observes. The
final component is never dereferenced. The classification and the native rename are separate
operations, with the concurrency boundary described in the PS6 design below.

When new file bytes depend on a previously read file, pass its digest as
`FileAdmission::ReplaceMatching` to single-file publication. The publisher consumes the digest
at its commit boundary, returns `PredecessorChanged` for a missing or different predecessor, and
returns publication evidence only after a successful rename. It reads the predecessor through
an admitted regular-file handle. This observes its bytes before rename and carries the same
namespace stability assumption as regular replacement.

`LocalPath::publish_file` is the same boundary at the size of one file: a hidden temporary file
beside the target, the same admission check before the rename, the same mode inheritance, and no
tree. The tree is the right unit when several entries must become visible together; it is the wrong
unit when each file is an independent decision, because then every file pays for a workspace
directory, a `tree/` child, a post-commit walk, and a recursive removal that publish nothing. The
two entries share the regular-file admission check, `inherit_mode`, and the two rename forms, so platform differences
still exist once.

Process scratch output uses separate custody. A scratch worktree cannot accidentally acquire the
authority of a destination-oriented stage.
Diagnostic capture reads the handles opened during spawn. The reported bytes and truncation
flags describe those streams even if a child changes their workspace names. A stream cap limits
the bytes returned, and a zero cap discards the stream during execution.

### Observation and reconciliation

`LocalPath::inspect_entry` reads metadata for the final entry without following links.
`inspect_descendant` checks ordinary directory components below a caller-selected base and
classifies the final entry. `resolve_directory` follows directory links for placement planning
and retains the missing suffix beneath an existing directory. Both observe the namespace and
assume path stability between an observation and a later path-based effect.
`LocalPath::open_file` admits a regular-file handle for data and metadata read from the same
object. A live handle is still subject to in-place changes. A snapshot owns the bytes it copied,
and digest verification establishes whether those captured bytes match a declared value.

`compare` borrows the observation and the caller's expectation and returns a classification.
`RecordStore::read` returns bounded optional bytes. `RecordEdit::publish_from` decides between
creation and replacement under the edit lock without reading the old body. Repair remains
application policy.

`inspect_view` follows a view to compare its relationship with the declared exposed directory.
It distinguishes a missing view, an occupied non-link, a dangling link, and a different referent.
`link_view` installs or switches that relationship and `unlink_view` withdraws it. `None` selects
the tree root for inspection and linking. A failed confirmation after a successful link carries
the link receipt in `LinkError::AfterLink`.

## Process lifecycle

Process resources separate four concerns:

| Concern | Contract |
| --- | --- |
| Command and worktree admission | resolve executable identity and restrict working-directory scope |
| Environment construction | build an exact environment rather than inheriting ambient state implicitly |
| Execution | run to completion with explicit time and diagnostic bounds |
| Managed lifetime | start a session, poll its exit, wait, or stop it within a caller-supplied bound |

Configuration choices that change behavior use separate constructors or methods rather than an
`Option<T>` switch. Managed and bounded execution share the native process startup.

## Ownership by module

| Module or feature | Responsibility |
| --- | --- |
| `local` | path admission, staging, publication, views, records, metadata inspection |
| `hash` | digest vocabulary |
| `blake3`, `sha2` | concrete exact-digest verification |
| archive format features | safe preparation through a selected decoder |
| `net` | URL admission, retry, pacing, limits, and transport-neutral evidence |
| `http-ureq` | shared HTTP transport for inspection and download |
| `process` | admitted worktrees, exact environments, bounded execution, managed sessions |

Parent features do not silently choose unrelated algorithms or runtimes. Consumers select only the
adapter, codec, or digest they need.

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

An archive digest attests to the compressed or TAR bytes supplied for installation. Once
published, verifying the installed contents would require evidence about the extracted tree.
An application may reconcile an existing directory and its view without rereading the tree,
and should report that narrower result. A regular-file digest can verify installed byte
content directly. The example owns these reporting choices, along with how to handle local
drift and when to retry.

For a file selected by an application, the resolved parent plus the final name denotes the
publication position. The application owns rules such as protecting its source tree or reserving
a credential path. A digest describes the byte stream observed through a handle. Comparing that
digest before an ordinary replacing rename detects earlier changes without making the check and
rename atomic.

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

## Local file publication and protection

### Observe, admit, publish

A caller observes a file to decide what to do, then chooses the publication admission. Observation
is read-only and gives no reservation. The publisher rechecks admission using current filesystem
facts. The successful effect supplies the receipt. Applications still decide whether to write and
how to use that receipt.

- **P1, custody and admission.** Exclusive creation establishes ownership of a temporary file.
  Cleanup begins only after that operation succeeds. A collision leaves the existing sibling
  untouched. After a successful rename the destination is outside temporary custody. A failure
  before publication keeps the predecessor intact, subject to external changes, and cleans only
  the stage owned by the call. Parent directories may already have been created. `Absent` must
  enforce no replacement in the native operation. `ReplaceRegularFile` classifies the final
  component without following it before choosing the rename. Ordinary parent-path resolution is
  allowed. The name can change between classification and replacement. The contract guarantees
  no dereference of the final link and no reservation against a concurrent namespace writer.
  The caller controls the destination namespace. Private publication cannot protect it against
  a principal allowed to replace the stage or redirect its parent.
- **P2, observed facts.** Keep occupancy, bytes or length, and protection separate. Ask for the
  facts needed by the caller and take file metadata and bytes from the same regular-file handle
  when both are requested. This prevents a path swap from redirecting the read and avoids a
  second path query for protection. It gives no snapshot of a file being modified in place.
  Absent, refused entry kind, unavailable protection, and I/O failure remain distinguishable.
  Keep metadata-only inspection cheap. Use `inspect_entry` for the final entry and `inspect_view`
  for link resolution. `LocalPath::open_file` returns `OpenedFile`, absence, or a typed refusal.
  Exact byte observation uses `OpenedFile::digest`.
  Observation of a view's referent is a separate operation from opening regular data.
- **P3, publication evidence.** Stage contents report prepared files, directories and bytes.
  Publication evidence reports the successful operation: a native no-replace publication, or a
  replacement after observing a regular predecessor. A replacement receipt attests to the
  pre-rename observation and the successful rename. It cannot prove that the same predecessor
  was still present at the syscall. An absent target that becomes occupied before no-replace
  succeeds yields a conflict. Use `TreePublication` for tree totals and `FilePublication` for a single-file result. Tree evidence can
  adopt the distinction when a tree caller consumes it.

### P6 protection

Admission answers whether a name may be occupied. Protection answers who can access the bytes.
The ordinary path uses the existing platform defaults and POSIX predecessor mode on replacement.
An owner-only single-file publication uses the same custody and admission. On POSIX, create-only publication links the synced stage into a free name and removes the temporary name. Replacement uses a native rename.
Apply protection before writing the first byte and verify the required result on the object
being published. POSIX requires exactly `0600`, including under a restrictive umask. An
unsupported protection guarantee must be distinguishable from access denied or another I/O
error. No content is written with broader access as a fallback. An identity file also needs
create-only publication. Its caller retains the existing file-sync requirement before publishing
ciphertext that depends on it. Crash durability of the directory entry is a separate contract.

Windows local NTFS uses a temporary file created with a protected DACL. The file operation
owns protection from the first write through publication. Test it using actual access by the
publishing user and a distinct ordinary reader who can open a control with the same parent
permissions, both before and after publication. Include create-only and replacement, and run
under permissive and restrictive inheritance. The contract covers new bytes written by the
operation. Access already granted to an old file, privileged access, and mutation of the
namespace or protection by an authorized actor remain outside this scope. Unsupported
storage and failed publication have distinct observable outcomes.
Use the same read-only protection predicate for the plan and for verification after publication.
Unknown protection is a refusal for operations requiring it. A confirmed protection mismatch
on an otherwise valid file can schedule repair, including when its bytes already match.
Report known capability absence during read-only planning when the caller needs a zero-write
refusal. Revalidate at the effect boundary. Unexpected access or I/O failures during publication
keep the existing single-file failure semantics.
The Windows preview uses the existing private publication operation on local NTFS. Planning may
reject a known unsupported destination using its nearest existing parent. The publisher obtains
the volume and protection facts from its actual staging handle before accepting any bytes.
An existing file's protection is observed on its opened handle. Release scope follows the
observed access results for each operation.
[Security descriptors at object creation](https://learn.microsoft.com/en-us/windows/win32/secauthz/security-descriptors-for-new-objects)
· [Linux rename semantics](https://www.man7.org/linux/man-pages/man2/renameat2.2.html)

The NTFS access contract can be evaluated without a second login. Exercise native staging and
publication, read back the persisted security descriptors, and submit them to Windows Authz
with the owner and a distinct synthetic ordinary user. The reader must be granted access to
an inherited control, denied access to protected data, and gain the requested access when
the control ACL is deliberately changed. This establishes discretionary access for the tested
subjects and objects. Real logon tokens, existing open handles and directory traversal have
their own runtime conditions. Keep this evaluation in acceptance tooling.
[AuthzAccessCheck](https://learn.microsoft.com/en-us/windows/win32/api/authz/nf-authz-authzaccesscheck)

An editor may replace its input or leave a sibling backup. The protected directory supplies the
inheritance boundary for cooperating editors creating files within it. The caller owns editor
execution, the no-follow read of the edited leaf, and cleanup at session end. The library can
supply the private workspace when a consumer needs the directory capability. An editor that
exports bytes, broadens permissions, or moves in a file prepared elsewhere can leave that
boundary. Ordinary file access during creation, replacement and backup and cleanup on session
exit are the evidence for this capability. P4 headless process semantics and P5 eager staging
effects were established in PS5.
[Windows file replacement and backups](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew)
