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
  -> RecognitionSession Adapter
  -> Normalized Caption Snapshots
  -> Optional Translation
  -> Publication Policy
  -> Output Sink Publishers
```

A recognition Adapter produces the source lane. A translation path may consume
normalized source text or audio directly; the architecture does not require
every translation to wait behind STT.

## Core boundaries

- The frontend does not process raw audio.
- Capture produces provider-independent frames or bounded spans; provider
  chunk sizes, VAD rules, commits, and endpointing do not leak into the
  frontend.
- Provider raw events never reach UI-facing consumers or output sinks
  ([ADR 0010](./adr/0010-adapters-emit-full-snapshots-not-deltas.md)).
- Provider-authored error messages and metadata are discarded inside the
  Adapter. Only an allowlisted application classification and stable,
  application-authored diagnostic text may cross the seam.
- A provider adapter never publishes directly to Chatbox. Chatbox is an
  output sink, not the center of the runtime.
- Publication eligibility and sink pacing are separate decisions.
- Translation never blocks capture or recognition.
- Local inference runs out of process behind a Rust worker seam
  ([ADR 0020](./adr/0020-keep-local-inference-out-of-process.md)).
- Runtime failures are categorized, visible, and never silently change
  provider, model, backend, publication mode, or content selection.

## Recognition session seam

**Current implementation.** The recognition Module presents one deep
`RecognitionSession` Interface to the runtime:

```text
capture
  -> RecognitionSession: provider-independent audio and lifecycle controls
       -> OpenAI Adapter: Realtime WebSocket protocol
       -> future Local Adapter: out-of-process worker protocol
  -> normalized lifecycle, caption snapshots, and categorized errors
