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
  -> Active Recognition Module
  -> Caption Aggregate
  -> Optional Translation Module
  -> Caption Pipeline Plan
  -> UI and Output Sink Publishers
```

A Recognition Driver produces the source lane. A translation path may consume
normalized source text or audio directly; the architecture does not require
every translation topology to wait behind source recognition.

## Core boundaries

- The frontend does not process raw audio.
- Capture produces provider-independent frames or bounded spans; provider
  chunk sizes, VAD rules, commits, and endpointing do not leak into the
  frontend.
- Provider raw events never reach UI-facing consumers or output sinks
  ([ADR 0010](./adr/0010-adapters-emit-full-snapshots-not-deltas.md)).
- Provider-authored error messages and metadata are discarded inside the
  concrete driver. Only an allowlisted application classification and stable,
  application-authored diagnostic text may cross the seam.
- A recognition or translation driver never publishes directly to Chatbox.
  Chatbox is an
  output sink, not the center of the runtime.
- Publication eligibility and sink pacing are separate decisions.
- Translation never blocks capture or recognition.
- Local inference runs out of process behind a Rust worker seam
  ([ADR 0020](./adr/0020-keep-local-inference-out-of-process.md)).
- Runtime failures are categorized, visible, and never silently change a
  recognition path, translation path, effective backend, publication mode, or
  content selection.

## Active recognition seam

**Current implementation.** Runtime sees one deep, path-neutral active
Recognition Module:

```text
runtime coordinator
  -> start immutable generation scope and selected Recognition Module
  -> try-submit continuous owned mono audio (bounded, non-blocking)
  <- Ready / Reconnecting / normalized caption and unit signals
  -> acknowledge capture retirement / hard Stop out of band

Recognition Module
  -> selected Recognition Driver
     -> current OpenAI driver: unitization + attempts + Realtime protocol owner
     -> future local driver: model lifecycle + out-of-process worker protocol
