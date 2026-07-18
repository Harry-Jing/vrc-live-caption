# Architecture

## Scope

This document describes runtime seams, normalized semantics, and data flow —
where the boundaries are and what crosses them. How each mechanism works
inside a boundary lives in the code and its tests, which are authoritative.
This document does not define Rust module names, Vue store names, Tauri
command names, database schema, or provider wire protocols.

## Status convention

Sections marked current describe code that exists today; everything else is
accepted target architecture. Implementation status lives in
[roadmap.md](./roadmap.md).

## High-level flow

```text
Audio Sources
  -> capture / provider-independent audio
  -> Provider Path Adapter
  -> Normalized Caption Snapshots
  -> Optional Translation
  -> Publication Policy
  -> Output Sink Publishers
```

A provider path may produce a source lane, a translated lane, or both. A
translation path may consume normalized source text or audio directly; the
architecture does not require every translation to wait behind STT.

## Core boundaries

- The frontend does not process raw audio.
- Capture produces provider-independent frames or bounded spans; provider
  chunk sizes, VAD rules, commits, and endpointing do not leak into the
  frontend.
- Provider raw events never reach UI-facing consumers or output sinks
  ([ADR 0018](./adr/0018-adapters-emit-full-snapshots-not-deltas.md)).
- A provider adapter never publishes directly to Chatbox. Chatbox is an
  output sink, not the center of the runtime.
- Publication eligibility and sink pacing are separate decisions.
- Translation never blocks capture or recognition.
- Local inference runs out of process behind a Rust worker seam
  ([ADR 0003](./adr/0003-keep-local-inference-out-of-process.md)).
- Runtime failures are categorized, visible, and never silently change
  provider, model, backend, publication mode, or content selection.

## Recognition session seam

The runtime faces one conceptual session:

```text
provider-independent audio in
  -> zero or more ongoing caption snapshots
  -> one completed snapshot per completed caption unit, when supported
```

Concrete adapters hide very different mechanics: the current bounded OpenAI
adapter uploads one application-bounded audio unit and emits at most one
completed snapshot; a streaming adapter emits revisable snapshots and
completes units at endpoints; a continuous path may emit only ongoing
snapshots and cannot claim Completed support. Each behaviorally distinct
provider family gets its own concrete adapter — there is no universal model
adapter full of name branches.

Capabilities belong to the complete provider path — provider, endpoint or
session mode, model, runtime, backend, and relevant configuration:

| Dimension | Example values |
|---|---|
| Input shape | completed segment, committed items, continuous frames |
| Boundary owner | application, provider, hybrid, none |
| Update behavior | completed only, ongoing plus completed, ongoing only |
| Revision behavior | revisable snapshot, append-only |
| Produced lanes | source, translation, both |

A backend-owned planner resolves each publication request against these
facts. Currently the bounded OpenAI path is completed-only and deterministic
Mock profiles cover the other shapes. An incompatible plan preserves the
request and reports the supported alternatives
([ADR 0014](./adr/0014-publication-timing-is-completed-or-live.md)).

The OpenAI endpoint facts and open questions are in
[research/openai-speech-streaming-options.md](./research/openai-speech-streaming-options.md).

## Current UI-facing contracts

| Semantic concept | Tauri event | Meaning |
|---|---|---|
| `runtime.status` | `runtime-status` | `idle`, `starting`, `running`, `stopping`, `stopped`, or `error` |
| `utterance.started` | `utterance-started` | speech activity before caption text exists |
| `caption.session.changed` | `caption-session-changed` | the newest full `CaptionSessionSnapshotV1` aggregate |
| `utterance.ended` | `utterance-ended` | a unit ended without a final result |
| `diagnostic` | `diagnostic-event` | categorized report with a stable code ([ADR 0006](./adr/0006-diagnostic-codes-are-category-detail.md)) |

Rust owns one versioned caption-session aggregate,
`CaptionSessionSnapshotV1`: a monotonic aggregate revision, backend-assigned
generation and stream identity, active units, and full-text caption snapshots
with lane, per-scope revision, and ongoing/completed state
([ADR 0018](./adr/0018-adapters-emit-full-snapshots-not-deltas.md)).

Event delivery is best-effort and at-most-once; the frontend can always pull
the same aggregate to resynchronize, and reducers ignore older revisions
([ADR 0007](./adr/0007-event-delivery-is-best-effort.md)). A shared JSON
fixture pins the Rust serialization and the TypeScript runtime decoder to the
same V1 wire shape. Admission, ordering, and reload-race handling live in the
reducers and their tests.

Caption text never contains presentation placeholders; the UI derives
listening, translating, degraded, and failure states from lifecycle and
health events.