```

The Interface accepts an immutable, already-planned session selection,
provider-independent mono audio frames with their source format, input-end, and
Stop. It emits unit-started and unit-ended lifecycle events, zero or more
ongoing full-text snapshots, at most one completed full-text snapshot per
caption unit, and categorized recoverable or terminal errors. It does not
expose a URL, JSON event, audio-buffer commit, provider item identifier, or
worker message.

These invariants hold on both sides of the seam:

- one runtime generation owns exactly one provider, Adapter selection, and
  model; saved changes take effect only on a later Start. A generation may
  replace a failed provider connection without changing that selection;
- each unit has one stable normalized identity, revisions increase within its
  lane, and completion is terminal for that unit;
- raw deltas are accumulated inside the Adapter and leave it only as full
  snapshots ([ADR 0010](./adr/0010-adapters-emit-full-snapshots-not-deltas.md));
- provider events that complete out of order are correlated to their original
  units before admission; unit sequence follows input/commit order, and a
  missing or failed bound unit ends explicitly so later terminal results can
  advance without attaching to or overtaking the wrong unit; an unacknowledged
  commit fails the whole session because its later provider identity cannot be
  attached safely;
- recoverable item failure may end that item and keep the connection running;
  explicitly transient connection or provider availability failures enter a
  visible reconnect loop, while authentication, permission, invalid-request,
  usage-limit, proxy-policy, TLS-configuration, protocol, worker, and unknown
  failures end the generation visibly. Neither kind changes provider, model,
  or publication timing; and
- the runtime generation gate rejects all output after Stop, including a
  provider's drained tail or a local worker's late response.

Concrete Adapters hide behaviorally different mechanisms. The OpenAI Module
owns two release paths behind the same Interface:

| Catalog entry | Hidden input/event behavior | Declared publication timing |
|---|---|---|
| `openai/gpt-transcribe` | append 24 kHz PCM, commit one item, then await its completed transcript; preserve provider-detected language separately from input hints | Completed |
| `openai/gpt-live-transcribe` | append 24 kHz PCM continuously; normalize transcript deltas and completion | Completed or Live |

Both use an OpenAI Realtime transcription WebSocket, and both reconcile raw
events by `item_id` and receive expected-language hints through `languages[]`.
Hints never masquerade as detected language. An `item_id` never becomes a
UI-facing or output-sink identity. `gpt-transcribe` does not produce a
fabricated ongoing snapshot while the committed item is being recognized.
These protocol facts are recorded in
[research/openai-speech-streaming-options.md](./research/openai-speech-streaming-options.md)
and the decision is [ADR 0024](./adr/0024-use-openai-realtime-transcription.md).

There is deliberately no generic WebSocket abstraction. Connection setup,
authentication, system-proxy tunneling, JSON encoding, 24 kHz PCM conversion,
append/commit sequencing, `item_id` bookkeeping, session-failure policy, and
OpenAI error decoding are hidden implementation of the OpenAI Module. The
recognition Module depends only on the semantic Interface. A future local
Adapter instead hides worker startup, model loading, frame transport, and
crash mapping.

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
facts. The release catalog uses exact path identifiers and capability records,
not arbitrary model strings or model-name heuristics. An incompatible or
removed selection preserves the request and reports the supported alternatives
([ADR 0006](./adr/0006-publication-timing-is-completed-or-live.md)).

The implementation has no REST/WAV recognition fallback, legacy OpenAI model
compatibility, or production Mock provider. Scripted RecognitionSession
Adapters remain test-only and are selected by dependency injection rather than
configuration.

Transport libraries are replaceable OpenAI-Module dependencies, not
architecture Interfaces. The concrete WebSocket, TLS, Base64, and system-proxy
libraries are private OpenAI-Module dependencies. The removed direct HTTP
client and WAV encoder are not retained for hypothetical future translation.
The current transport honors a selected manual system HTTP proxy, rejects
malformed explicit, Windows protocol-mapped, and macOS manual proxy settings
instead of silently connecting directly, and never falls back to direct after
a selected proxy fails. macOS resolves the actual target through CFNetwork so
`ExceptionsList` and `ExcludeSimpleHostnames` retain Apple semantics; an
unpaired environment `NO_PROXY` cannot override that operating-system route.
Unsupported SOCKS and PAC/WPAD selections fail visibly. The relay/base-URL
option remains later work under
[ADR 0019](./adr/0019-follow-system-proxy-plan-relay-api.md).

Hostname resolution is a shared application boundary for OpenAI target/proxy
hosts and OSC targets. It has a monotonic deadline and observes the relevant
Stop cancellation signal, so Start and Stop do not wait indefinitely on name
resolution. TCP connect, proxy CONNECT, and TLS/WebSocket handshakes retain
their separate existing timeouts. OpenAI name resolution completes before the
microphone is opened.

## Current UI-facing contracts

| Semantic concept | Tauri event | Meaning |
|---|---|---|
| `runtime.status` | `runtime-status` | `idle`, `starting`, `running`, `reconnecting`, `stopping`, `stopped`, or `error` |
| `utterance.started` | `utterance-started` | speech activity before caption text exists |
| `caption.session.changed` | `caption-session-changed` | the newest full `CaptionSessionSnapshotV1` aggregate |
| `utterance.ended` | `utterance-ended` | a unit ended without a final result |
| `audio.level` | `audio-level` | generation/revision-scoped 100 ms RMS/peak/gate/clipping scalars; never PCM |
| `diagnostic` | `diagnostic-event` | categorized report with a stable code ([ADR 0014](./adr/0014-diagnostic-codes-are-category-detail.md)) |

Rust owns one versioned caption-session aggregate,
`CaptionSessionSnapshotV1`: a monotonic aggregate revision, backend-assigned
generation and stream identity, active units, and full-text caption snapshots
with lane, per-scope revision, and ongoing/completed state
([ADR 0010](./adr/0010-adapters-emit-full-snapshots-not-deltas.md)).
Unit-based captions are newest-unit-first by backend-accepted unit sequence;
later revisions keep their unit's position, and wall-clock timestamps are
metadata rather than ordering keys.

Event delivery is best-effort and at-most-once. The frontend can always pull
the caption aggregate to resynchronize, and reducers ignore older revisions
([ADR 0013](./adr/0013-event-delivery-is-best-effort.md)). Audio level telemetry
is deliberately ephemeral: the UI accepts only newer generation/revision pairs
and hides stale readings outside Running. A shared JSON fixture pins the Rust
caption serialization and TypeScript runtime decoder to the same V1 wire
shape. Admission, ordering, and reload-race handling live in reducers and
their tests.

Caption text never contains presentation placeholders; the UI derives
listening, translating, degraded, and failure states from lifecycle and
health events.

## Runtime control snapshot

Saved settings and the running session are separate state
([ADR 0012](./adr/0012-saved-settings-are-not-the-running-session.md)). Rust
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
running. A classified transient recognition failure closes capture, retires
and joins the old connection, ends unconfirmed units, and enters visible
`reconnecting` backoff within the same generation. A fresh connection starts
with no audio replay. A terminal session-level failure moves the runtime to an
explicit error state
([ADR 0025](./adr/0025-reconnect-within-one-runtime-generation.md)).

While Running, the backend derives fixed-window RMS/peak, gate, and clipping
scalars from the same mono capture frames used by recognition. Raw PCM never
crosses IPC. Settings also offers a short, local-only microphone probe that is
mutually exclusive with the runtime and does not open a provider, OSC, secret,
or persistence path.

Stop is a hard generation boundary
([ADR 0011](./adr/0011-stop-is-a-hard-cutoff.md)): release the microphone,
discard buffered and queued work, reject every late snapshot from the stopped
generation for both App and Chatbox, and allow only one typing-off cleanup
message. Publishers discard their queues; they do not drain them.

Typing indication follows speech and publication activity, reasserted every
four seconds while activity continues
([ADR 0016](./adr/0016-signal-speech-activity-with-the-typing-indicator.md)),
and stays outside the text pacer.

## Chatbox publication modes

Provider path, publication mode, and content selection are independent
([ADR 0006](./adr/0006-publication-timing-is-completed-or-live.md)):

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
[ADR 0007](./adr/0007-bilingual-output-is-one-asynchronous-view.md) and are
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
([ADR 0020](./adr/0020-keep-local-inference-out-of-process.md)). One STT
model and one effective backend are loaded per recognition session; the
backend preference and effective-backend rules are
[ADR 0021](./adr/0021-users-choose-the-local-backend.md). A worker crash
stops the session and waits for an explicit user decision. The local worker
Adapter implements the same `RecognitionSession` Interface as OpenAI while
keeping worker messages, native-runtime types, and model-specific streaming
state behind that seam. Candidate models and backend facts are in
[research/local-inference-notes.md](./research/local-inference-notes.md).

## Incoming caption boundary

System or VRChat audio can later enter as a separate incoming pipeline with
its own caption lanes and publication policy. Nothing in the runtime assumes
microphone-only input forever, but no incoming capture, diarization, or
overlapping-speaker handling is implemented (roadmap Later).
