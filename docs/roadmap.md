# Roadmap

This roadmap is the implementation order from the current codebase. Each phase
ends in a usable, testable result. This file is the only record of
implementation status; the ADRs in [adr/](./adr/) record why decisions were
made, and [architecture.md](./architecture.md) records the runtime seams.

The strategy: prove the whole experience on cloud first — recognition, Live,
then translation — because cloud paths are the fastest way to complete the
product shape. Then replicate the validated experience locally, because local
is the long-term default ([ADR 0004](./adr/0004-local-stt-is-the-long-term-default.md)).
Release comes last, once the outgoing chain is basically complete; incoming
captions never gate the release
([ADR 0002](./adr/0002-build-outgoing-captions-first.md)).

## Phase 0: Project foundation

Status: complete.

Tauri 2 / Vue 3 / Rust skeleton, config shape, logging and error model,
runtime event path, minimal App shell, OSC test path, and a short diagnostics
surface. Keep this foundation stable; do not reopen it as a general rewrite.

## Phase 1: Completed cloud baseline

Status: complete and superseded as the active provider implementation,
including its original real Windows/VRChat validation.

The segmented `gpt-4o-mini-transcribe` Completed path is the trustworthy
baseline: process-wide 1000 ms pacing
([ADR 0015](./adr/0015-pace-chatbox-sends-at-one-second.md)), the full Chatbox
layout engine
([research](./research/vrchat-chatbox-reference.md)), ordered bounded
pagination, the typing-indicator lifecycle
([ADR 0016](./adr/0016-signal-speech-activity-with-the-typing-indicator.md)),
and hard Stop ([ADR 0011](./adr/0011-stop-is-a-hard-cutoff.md)).

The provisional publisher limits (32 resident pages, 30-second unstarted-unit
age) still need adjustment from real backlog measurements; that lands in the
release phase.

## Phase 2: Frontend and contract tests

Status: complete.

Vitest runs in the normal quality gate. Framework-free lifecycle and
caption-session reducers, one behavior suite shared by the Preview and Tauri
backends, and deterministic outcomes for reload, duplicate, out-of-order, and
late-after-Stop delivery.

## Phase 3: Normalized recognition sessions and a Live-capable publisher

Status: generic foundation complete. The first real Live provider is Phase 4.

Backend-owned `CaptionSessionSnapshotV1` state, the capability planner
([ADR 0006](./adr/0006-publication-timing-is-completed-or-live.md),
[ADR 0010](./adr/0010-adapters-emit-full-snapshots-not-deltas.md)), a
versioned config/control contract, a latest-wins Live worker sharing pacing and
Stop with the Completed path, and Settings that keeps both modes visible with
explicit alternatives instead of automatic fallback. The current config and
runtime-control contract are V3; deterministic recognition scripts are now
test-only rather than a product provider.

## Phase 4: OpenAI Realtime recognition cutover

Status: implementation and authenticated adapter smoke complete; Windows/VRChat
validation pending.

Goal: replace the bounded OpenAI implementation with the complete release
recognition surface: `gpt-transcribe` for Completed, and
`gpt-live-transcribe` for Completed or Live, both over Realtime transcription
WebSockets ([ADR 0024](./adr/0024-use-openai-realtime-transcription.md)).

Implemented:

- the backend-owned catalog contains exactly the two release paths and their
  publication capabilities; legacy models and incompatible timing are rejected
  without migration or fallback;
- the runtime uses one provider-independent `RecognitionSession` Interface;
  scripted deterministic Adapters are test-only;
- the OpenAI Module owns authenticated Realtime WebSocket setup, system-proxy
  routing, 24 kHz PCM conversion, append/commit, `item_id` correlation,
  full-snapshot normalization, detected-language handling, and explicit Stop;
- ordering, item failure, terminal session errors, bounded audio/protocol
  queues, a 30-second post-commit completion deadline, visible per-item
  diagnostics, authentication/status diagnostics, and late events after Stop
  are deterministic under tests;
- provider-authored error text is discarded at the Adapter boundary; stable
  structured classes distinguish terminal failures from transient network,
  rate-limit, and provider-availability failures;