## Runtime control snapshot

Saved settings and the running session are separate state
([ADR 0019](./adr/0019-saved-settings-are-not-the-running-session.md)). Rust
owns one revisioned, redacted control snapshot: the desired configuration,
the backend-derived publication plan, secret status, lifecycle status, and
the immutable selection captured by the current generation. Control
mutations return the resulting snapshot; pending-change indicators are
derived by comparing desired and active state. The Stop-versus-Start
ordering guarantees live in the runtime code and its tests.

## Runtime lifecycle

Start validates configuration, credentials, audio devices, and the requested
plan before capture begins. An incompatible combination is explained with
explicit alternatives, never adjusted silently. A safe per-unit provider,
translation, or OSC failure emits a diagnostic and may leave the session
running; a session-level failure moves the runtime to an explicit error
state.

Stop is a hard generation boundary
([ADR 0008](./adr/0008-stop-is-a-hard-cutoff.md)): release the microphone,
discard buffered and queued work, reject every late snapshot from the stopped
generation for both App and Chatbox, and allow only one typing-off cleanup
message. Publishers discard their queues; they do not drain them.

Typing indication follows speech and publication activity, reasserted every
four seconds while activity continues
([ADR 0013](./adr/0013-signal-speech-activity-with-the-typing-indicator.md)),
and stays outside the text pacer.

## Chatbox publication modes

Provider path, publication mode, and content selection are independent
([ADR 0014](./adr/0014-publication-timing-is-completed-or-live.md)):

| Selected-lane behavior | Completed | Live |
|---|---|---|
| Completed snapshots only | supported | unsupported: no rolling text exists |
| Ongoing and completed snapshots | supported | supported |
| Ongoing snapshots without unit completion | unsupported | supported |

Live is a deliberately lossy current view rather than transcript history: the
App retains complete normalized state while Chatbox may skip or replace
intermediate revisions. Translation-only Live remains provisional until real
translators are benchmarked (roadmap Phase 6).

## Chatbox publishers

Two independent workers sit behind one closed publication facade and share
one process-wide pacer that enforces the 1000 ms actual-attempt interval
([ADR 0015](./adr/0015-pace-chatbox-sends-at-one-second.md)). Publisher
instances are generation-scoped; only the pacer survives Stop/Start.

- The **Completed** worker paginates completed units and publishes an
  ordered, bounded queue without blocking producers. Under sustained overload
  it may drop only whole oldest units that have not started publication, with
  a diagnostic. The queue limits and full policy are in
  [research/vrchat-chatbox-reference.md](./research/vrchat-chatbox-reference.md).
- The **Live** worker observes store-accepted aggregates and recomputes one
  latest-wins recent-content viewport; it never queues historical screens. A
  unit-based path observes the unit's first second before rolling; a unitless
  path waits one second after the stream's first non-empty snapshot. A
  completed correction replaces an unsent draft and is skipped when identical
  to the last published view.

All output obeys the 144 UTF-16-unit budget, at most nine visible lines,
real glyph-width wrapping, and grapheme-safe boundaries, per the layout model
in [research/vrchat-chatbox-reference.md](./research/vrchat-chatbox-reference.md).

Bilingual and translation-aware rendering follow
[ADR 0016](./adr/0016-bilingual-output-is-one-asynchronous-view.md) and are
built together with the first concrete translator.

## Target translation boundary

Two topologies stay separate: transcript-driven translation consumes
completed or revising source snapshots and links every target result to the
source unit and revision it translated; direct speech translation consumes
audio and remains a research candidate. The first implementation translates
completed source text — repeatedly calling an ordinary translator for every
unstable ASR revision is rejected because it creates request amplification,
races, cost, and visible rewrites. Stale target work can never overwrite a
newer source revision or another caption unit.

## Target local inference boundary

Local inference is a Rust application and Rust worker using packaged native
libraries, with no Python, PyTorch, or Conda
([ADR 0003](./adr/0003-keep-local-inference-out-of-process.md)). One STT
model and one effective backend are loaded per recognition session; the
backend preference and effective-backend rules are
[ADR 0020](./adr/0020-users-choose-the-local-backend.md). A worker crash
stops the session and waits for an explicit user decision. Candidate models
and backend facts are in
[research/local-inference-notes.md](./research/local-inference-notes.md).

## Incoming caption boundary

System or VRChat audio can later enter as a separate incoming pipeline with
its own caption lanes and publication policy. Nothing in the runtime assumes
microphone-only input forever, but no incoming capture, diarization, or
overlapping-speaker handling is implemented (roadmap Later).
