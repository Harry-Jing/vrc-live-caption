# Architecture

## Scope

This document describes runtime seams, normalized semantics, and data flow. It
intentionally does not define Rust module names, Vue store names, Tauri command
names, database schema, or provider-specific wire protocols.

## Status Convention

The explicitly named current flow and current wire contract describe code that
exists today. The remaining seams and policies are the accepted target
architecture unless a section is marked provisional or research-only. Their
implementation order and status live in [roadmap.md](./roadmap.md).

## High-Level Flow

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
translation path may consume normalized source text or audio directly. The
architecture does not require every translation to wait behind STT.

The currently implemented Phase 1 path is deliberately smaller:

```text
Microphone
  -> application-bounded speech segment
  -> OpenAI /v1/audio/transcriptions
  -> transcript-final event
  -> App preview
  -> completed Chatbox publication
  -> OSC
```

That event is semantically a completed source result, but the current wire does
not yet use the target generation/lane/full-snapshot state shape. The path is a
working baseline, not the capability ceiling for future cloud or local
providers.

## Core Boundaries

- The frontend does not process raw audio.
- Capture produces provider-independent frames or bounded spans; provider chunk
  sizes, VAD rules, commits, and endpointing do not leak into the frontend.
- Provider raw events never reach UI-facing consumers or output sinks.
- A provider adapter never publishes directly to Chatbox.
- Chatbox is an output sink, not the center of the runtime.
- Publication eligibility and sink pacing are separate decisions.
- Translation never blocks capture or recognition.
- Local inference runs out of process behind a Rust worker or sidecar seam.
- Runtime failures are categorized, visible, and never silently change provider,
  model, backend, publication mode, or content selection.

## Target Recognition Session Seam

The target main runtime faces one conceptual session:

```text
provider-independent audio in
  -> zero or more ongoing caption snapshots
  -> one completed snapshot per completed caption unit, when supported
```

Concrete adapters hide their very different mechanics:

- a bounded adapter may use local VAD, recognize one completed span, and emit
  only a completed snapshot;
- a streaming adapter may accept frames continuously, emit revisable snapshots,
  detect an endpoint, emit a completed snapshot, and reset its internal stream;
- a continuous path without per-unit completion may emit ongoing snapshots but
  cannot claim Completed support.

Do not implement one large universal model adapter. SenseVoice, streaming
Paraformer, streaming Zipformer, bounded OpenAI transcription, Realtime
transcription, and direct Realtime translation may each have separate concrete
implementations. Variants that differ only by size or parameters may share one
family adapter after their behavior is proven identical.

Capabilities belong to the complete provider path: provider, endpoint/session
mode, model, runtime, backend, and relevant configuration. A small capability
profile is sufficient:

| Dimension | Example values |
|---|---|
| Input shape | completed segment, committed items, continuous frames |
| Boundary owner | application, provider, hybrid, none |
| Update behavior | completed only, ongoing plus completed, ongoing only |
| Revision behavior | revisable snapshot, append-only |
| Produced lanes | source, translation, both |
| Optional features | timestamps, language detection, hotwords |

Provider-specific stable-prefix data may remain inside an adapter. It is not a
third application-wide caption state. Two-pass is future pipeline topology, not
a `supports_two_pass` model flag.

The OpenAI mappings and documentation uncertainties are recorded in
[research/openai-speech-streaming-options.md](./research/openai-speech-streaming-options.md).

## Current Wire Contract And Migration

The implemented UI-facing event concepts map to concrete Tauri event names as
follows:

| Semantic concept | Current Tauri event | Current meaning |
|---|---|---|
| `runtime.status` | `runtime-status` | `idle`, `starting`, `running`, `stopping`, `stopped`, or `error` |
| `utterance.started` | `utterance-started` | confirmed speech activity before transcript text exists |
| `transcript.partial` | `transcript-partial` | revisable text supported by the wire and mock path; the real bounded OpenAI adapter does not emit it |
| `transcript.final` | `transcript-final` | completed result from the current bounded OpenAI adapter |
| `utterance.ended` | `utterance-ended` | a unit ended without a final result: no speech, STT failure, or discard |
| `diagnostic` | `diagnostic-event` | categorized report with a stable code and English fallback text |

