# Pulith Stream/Chunk, Resume/Range, and Async Runtime Analysis

## Status

This report answers:

1. Whether stream-style async body handling changes the resume/range design.
2. Whether the current `Response::chunk().await` design is compatible with resume/range.
3. Whether a `bytes_stream()` style design would be more compatible.
4. What the async runtime plan should be now.

No production code is changed by this report.

## Current implementation inspected

Current async reqwest path:

```rust
while let Some(chunk) = match response.chunk().await { ... } {
    stage.write_chunk(&chunk, source.policy.max_bytes).await?;
}
let stage = stage.finish().await?;
stage.persist(&source.destination)?;
```

Current staging shape:

```rust
StagedDownload<Open> {
    temp: tempfile::NamedTempFile,
    bytes: u64,
    writer: Open { file: tokio::fs::File },
}

StagedDownload<Open>::write_chunk(...)
StagedDownload<Open>::finish() -> StagedDownload<Closed>
StagedDownload<Closed>::persist(...)
```

Current sync ureq path:

```rust
copy_response_body(response.body_mut().as_reader(), temp.as_file_mut(), max_bytes)
```

Current feature/runtime shape:

```toml
async = []
runtime-tokio = ["async", "dep:tokio"]
reqwest = ["net", "runtime-tokio", "dep:reqwest"]
```

So `reqwest` is explicitly Tokio-backed via `runtime-tokio`.

## External facts checked

### reqwest body APIs

From local reqwest source/docs:

```rust
pub async fn chunk(&mut self) -> crate::Result<Option<Bytes>>
```

Semantics:

```text
returns one body chunk at a time;
returns None when exhausted;
errors on body decode/transport failure.
```

Also available behind reqwest's `stream` feature:

```rust
pub fn bytes_stream(self) -> impl Stream<Item = crate::Result<Bytes>>
```

The crate feature list shows:

```text
reqwest stream = [tokio/fs, futures-util, tokio-util, wasm-streams]
```

Pulith does not currently enable reqwest's `stream` feature.

### HTTP Range/Resume facts

From the previous resume-first research and MDN references:

```text
Range asks for byte intervals.
206 Partial Content is successful range response.
Content-Range describes returned interval and complete length.
Server may ignore Range and return 200 OK.
If-Range mismatch can intentionally yield 200 OK full body.
416 Range Not Satisfiable may indicate stale/corrupt partial state and may be recoverable by restart.
Accept-Ranges is only advertisement; response status/headers decide actual behavior.
```

### Tokio/runtime facts

Existing Pulith assessment still holds:

```text
reqwest async is Tokio/hyper-backed.
Tokio runtime is caller-owned; Pulith library must not create a hidden runtime.
ReqwestResource owns shared Client and delay provider, not runtime state.
Operation-local state is response body + staging file + attempt/resume state.
```

## Core conclusion

Stream/chunk mechanics do not decide resume correctness.

Resume correctness is decided by:

```text
request construction: Range, If-Range
response interpretation: 200/206/416
validator handling: ETag, Last-Modified
Content-Range validation
partial staging state
append/restart/discard policy
final persist law
```

`chunk()` and `bytes_stream()` are both just ways to consume a response body after those decisions are made.

Therefore:

```text
chunk() is compatible with resume/range as a body-copy primitive.
bytes_stream() is also compatible, but not necessary for first resume implementation.
Neither API by itself provides resume semantics.
```

## Current `chunk()` design compatibility

### Compatible parts

The current code already has good building blocks:

```text
1. chunks are written incrementally, so memory does not require whole-body buffering.
2. StagedDownload tracks bytes written.
3. Only Closed staging can persist.
4. A chunk error can trigger retry/restart without touching final destination.
5. Same-parent temp file placement remains compatible with resume.
```

These are compatible with resume/range if the write offset is generalized.

### Missing parts

Current `StagedDownload` assumes each attempt starts at byte 0:

```rust
bytes: 0
```

and `write_chunk()` only checks:

```rust
self.bytes + chunk.len() <= max_bytes
```

For resume, the stage must know whether it is:

```text
fresh full-body staging
partial staging at validated offset N
append staging expecting Content-Range start == N
restart staging after discarding partial
```

So `chunk()` is fine, but `StagedDownload<Open>` needs a resume-aware state model.

## Current `chunk()` vs future `bytes_stream()`

### `chunk()` advantages now

```text
No new reqwest feature required.
No futures StreamExt dependency in Pulith.
Control flow stays simple: while let Some(chunk) = response.chunk().await.
Good enough for sequential body copy.
Easier to keep helper count low.
```

### `bytes_stream()` advantages later

```text
Can compose with stream adapters for progress, throttling, checksumming, cancellation wrappers, or test harnesses.
Can abstract body copy over generic Stream<Item = Result<Bytes, E>>.
May fit a future `copy_body_stream_to_stage` helper if multiple async backends share the body-copy loop.
```

### `bytes_stream()` costs now

```text
Requires enabling reqwest `stream` feature.
Pulls futures-util/tokio-util dependencies through reqwest feature.
Adds abstraction before Pulith has more than one async streaming backend.
Does not solve Range/If-Range/Content-Range logic.
```

Recommendation:

```text
Keep `chunk()` for the first resume/range implementation.
Do not enable reqwest `stream` yet.
Revisit `bytes_stream()` only when progress/throttle/checksum adapters or a second async backend need shared stream copying.
```

## How resume should integrate with current chunk loop

### Request phase

Before consuming body, decide request type:

```text
Full request:
  GET url

Resume request:
  GET url
  Range: bytes=<partial_len>-
  If-Range: <etag or http-date>  // only when validator is acceptable
```

This is independent of `chunk()`.

### Response phase

Before entering body loop, classify response:

```text
200 OK
206 Partial Content
416 Range Not Satisfiable
other status
```

Decision table:

```text
200 to normal GET:
  write fresh stage from byte 0.

206 to resume GET:
  require Content-Range start == partial_len;
  append body to partial stage.

200 to resume GET:
  server ignored Range or If-Range failed;
  discard partial;
  restart full download into fresh stage.

416 to resume GET:
  partial is stale/invalid for current representation;
  discard partial;
  restart full once or error by policy.

206 missing/wrong Content-Range:
  protocol failure; do not append; do not persist.
```

Only after this classification should Pulith call:

```rust
while let Some(chunk) = response.chunk().await { ... }
```

### Stage phase

A first resume-aware staging shape can stay simple:

```rust
enum StageMode {
    Fresh,
    Append { expected_start: u64, total: Option<u64> },
}
```

or typed state:

```text
StagedDownload<FreshOpen>
StagedDownload<AppendOpen>
StagedDownload<Closed>
```

Minimal implementation direction:

```text
Keep StagedDownload private.
Add a constructor for fresh staging and a constructor for validated append staging.
Append constructor must know partial_len and Content-Range start before body write.
Only Closed can persist.
```

Avoid public partial material types in first slice.

## Does stream behavior affect retry semantics?

Yes, but only at the body-failure boundary.

Current behavior:

```text
if response.chunk().await errors mid-body:
  record bytes written for the attempt;
  retry starts a new request from byte 0.
```

With resume enabled:

```text
if body stream errors mid-body and policy allows resume:
  the next attempt may request Range from the validated partial byte length,
  but only if the partial file and validator are safe.
```

Important distinction:

```text
stream/chunk error does not itself imply resume is safe.
resume is safe only if partial material is validated and the server confirms the requested range through 206 + Content-Range.
```

So the chunk loop must report:

```text
bytes_written_this_attempt
partial_total_bytes_available
whether partial is still safe to keep
```

This should become operation record/resume record data before final net-owned error design.

## Sync ureq compatibility

The sync path currently uses:

```rust
Read -> copy_response_body -> Write
```

This is equivalent to a blocking stream loop. It is also compatible with resume/range because the same decision table applies:

```text
request headers first;
classify status/Content-Range before body copy;
copy reader into fresh or append stage;
never persist failed partial as final.
```

Therefore the resume model should be backend-common:

```text
status/header classification + stage policy shared conceptually;
body copy primitive backend-specific: ureq Read loop vs reqwest chunk loop.
```

Do not design resume only around async stream traits.

## Runtime plan

### Current plan remains correct

```text
reqwest backend = Tokio-backed.
runtime-tokio feature owns Tokio dependency.
Pulith library does not create runtime.
caller supplies/owns runtime when using AsyncAcquireNode.
ReqwestResource holds reqwest::Client and injected async delay provider.
```

### Do not hide runtime selection behind `async`

`async` remains a modality axis, not a runtime promise:

```text
async = async behavior traits exist
runtime-tokio = Tokio-backed support is enabled
reqwest = net + runtime-tokio + reqwest backend
```

This is still the right feature structure.

### Future runtime-neutral backend

If runtime-neutral async HTTP is desired, make it a separate backend:

```text
isahc backend candidate, because its async API is runtime-agnostic/curl-backed.
```

Do not try to make `ReqwestAcquire` runtime-neutral.

Future shape:

```toml
isahc = ["net", "async", "dep:isahc"]
```

and a separate type:

```rust
IsahcAcquire
IsahcResource
```

not hidden under:

```rust
ReqwestAcquire
```

### Compio/smol/etc.

Keep as future separate backend families only:

```text
smol-native backend: possible but not via reqwest.
compio: interesting completion-based future backend, likely changes staging design.
monoio/glommio: specialized, not baseline.
async-std: rejected for new work due discontinued/deprecated status.
```

## Updated next implementation slice

Do not start with `bytes_stream()`.

Next slice should be docs/tests for resume behavior using current primitives:

```text
1. Add NetResumePolicy shape in design/tests.
2. Add RED tests around 200/206/416 response classification.
3. Keep current chunk loop but move it behind response classification.
4. Add private append/fresh staging distinction.
5. Only after resume records stabilize, design net-owned errors.
```

Suggested RED tests:

```text
reqwest_resume_206_appends_after_valid_content_range
reqwest_resume_200_to_range_restarts_full_with_fresh_stage
reqwest_resume_416_restarts_once_without_persisting_partial
reqwest_resume_missing_content_range_rejects_without_append
ureq_resume_206_appends_after_valid_content_range
ureq_resume_200_to_range_restarts_full_with_fresh_stage
```

## Final answer

```text
Does async stream behavior affect resume design?
  Yes at the body-failure boundary, because a mid-stream failure can become a resume candidate.
  But stream mechanics do not define resume correctness.

Is current chunk design compatible?
  Yes. `Response::chunk().await` is compatible as the body-copy primitive.
  It must be preceded by Range/If-Range request construction and 200/206/416 + Content-Range classification.

Is bytes_stream needed?
  No for the first resume slice. It adds feature/dependency surface and does not solve resume semantics.

Runtime plan?
  Keep reqwest explicitly Tokio-backed, caller-owned runtime, no hidden runtime creation.
  Future runtime-neutral async HTTP should be a separate backend such as IsahcAcquire, not a mode inside ReqwestAcquire.
```