- transient recognition failures reconnect inside the same runtime generation
  with capped jittered backoff, a fresh provider session, no ambiguous audio
  replay, explicit unconfirmed-unit termination, and Stop-cancellable waits
  ([ADR 0025](./adr/0025-reconnect-within-one-runtime-generation.md));
- VAD analysis uses fixed 10 ms frames and sample-count boundaries independent
  of capture callback partitioning; the UI receives only 100 ms scalar audio
  levels and provides a local-only, runtime-exclusive microphone probe;
- invalid legacy settings block Start until the current strict configuration is
  reviewed and saved, while malformed or unsupported selected environment,
  Windows, or macOS proxy settings fail closed rather than allowing a direct
  connection;
- macOS target routing preserves `ExceptionsList` and
  `ExcludeSimpleHostnames` through CFNetwork instead of translating Apple
  rules into `NO_PROXY` semantics;
- an authenticated production-adapter smoke covers both exact models with
  Chinese, English, and mixed-language audio plus a provider-error probe; it
  exposed and verified fixes for the transcription-intent route and rustls
  crypto-provider selection; and
- the production Mock selection, bounded HTTP/WAV Adapter, old model fields,
  and otherwise-unused direct HTTP/WAV dependencies are removed with no
  fallback branch.

Remaining before exit:

- validate the native system-proxy route and cancellation/Stop behavior on the
  Windows Tier 1 path; and
- run short, long uninterrupted, English, Chinese, mixed-language, network
  interruption, and Stop scenarios with VRChat active; record latency,
  revision/correction, and resource measurements.

Exit: only the two catalog paths can start an OpenAI recognition session; the
capability matrix is enforced; no REST/WAV, legacy-model, production-Mock, or
silent fallback path remains; both models pass contract tests and recorded
Windows/VRChat protocol validation.

## Phase 5: Completed translation

Status: not started.

Goal: the smallest reliable text-driven translation path — the original
reason this project exists.

- select and implement one concrete translator for completed source units; no
  local model download required;
- link every target result to its source generation, unit, and revision;
- add source-only, translation-only, and bilingual content settings; bilingual
  Completed output follows
  [ADR 0007](./adr/0007-bilingual-output-is-one-asynchronous-view.md);
- bound translation work with timeout, cancellation, and explicit
  pending/degraded/failed/recovered diagnostics; failure never relabels a
  stale target as the translation of newer source text;
- add the custom OpenAI-compatible base URL setting (relay API,
  [ADR 0019](./adr/0019-follow-system-proxy-plan-relay-api.md)).

Exit: translation never blocks capture, recognition, or Chatbox pacing; late
or retried targets cannot overwrite the wrong revision; bilingual pages obey
the real layout model; failure is visible without changing the user's content
choice.

## Phase 6: Live translation evaluation

Status: not started; conditional on measured results.

Goal: decide which translation update shapes are honest to present as Live —
the [ADR 0006](./adr/0006-publication-timing-is-completed-or-live.md) honesty
rule applied to translation.

- benchmark provider-native simultaneous revisions, token streaming after a
  completed source, and one-shot translation separately;
- run a protocol spike for `/v1/realtime/translations`; promote direct-audio
  translation only if its continuous, ongoing-only semantics fit the product,
  and extend the cloud-audio disclosure
  ([ADR 0009](./adr/0009-cloud-audio-disclosure-lives-in-settings.md)) first;
- never simulate Live by repeatedly translating unstable source partials;
- test lag, failure, recovery, Stop, and stale-target suppression with both
  local-sender and remote-observer users.

Exit: at least one target lane ships with honestly described timing, or a
recorded no-go defers Live translation without blocking the release.

## Phase 7: Localization, diagnostics, and headset UX

Status: groundwork started — a typed English frontend catalog exists; the
locale switch and diagnostic mapping do not.

Goal: make the app operable by English- and Chinese-speaking players.

