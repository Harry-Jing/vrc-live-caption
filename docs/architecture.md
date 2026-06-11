# Architecture

## Scope

This document describes runtime boundaries and data flow. It intentionally does
not define Rust module names, Vue store names, Tauri command names, database
schema, or provider-specific protocols.

## High-Level Flow

```text
Audio Sources
  -> Runtime Pipeline
  -> STT Providers
  -> Normalized Transcript Events
  -> Optional Translation
  -> Output Sinks
```

MVP implements the outgoing path:

```text
Microphone
  -> capture / framing
  -> cloud STT
  -> transcript.partial / transcript.final
  -> App preview
  -> Chatbox renderer
  -> OSC rate limiter
  -> VRChat Chatbox
```

## Core Boundaries

- The frontend does not process raw audio.
- The frontend consumes normalized runtime events and sends user commands.
- Provider raw events do not leak into UI-facing runtime consumers.
- Chatbox is an output sink, not the center of the runtime.
- Translation is optional and must not block capture or STT.
- Local inference, when added, runs out of process behind a sidecar or worker
  boundary.
- Runtime failures should be reported in user-readable diagnostic categories.

## Event Semantics

The runtime event model supports:

- `runtime.status`: runtime lifecycle state (`idle`, `starting`, `running`,
  `stopping`, `stopped`, `error`) with an optional human-readable message.
- `utterance.started`: start of a confirmed utterance, before any transcript
  text exists.
- `transcript.partial`: temporary recognition text that may change.
- `transcript.stable`: text that is likely to remain but is not final.
- `transcript.final`: finalized recognition text.
- `utterance.ended`: end of an utterance that will not produce a final
  transcript, with a reason such as no recognized speech, a failed STT
  request, or a discarded segment.
- `diagnostic`: a categorized report carrying a stable machine-readable code
  and English fallback text.
- `translation.draft`: optional temporary translation text.
- `translation.final`: finalized translation text.

These are semantic names. The concrete Tauri event names and payload shapes
live in code (the Rust events module and the frontend event map) and are
pinned by wire-format tests.

MVP providers may emit only `partial` and `final`. `stable` is part of the
architecture so later two-pass, incoming caption, and interpretation work does
not require a new event model.

Transcript events carry recognition text only. Placeholder text such as a
listening indicator is presentation state that the UI derives from utterance
lifecycle events, never transcript text.

Delivery is best-effort and at-most-once. The UI must tolerate missed events
and derive state from the newest status and lifecycle events; the runtime
lifecycle never depends on whether an event was delivered.

Diagnostic codes are shaped `<category>.<detail>`, where the prefix equals the
event's serialized category. Codes are the stable contract for filtering,
tests, and UI localization; `message` and `detail` are English fallback text
(see [decisions.md](./decisions.md)).

## Runtime Lifecycle

Start validates config and credentials before capture begins. Startup failures
such as invalid config or an unavailable microphone are fatal; per-segment STT
or OSC failures emit diagnostics and keep the runtime alive.

Stop is a hard cutoff: the microphone is released promptly, buffered and
queued speech is discarded, and nothing is sent to the Chatbox after the stop
request; only an STT request already in flight is awaited. The app also stops
the runtime explicitly on exit.

## Output Strategy

App preview can consume partial and final text.

VRChat Chatbox consumes final text in the MVP. It is not a real-time subtitle
terminal and should not receive high-frequency partial updates.

Chatbox output must handle:

- pacing
- length limits
- line and wrap constraints
- final text replacement or history behavior
- OSC send failures

Detailed VRChat layout facts live in
[research/vrchat-chatbox-reference.md](./research/vrchat-chatbox-reference.md).

## Translation Boundary

Translation starts from final transcript text by default.

Translation should run independently from audio capture and STT. Translation
timeouts or failures should not stop source captions from being displayed.

MVP-B may add target-only or bilingual output, but the architecture should keep
translation as a processing stage between transcript events and output sinks.

## Local Inference Boundary

Local STT is the planned long-term default path; the MVP ships cloud-first
(see [decisions.md](./decisions.md)). Whether local translation follows the
local-first direction is an open question in [product.md](./product.md).

The main app should not require users to install Python, PyTorch, CUDA Toolkit,
or model-specific development dependencies. Local STT and local translation
run behind sidecars or workers so model crashes, GPU failures, and large
runtime dependencies do not destabilize the main app.

The main app should be able to fall back to a cloud path or show a clear
diagnostic error when local inference is unavailable.

## Incoming Caption Boundary

The architecture should not assume microphone-only input forever. It should be
possible to add system or VRChat audio capture later as a separate incoming
pipeline.

MVP does not implement incoming capture, speaker diarization, or overlapping
speaker handling.

## Lessons From The Python Prototype

The Python prototype remains useful for behavior and test lessons:

- normalize provider events before runtime consumers see them
- keep Chatbox pacing and text shaping separate from STT providers
- keep translation independent from audio capture and STT lifecycle
- use fake providers and opt-in live tests
- make diagnostics explicit for audio, config, STT, translation, and OSC

The Python package structure, asyncio lifecycle details, CLI contracts, sidecar
protocols, and provider implementation details are not new architecture
constraints.