The concrete event names remain valid Tauri identifiers even when architecture
discussion uses dotted semantic names. Diagnostic codes follow
`<category>.<detail>`; the code is the localization and test contract, while
message and detail are fallback/debug prose.

The current transcript payload already carries an utterance id, revision, and
partial/final kind, but not generation, lane, or the target named state field.
There is no emitted `transcript-stable` event. The Rust enum reservation for
`Stable` is unused and should be removed during the versioned migration rather
than given a new application meaning.

Runtime lifecycle events are not replaced by caption snapshots. In particular,
`utterance.ended` remains necessary for no-result and failed units, while
`runtime.status` remains the pull/push resynchronization boundary.

## Target Caption Snapshot Contract

Provider adapters absorb raw delta, append, replacement, item, and ordering
rules. Downstream consumers receive the full current text for a lane, not an
operation they must reconstruct independently.

A normalized snapshot conceptually carries:

- session generation;
- session/stream correlation identity;
- application-owned caption-unit identity when real units exist;
- source or translation lane;
- full current text and monotonic revision;
- state: ongoing or completed;
- source unit and revision identity for transcript-driven translation;
- provider/model metadata and timing only when they are actually known.

An ongoing snapshot may replace earlier text in the same correlation scope: a
caption unit when one exists, otherwise the ongoing stream/lane. A completed
snapshot closes that adapter's real caption unit. Session termination is a
separate lifecycle event and never stands in for unit completion.

Before the first new streaming adapter ships, version the wire contract with
explicit generation, stream correlation, lane, revision, optional unit, and
ongoing/completed fields and pin the Rust/frontend mapping with wire-format
tests.

Future two-pass work may add a separate authority dimension. It must not
overload ongoing/completed or reintroduce a partial/stable/final ladder.

Transcript text never contains presentation placeholders. The UI derives
listening, translating, degraded, and failure state from lifecycle and health
events.

UI event delivery remains best-effort and at-most-once. The UI derives its view
from the newest status and snapshots; runtime lifecycle never depends on a
successful webview emit.

## Runtime Lifecycle

Start first validates configuration, credentials, audio-device availability,
and the requested provider plan. Configuration, credential, or microphone
failure prevents capture from starting. A safe per-unit provider, translation,
or OSC failure emits a diagnostic and may leave the session running; a session-
level failure moves the runtime to an explicit error/stopped state.

The target planner resolves one concrete plan before capture begins: provider
path, model, effective backend, publication mode, and selected content lanes.
An incompatible combination is never silently changed. The App offers two
directions: keep the selected model/provider and choose a supported mode, or
keep the requested experience and choose a compatible model/provider.

Stop is a hard generation boundary:

- release the microphone promptly;
- discard buffered and queued audio;
- cancel or close provider and translation work where possible;
- discard pending Chatbox drafts, completed pages, and translations;
- reject every late snapshot from the stopped generation for both App and
  Chatbox;
- allow only a typing-off cleanup message after the stop request.

An uncancellable request may finish during cleanup, but its result is ignored
and the discard is visible in diagnostics.

The current Completed publisher implements Stop and a runtime-fatal close as
discard, not drain. Closing admission interrupts any pending pacing wait,
discards every resident page including the remainder of a unit whose
publication has begun, attempts the one typing-off cleanup, and then joins the
publisher worker. Closing never publishes queued caption text.

Typing indication follows normalized speech or pending publication activity,
not provider completion alone. It turns off after successful resolution, a
unit ending without text, a safe failure, and Stop. While activity remains
active, the publisher reasserts typing-on every four seconds because the
VRChat client hides an unrefreshed indicator after about five seconds. Typing
control packets remain outside the process-wide text pacer. Caption publication
and typing cleanup remain independently testable.

## Chatbox Publication Modes

Provider path, publication mode, and content selection are independent:

- **Completed** publishes completed caption units only;
- **Live** may publish ongoing snapshots and later completed corrections;
- content is source only, translation only, or bilingual.

There is no public Automatic publication mode. Compatibility is resolved from
the lanes the user selected:

| Selected-lane behavior | Completed | Live |
|---|---|---|
| Completed snapshots only | supported | unsupported: no rolling text exists |
| Ongoing and completed snapshots | supported | supported |
| Ongoing snapshots without unit completion | unsupported | supported |

For bilingual Live, one lane may progress ahead of the other. Recognition-side
compatibility is settled by the table above. Translation-only Live remains a
provisional product mapping until concrete translators are benchmarked: a path
that returns only one complete target cannot update during speech, while the
usefulness of token streaming that begins only after a pause still needs user
testing. It must not be simulated by repeatedly submitting unstable source
partials. If a requested combination is incompatible, the App offers the two
explicit alternatives described in Runtime Lifecycle.

Live is a deliberately lossy current view rather than transcript history. The
App retains normalized state; Chatbox may skip or replace intermediate revisions
to remain current and readable.

## Independent Chatbox Publisher

The Phase 1 Completed publisher is implemented as an independent worker.
Runtime producers submit whole caption-unit lifecycle events without waiting
for a pacing opportunity or OSC; Completed text is paginated before queue
admission. The worker owns typing transitions, ordered Completed publication,
overload handling, and diagnostics, and uses the shared process-wide pacer for
actual text-send attempts. Capture and provider ingestion never wait for
Chatbox pacing. Live policy remains a Phase 3 extension of this publisher rather
than current behavior. Publisher instances are Runtime-generation scoped; only
the process-wide pacer survives Stop/Start, so an old Publisher handle can never
submit into a new generation while the new generation still respects the old
generation's most recent actual send attempt.

Project publication rules:

- measure at least `1000 ms` from the previous actual text-send attempt; do not
  exploit the initial leaky-bucket burst;
- a failed OSC attempt also consumes the pacing opportunity, preventing a retry
  storm;
- provider deltas are never forwarded one-for-one to OSC.

The following eligibility rules remain target Live behavior and are not part of
the current Completed implementation:

- for a path with real caption units, App preview may update during a unit's
  first second but Chatbox waits; if the unit completes in that interval, only
  its completed text is sent;
- for an ongoing-only unitless path, the App still updates immediately and the
  publisher waits one second after the stream's first non-empty snapshot before
  its first Chatbox send. It then remains Live for that stream; silence or a
  timer never creates completion, and any future activity-reset heuristic
  requires separate provider-specific testing;
- after that observation window, Live sends the newest eligible snapshot at
  each opportunity and discards obsolete unsent revisions;
- a completed correction replaces an unsent draft and need not be resent when
  it is identical to the last published view.

### Target Live rendering

Live uses one recomputed rolling viewport, not a queue of historical screens:

- preserve as much recent context as fits, but always keep the newest content;
- advance at word, punctuation, line-break, or grapheme boundaries;
- source and translation keep independent progress watermarks;
- bilingual output renders source above translation and combines the latest
  available view of each lane in one Chatbox message;
- source may lead translation; available matching context is retained when
  space permits, but strict one-to-one alignment never blocks fresher text;
- once both lanes have text, both receive visible capacity; remaining capacity
  is shared dynamically with a modest default preference for translation.

Normal translation delay may leave the target lane one caption unit behind. If
translation explicitly fails, the user's bilingual selection remains unchanged,
the App reports a degraded translation state, and new Chatbox snapshots omit
the stale target rather than presenting it as a translation of newer source
text. A previously published coherent bilingual message need not be cleared
until newer source text is sent.

### Current Completed rendering

Completed output preserves stable content rather than truncating it:

- shape and paginate at legal grapheme and line-break boundaries;
- send pages and distinct caption units in order;
- keep adjacent completed units distinct initially; any later merge rule is a
  conditional reading-time optimization that requires measured approval and
  must preserve unit identity and meaning;
- keep the queue bounded by page count and age;
- only under sustained exceptional overload may the publisher drop the oldest
  whole caption units that have not begun publication;
- never drop arbitrary middle pages; retain complete text in the App and emit a
  diagnostic when Chatbox content is dropped.

The current Phase 1 queue policy uses internal, non-user-configurable limits:

- at most `32` resident pages that have not yet been sent successfully;
- at most `30` seconds of residence for a unit that has not begun publication;
- a unit becomes started at its first actual text-send attempt, not when it is
  accepted, selected, or waiting for a pacing opportunity;
- overload removes the oldest whole unstarted units until the new unit fits; if
  the new unit cannot fit without splitting itself or displacing a started unit,
  it is rejected as a whole;
- a failed text-send attempt is not retried. It consumes the pacing opportunity,
  aborts that unit, and discards the failed and remaining pages while later
  units may continue;
- Stop and runtime-fatal close discard all resident pages without draining them,
  then attempt the one allowed typing-off cleanup.

The `32`-page and `30`-second limits are provisional safety bounds. Phase 1
real-machine VRChat validation must measure backlog and readability and adjust
them before they are treated as settled product limits.

All output respects VRChat's 144-character input cap, at most nine visible
lines, real glyph-width wrapping, and grapheme-safe clipping. The implementation
uses a conservative UTF-16 budget and the fixed layout model documented in
[research/vrchat-chatbox-reference.md](./research/vrchat-chatbox-reference.md).

## Target Translation Boundary

The architecture keeps two topologies separate:

- transcript-driven translation consumes completed or revising source
  snapshots and links every target result to the source unit and revision it
  translated;
- experimental direct speech translation consumes audio and may produce a
  target lane plus an optional source lane. This is a research candidate, not a
  requirement for the first translation implementation.

Translation never blocks capture or recognition. Stale target work cannot
overwrite a newer source revision or another caption unit.

The first implementation should translate completed source text unless the
selected provider natively produces useful ongoing target revisions. Repeatedly
calling an ordinary translator for every unstable ASR revision is deferred: it
creates request amplification, races, cost, and large visible rewrites.

The complete mapping of translation update shapes to Live is provisional. Token
streaming from a fixed completed source may feed ongoing target snapshots while
Completed waits for target completion, but that experience begins after a pause
and may not meet users' expectation of Live. The UI must disclose whether a
translator updates during speech, starts streaming after a pause, or returns one
complete result. Revisit the public compatibility rule after concrete local and
cloud translators are benchmarked.

## Target Local Inference Boundary

Local inference means a Rust application and Rust worker using packaged native
libraries without Python, PyTorch, Conda, or developer tooling. The underlying
runtime may contain C/C++ and ONNX Runtime.

The first local implementation is single-pass. Only one STT model and one
effective backend are loaded for a recognition session. CPU is the first
compatibility implementation because it is simplest to package; this is an
engineering order, not a claim that CPU is best for every VRChat machine.
NVIDIA CUDA follows in the same local-STT program after the CPU worker boundary
works, and is benchmarked per model.

The global backend preference is CPU or prefer NVIDIA GPU (CUDA). No automatic
performance selector is planned now. A session records and displays its
effective backend separately:

- an unsupported GPU/model combination uses CPU and exposes the reason;
- a CUDA startup failure may use CPU only with a clear warning;
- a running worker crash never changes backend automatically;
- after a crash, the user explicitly retries the same backend or selects
  another one;
- local failure never uploads audio to a cloud provider without explicit user
  action.

Two-pass remains a very low-priority future experiment after the main speech,
translation, model-management, and benchmarking work is mature. The current
architecture reserves correlation identities but does not implement a second
recognizer, preview authority, or two-pass settings.

## Incoming Caption Boundary

The architecture does not assume microphone-only input forever. System or
VRChat audio can later enter as a separate incoming pipeline with its own
caption lanes and publication policy.

The current MVP does not implement incoming capture, speaker diarization, or
overlapping-speaker handling.

## Lessons From The Python Prototype

- normalize provider events before runtime consumers see them;
- keep Chatbox publication, pacing, and shaping separate from providers;
- keep translation independent from capture and recognition lifecycle;
- use fake providers and opt-in live tests;
- make diagnostics explicit for audio, config, provider, translation, worker,
  backend, and OSC failures.

The Python package structure, asyncio lifecycle details, CLI contracts, sidecar
protocols, and provider implementation details are not architecture constraints.