```

The Module accepts an immutable, already-planned selection and continuous
provider-independent mono frames with capture sequence and timestamp. Runtime
does not manufacture input-end, commit, item, or inference-window commands.
The desktop composition boundary resolves the selected path's credential,
constructs the concrete Module, and binds it to the generation-facing
credential snapshot and microphone-upload disclosure as one prepared
recognition value. Runtime receives that value, not independently selectable
metadata, an OpenAI key, a provider factory, or a recognition transport
dependency.
It emits unit-started and unit-aborted lifecycle signals, zero or more ongoing
full-text snapshots, at most one completed full-text snapshot per caption unit,
and categorized recoverable or terminal errors. A completed Source snapshot is
the normal source-unit close; `unit-aborted` is reserved for no-speech or
failure paths. The Module does not expose a URL, JSON event, provider item
identifier, worker message, resampling window, or model-native tensor.

One owner executes the selected Driver inside the Module for a runtime
generation. Its input is
bounded by represented audio duration plus a frame-count safety ceiling rather
than capture callback count. An admitted frame carries an attempt epoch and an
RAII budget permit. At reconnect, admission closes and advances before queued
audio is discarded; runtime drops microphone capture and acknowledges that
exact retirement before a fresh attempt can become Ready. A racing old frame
therefore cannot be consumed by the new attempt. Stop is a separate wake path
that does not wait behind audio, connect, protocol, backoff, or future model
loading work ([ADR 0026](./adr/0026-recognition-modules-own-attempt-execution.md)).

Normalized signals preserve order in one bounded queue. Revisions of the same
ongoing caption coalesce latest-wins, while lifecycle control keeps reserved
capacity. Exhausting audio or durable-signal capacity fails visibly instead of
silently losing speech or a completed unit.

These invariants hold on both sides of the seam:

- one runtime generation owns exactly one recognition-path selection; saved
  changes take effect only on a later Start. A generation may replace a failed
  recognition attempt without changing that selection;
- each unit has one stable normalized identity, revisions increase within its
  lane, and completion is terminal only for that lane's revision chain;
- raw deltas are accumulated inside the Driver and leave it only as full
  snapshots ([ADR 0010](./adr/0010-adapters-emit-full-snapshots-not-deltas.md));
- provider events that complete out of order are correlated to their original
  units before admission; unit sequence follows input/commit order, and a
  missing or failed bound unit ends explicitly so later terminal results can
  advance without attaching to or overtaking the wrong unit; an unacknowledged
  commit fails the attempt because its later provider identity cannot be
  attached safely;
- recoverable item failure may end that item and keep the connection running;
  explicitly transient connection or provider availability failures enter a
  visible reconnect loop, while authentication, permission, invalid-request,
  usage-limit, proxy-policy, TLS-configuration, protocol, worker, and unknown
  failures end the generation visibly. Neither kind changes the recognition
  path or publication timing; and
- the runtime generation gate rejects all output after Stop, including a
  provider's drained tail or a local worker's late response.

Concrete Drivers hide behaviorally different mechanisms. The OpenAI Module
owns two release paths behind the same Module boundary:

| Catalog entry | Hidden input/event behavior | Declared publication timing |
|---|---|---|
| `openai/gpt-transcribe` | append 24 kHz PCM, commit one item, then await its completed transcript; preserve provider-detected language separately from input hints | Completed |
| `openai/gpt-live-transcribe` | append 24 kHz PCM continuously; normalize transcript deltas and completion | Completed or Live |

Both use an OpenAI Realtime transcription WebSocket, and both reconcile raw
events by `item_id` and receive expected-language hints through `languages[]`.
Hints never masquerade as detected language. An `item_id` never becomes a
UI-facing or output-sink identity. `gpt-transcribe` does not produce a
fabricated ongoing snapshot while the committed item is being recognized.
The release decision is
[ADR 0024](./adr/0024-use-openai-realtime-transcription.md).

There is deliberately no generic WebSocket abstraction. Connection setup,
authentication, system-proxy tunneling, JSON encoding, 24 kHz PCM conversion,
append/commit sequencing, `item_id` bookkeeping, recognition-attempt failure
policy, and OpenAI error decoding are hidden implementation details of the
OpenAI Module. Its
Network Owner permanently uses non-blocking established I/O and independently
advances bounded TLS reads, TLS writes, WebSocket data, Ping/Pong, Close, and
partial records. Audio submission never performs a fake-timeout socket read.
A future Local driver instead hides worker startup, model loading, bounded IPC,
frame transport, and crash mapping behind the same Recognition Module boundary.

Capabilities belong to the complete recognition path — provider, endpoint or
protocol mode, model, runtime, effective backend, and relevant configuration:

| Dimension | Example values |
|---|---|
| Input shape | completed segment, committed items, continuous frames |
| Boundary owner | application, provider, hybrid, none |
| Update behavior | completed only, ongoing plus completed, ongoing only |
| Revision behavior | revisable snapshot, append-only |
| Produced lanes | source, translation, both |

The application resolves a Caption Pipeline Plan for each publication request
against these facts. The release catalog uses exact path identifiers and
capability records, not arbitrary model strings or model-name heuristics. An
incompatible or removed selection preserves the request and reports the
supported alternatives
([ADR 0006](./adr/0006-publication-timing-is-completed-or-live.md)).

The implementation has no REST/WAV recognition fallback, legacy OpenAI model
compatibility, or production Mock provider. Scripted Recognition Drivers
remain test-only and are selected by dependency injection rather than
configuration.

Transport libraries are replaceable OpenAI-Module dependencies, not
architecture interfaces. The concrete WebSocket, TLS, Base64, and system-proxy
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

Hostname resolution is an application boundary for OpenAI target/proxy hosts
and OSC targets. Each of those two current network subsystems owns a separate
bounded resolver worker, so a stuck operating-system lookup cannot become a
shared failure domain. Resolution has a monotonic deadline and observes the
relevant Stop cancellation signal, so Start and Stop do not wait indefinitely
on a result. TCP connect, proxy CONNECT, and TLS/WebSocket handshakes retain
their separate existing timeouts. OpenAI name resolution completes before the
microphone is opened.

## Current UI-facing contracts

| Semantic concept | Tauri event | Meaning |
|---|---|---|
| `runtime.status` | `runtime-status` | `idle`, `starting`, `running`, `reconnecting`, `stopping`, `stopped`, or `error` |
| `runtime.control.changed` | `runtime-control-changed` | the newest revisioned, redacted `RuntimeControlSnapshot` after an authoritative control change |
| `caption.aggregate.changed` | `caption-aggregate-changed` | the newest full `CaptionAggregateSnapshot` |
| `audio.level` | `audio-level` | generation/revision-scoped 100 ms RMS/peak/gate/clipping scalars; never PCM |
| `diagnostic` | `diagnostic-event` | categorized report with a stable code ([ADR 0014](./adr/0014-diagnostic-codes-are-category-detail.md)) |

Caption-unit lifecycle is internal to the Recognition Module. The Caption
Aggregate exposes only application-owned `openSourceUnits` and normalized
snapshots; there are no separate Tauri utterance events. The UI derives
listening state from `openSourceUnits`, while recognition failures remain
visible through diagnostics.

The Diagnostics page can copy a versioned, redacted JSON report containing
only app metadata, a normalized platform family, runtime status and timestamp,
and each bounded diagnostic's category, severity, stable code, and timestamp.
The report is built through an explicit field allowlist: caption text,
diagnostic messages and details, configuration, device identifiers, network
targets, paths, and credential status are not serialized. Clipboard
access is write-only, and the App does not create or retain a report file.

Rust owns one versioned Caption Aggregate, `CaptionAggregateSnapshot`: a
monotonic aggregate revision, the active application-assigned stream, open
Source units, and bounded recent full-text snapshots with lane, per-scope
revision, and ongoing/completed state
([ADR 0010](./adr/0010-adapters-emit-full-snapshots-not-deltas.md)).
Translation snapshots additionally identify the exact completed Source
snapshot they consumed. Lane completion is terminal for that lane's revision
chain, not for the entire correlated unit
([ADR 0027](./adr/0027-link-translations-to-exact-source-snapshots.md)). The
aggregate may retain completed captions from older runtime generations, so it
is deliberately not called a session.

Unit-based captions are newest-unit-first by application-accepted unit sequence;
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

A shared IPC manifest pins the five UI-facing Tauri event names and all invoke
command names in Rust and TypeScript. A separate wire-vocabulary manifest pins
every closed cross-language enum value and tagged-union discriminator; the
scenario fixtures continue to pin complete payload shapes.

Caption text never contains presentation placeholders; the UI derives
listening, translating, degraded, and failure states from lifecycle and
health events.

The frontend reaches caption-runtime, settings, audio, and OSC host
capabilities through one typed `AppGateway`. Production Tauri IPC and the
deterministic Preview are concrete adapters behind that boundary; frontend
domain state does not import Tauri commands, wire decoders, or Preview behavior
directly. Narrow UI-only services such as confirmation and diagnostic-report
clipboard access use feature-specific host ports instead of inflating the
runtime store. Host command failures are normalized into structured
application failures that retain a stable code and fallback message.

## Runtime control snapshot

Saved settings and the active runtime generation are separate state
([ADR 0012](./adr/0012-saved-settings-are-not-the-running-session.md)). Rust
owns one revisioned, redacted control snapshot: the desired configuration,
the application-resolved Caption Pipeline Plan, service-credential status,
lifecycle status, and the immutable selection captured by the current
generation. `RuntimeControlSnapshot` names that generation explicitly and
records its phase, selection, credential, Chatbox-publication state, and
pending generation changes. Control mutations return the resulting snapshot;
the frontend never reconstructs active state by shallow-merging saved config.
The Stop-versus-Start ordering guarantees live in the runtime code and tests.
The first supported App Config, Runtime Control, and Caption Aggregate formats
are independent V1 contracts; current implementation types do not carry those
wire versions in their names
([ADR 0028](./adr/0028-establish-the-supported-contract-baseline-at-v1.md)).

## Runtime lifecycle

Start validates configuration, credentials, audio devices, and the requested
plan before capture begins. An incompatible combination is explained with
explicit alternatives, never adjusted silently. A safe per-unit recognition,
translation, or OSC failure emits a diagnostic and may leave the generation
running. A classified transient recognition failure first closes audio
admission. The coordinator then drops capture, ends unconfirmed units, and
acknowledges the retired attempt; only then may the Recognition Module enter
visible `reconnecting` backoff and open a fresh attempt in the same generation.
The fresh attempt starts with no audio replay. A terminal generation-level failure
moves the runtime to an explicit error state
([ADR 0025](./adr/0025-reconnect-within-one-runtime-generation.md)).

While Running, the application derives fixed-window RMS/peak, gate, and clipping
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

Recognition/translation paths, publication mode, and content selection are independent
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

Runtime submits one application-internal `CaptionAggregateUpdate` for every
store-accepted change. It pairs the newest full aggregate with the exact
accepted Source-unit or caption change; it is not a second wire contract or a
replay journal. The Live worker consumes the full aggregate, while the
Completed worker consumes the exact change. This keeps Runtime independent of
publication timing without asking a bounded five-unit UI history to reconstruct
events that may already have been trimmed.

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

Admission of bounded translation work reserves its exact completed Source
snapshot in the Caption Aggregate. Completion, failure, timeout, cancellation,
and Stop all release that reservation; ordinary bounded-history trimming may
remove only unreserved units. The reservation seam lands with the first real
Translation Module so ownership follows a concrete scheduler rather than a
speculative API ([ADR 0027](./adr/0027-link-translations-to-exact-source-snapshots.md)).

## Target local inference boundary

Local inference is a Rust application and Rust worker using packaged native
libraries, with no Python, PyTorch, or Conda
([ADR 0020](./adr/0020-keep-local-inference-out-of-process.md)). One selected
local recognition path and one effective backend are loaded per recognition
attempt; the backend-preference and effective-backend rules are
[ADR 0021](./adr/0021-users-choose-the-local-backend.md). A worker crash
ends the runtime generation and waits for an explicit user decision. The local
driver runs behind the same active Recognition Module boundary as OpenAI: Runtime
still submits continuous owned mono frames and consumes normalized signals.
The driver owns its audio-duration/IPC budgets, resampling or model windows,
model loading, worker messages, native-runtime types, and model-specific
streaming state. A worker crash closes admission and ends the generation; it
does not silently restart on CPU or switch to cloud. Candidate models and
backend facts are in
[research/local-inference-notes.md](./research/local-inference-notes.md).

## Incoming caption boundary

System or VRChat audio can later enter as a separate incoming pipeline with
its own caption lanes and publication policy. Nothing in the runtime assumes
microphone-only input forever, but no incoming capture, diarization, or
overlapping-speaker handling is implemented (roadmap Later).
