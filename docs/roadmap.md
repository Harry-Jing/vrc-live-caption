# Roadmap

This is the only implementation-status document. Product intent lives in
[product.md](./product.md), runtime boundaries in
[architecture.md](./architecture.md), and durable decisions in [adr/](./adr/).

Current state: **Phase 4 complete; Phase 5 is next.** The app remains in active
development with no public release.

The sequence proves the outgoing experience on cloud recognition and
translation before reproducing it locally. Local becomes the default only after
real Windows/VRChat validation. Incoming captions do not gate the first release.

## Completed foundation

| Phase | Result |
|---|---|
| 0 — Project foundation | Tauri/Vue/Rust shell, settings, errors, runtime events, OSC test, and initial diagnostics |
| 1 — Completed cloud baseline | bounded cloud recognition, Chatbox pacing/layout/pagination, typing lifecycle, and hard Stop |
| 2 — Frontend and contract tests | deterministic frontend reducers, typed application gateway, Preview adapter, and shared contract coverage |
| 3 — Caption state and Live publisher | normalized Caption Aggregate, capability planning, versioned control contracts, and Completed/Live publisher foundations |
| 4 — OpenAI Realtime recognition | closed two-path OpenAI catalog, Realtime transcription, reconnect, and microphone telemetry/probe |

Phase 4 passed authenticated provider smoke tests for both selected OpenAI paths
and a native Windows/VRChat real-microphone matrix on 2026-08-10, including long
speech and mixed English/Chinese speech. No release-blocking issue was observed.

Compatibility cutoff, versioning rules, and current shared artifacts live in
[contracts/](../contracts/).

## Phase 5: Completed translation

Status: **not started — next**.

Goal: deliver the smallest reliable text-driven translation path, which is the
original cross-language product need.

- select one translator for completed Source snapshots;
- correlate every result to its exact source generation, stream, unit, and
  revision;
- bound admission, timeout, cancellation, retries, and retained source work;
- add source-only, translation-only, and bilingual content selection;
- render bilingual Completed pages against the verified Chatbox layout;
- add a user-configured OpenAI-compatible base URL without weakening credential
  or proxy disclosure.

Exit: translation never blocks capture, recognition, or Chatbox pacing; stale or
late results cannot overwrite another source revision; failure is visible
without changing the user's content choice.

## Phase 6: Live translation evaluation

Status: **not started; conditional on measured results**.

Goal: determine which target-update shapes are honest to present as Live.

- compare provider-native simultaneous output, token streaming after completed
  source, and one-shot translation;
- evaluate direct-audio translation as a separate path shape and disclosure;
- never simulate Live by repeatedly translating every unstable source revision;
- test lag, failure, recovery, Stop, and stale-result suppression with local and
  remote observers.

Exit: at least one translation path has an honestly described timing mode, or a
recorded no-go defers Live translation without blocking release.

## Phase 7: Localization, diagnostics, and headset UX

Status: **groundwork started**. A typed English UI catalog and copyable redacted
diagnostic report exist; locale switching and localized diagnostic presentation
do not.

Goal: make the app operable by English- and Chinese-speaking players and usable
while wearing a headset.

- add a locale setting and complete `zh-CN` catalog;
- render application failures from stable codes;
- evaluate global hotkeys, VRChat auto-start, and a later overlay;
- implement the smallest headset-friendly start/stop/error surface justified by
  those tests.

Exit: every first-release surface switches between English and Chinese, and a
headset user can start, stop, and notice failure without publishing technical
errors to Chatbox.

## Phase 8: Local recognition on CPU

Status: **research only; implementation not started**.

Goal: one packaged, reliable local Completed path with no Python requirement and
no silent cloud fallback.

- choose installer-bundled, on-demand, or managed-component distribution;
- pin and license the selected runtime/model artifacts;
- implement a bounded Rust worker protocol behind the Recognition Module;
- evaluate sherpa-onnx and SenseVoiceSmall as the first CPU path;
- benchmark English, Chinese, mixed speech, latency, resources, and VRChat frame
  time on native Windows;
- diagnose missing, incompatible, corrupt, overloaded, and crashed components.

The evidence and unresolved gates are in the
[local recognition evaluation](./research/local-recognition-evaluation.md).

Exit: a sustained Windows/VRChat generation works; a worker crash cannot
destabilize the app; the component can be installed, verified, repaired, and
removed.

## Phase 9: NVIDIA CUDA and local Live

Status: **not started**.

Goal: validate the explicit local backend choice and at least one true local
Live path.

- package and test the complete CUDA runtime chain on clean Windows machines;
- evaluate Streaming Paraformer and Streaming Zipformer independently;
- bring every supported model/backend through the Phase 8 component lifecycle;
- compare accuracy, latency, resources, VRChat frame time, and stability;
- switch the long-term default only after recorded thresholds are met.

Exit: at least one local Completed and one local Live path pass native
Windows/VRChat testing, with evidence-backed hardware guidance.

## Phase 10: Windows public release

Status: **release work not started; build groundwork exists**. CI already creates
Windows, macOS arm64, and Linux AppImage test artifacts. They are not supported
releases, and Windows remains the user platform.

Goal: ship an installable and supportable Windows release containing only paths
that passed their gates.

- configure signing and updater key handling without repository secrets;
- establish versioning and release notes;
- validate proxy and custom-base-URL behavior with users on Chinese networks;
- audit the final Tauri capability allowlist;
- tune provisional Chatbox backlog limits from real readability measurements;
- run long-duration Windows/VRChat tests for every advertised path;
- test clean install, update failure, recovery, and uninstall.

Exit: the release checklist passes on clean Windows machines, and every
advertised path has a real Windows/VRChat validation record.

## Later

Promote these only when a concrete user need justifies the cost:

- local translation worker and model packs;
- incoming captions from system or VRChat audio;
- persistent caption history and export;
- two-pass recognition;
- DirectML or other non-CUDA Windows GPU paths;
- interpretation, TTS, virtual microphone output, speaker diarization, and
  plugins.