- add a locale setting and a complete `zh-CN` catalog; render diagnostics
  from stable codes
  ([ADR 0014](./adr/0014-diagnostic-codes-are-category-detail.md),
  [ADR 0008](./adr/0008-localize-the-ui-in-the-frontend.md));
- add a copyable, redacted diagnostic report;
- compare global hotkeys, auto-start with VRChat, and a later overlay, then
  implement the smallest headset-friendly start/stop/error surface.

Exit: every first-release surface switches between English and Chinese; a
headset user can start, stop, and notice a failure without technical error
text reaching the public Chatbox.

## Phase 8: Local STT on CPU

Status: not started; model and runtime research may continue earlier.

Goal: one reliable single-model local Completed path with no Python and no
silent cloud fallback
([ADR 0020](./adr/0020-keep-local-inference-out-of-process.md),
[ADR 0004](./adr/0004-local-stt-is-the-long-term-default.md)).

- decide the distribution shape before building: installer-bundled, first-run
  download, or a managed component catalog;
- define a narrow Rust worker protocol with bounded queues, health checks,
  and crash isolation;
- implement sherpa-onnx plus SenseVoiceSmall on CPU as the first bounded local
  worker Adapter behind the same `RecognitionSession` Interface used by
  OpenAI ([research](./research/local-inference-notes.md));
- record an English/Chinese/mixed-speech, latency, and resource baseline with
  VRChat running;
- add distinct diagnostics for missing files, incompatible runtime, load
  failure, backlog, and worker crash.

Exit: a sustained local Windows/VRChat session works, a worker crash cannot
destabilize the app, and the local component can be installed, verified,
repaired, and removed per the recorded distribution decision.

## Phase 9: NVIDIA CUDA and local Live

Status: not started.

Goal: complete the local backend choice and add real local Live candidates
([ADR 0021](./adr/0021-users-choose-the-local-backend.md)).

- package and validate the CUDA runtime path; preserve the preference when a
  model lacks CUDA support;
- implement Streaming Paraformer and Streaming Zipformer as independent local
  Live Adapters behind the same `RecognitionSession` Interface;
- bring every model/runtime pack through the Phase 8 distribution,
  verification, and removal flow;
- benchmark accuracy, mixed-language speech, latency, resources, and VRChat
  frame time per combination; publish recommendations only from recorded
  data;
- switch the long-term default to local only after a candidate meets the
  recorded thresholds ([ADR 0004](./adr/0004-local-stt-is-the-long-term-default.md)).

Exit: at least one local Completed path and one local Live path pass real
Windows/VRChat testing, and the local-default decision is recorded with
benchmark evidence.

## Phase 10: Windows public release

Status: not started. Deliberately last: release waits until the outgoing
chain is basically complete.

Goal: ship an installable, supportable Windows Tier 1 release containing only
the paths that passed their gates
([ADR 0003](./adr/0003-windows-is-tier-1.md)).

- configure code signing and updater key handling without committing private
  material;
- finish versioning and the release-note flow;
- validate system-proxy behavior and the relay API path with real
  Chinese-network users
  ([ADR 0019](./adr/0019-follow-system-proxy-plan-relay-api.md));
- audit the Tauri permission allowlist against the APIs actually used;
- adjust the provisional Phase 1 publisher limits from measured backlog and
  readability;
- run long-session Windows tests for every enabled path with VRChat active;
  test install, update-failure recovery, and uninstall;
- produce final workflow artifacts for Windows, macOS arm64, and the Linux
  x86_64 AppImage.

Exit: the release checklist passes on clean machines, and every advertised
path has a Windows/VRChat validation record.

## Later

Promote these only when a concrete need justifies the cost:

- local translation worker and model packs;
- incoming captions from system or VRChat audio
  ([ADR 0002](./adr/0002-build-outgoing-captions-first.md));
- persistent history and export
  ([ADR 0023](./adr/0023-keep-session-history-in-memory-only.md));
- two-pass recognition
  ([ADR 0006](./adr/0006-publication-timing-is-completed-or-live.md));
- DirectML or other non-CUDA Windows GPU paths;
- interpretation, TTS, virtual microphone output, speaker diarization, and
  plugins.
