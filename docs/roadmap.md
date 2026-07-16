# Roadmap

This roadmap is the implementation order from the current codebase, not a list
of every possible provider or model. Each phase ends in a usable, testable
result. Research may begin early, but implementation should not skip the
contracts and tests on which a later phase depends.

The status and exit criteria are deliberate: they distinguish what is already
in the repository from accepted target behavior that still has to be built.

## Phase 0: Project Foundation

Status: complete.

Goal: establish the smallest Tauri/Vue/Rust base that can support the runtime.

- Tauri 2, Vue 3, TypeScript, and Rust skeleton
- basic config shape
- basic logging and error model
- minimal runtime event path
- minimal App shell
- OSC test path
- short diagnostics surface

Exit criteria: keep this foundation stable; do not reopen it as a general
rewrite while implementing later phases.

## Phase 1: Completed Cloud Baseline Hardening

Status: core microphone-to-OpenAI-to-App-to-Chatbox path implemented;
reliability corrections and real-client validation remain.

Goal: make the existing segmented `gpt-4o-mini-transcribe` Completed path a
trustworthy baseline before changing the provider and wire contracts.

- enforce one process-wide, fixed `1000 ms` interval from the previous actual
  text-send attempt, shared by Runtime output and OSC Test; failed attempts also
  consume the opportunity, and restarting Runtime does not reset the interval;
- remove the legacy `osc.minIntervalMs` setting from the config contract and
  settings UI; older config files may retain that ignored key while every other
  setting continues to load unchanged;
- implement the full Chatbox layout engine from
  [research/vrchat-chatbox-reference.md](./research/vrchat-chatbox-reference.md):
  144 UTF-16 code units, at most nine visible lines, measured glyph widths,
  explicit newlines, Unicode line-breaking, and grapheme-safe boundaries;
- paginate current Completed text in order through a bounded, non-blocking
  publisher; under sustained overload, drop only oldest whole units that have
  not started publication and report the loss;
- add fake-time and fake-OSC Rust tests for Completed pacing, ordered pages,
  overload, Unicode boundaries, typing, send failures, and Stop races;
- preserve the typing-indicator lifecycle across success, no-speech, per-unit
  failure, and Stop;
- make Stop release capture, discard buffered/queued audio and unsent publisher
  pages, prevent new provider submissions, cancel or close in-flight work where
  possible, and reject every late caption for both App and Chatbox; allow only
  one typing-off cleanup message;
- measure real Chatbox display duration and remote-observer readability before
  choosing any additional page hold-time or adjacent-unit merge rule;
- validate the full path on real Windows 10/11 VRChat setups;
- keep the quality workflow green on `ubuntu-latest`, `windows-latest`, and
  `macos-latest` for every branch push;
- run the native bundle workflow at least once for Windows, macOS arm64, and
  the Linux x86_64 AppImage. Its current triggers are qualifying pushes to
  `main`, pull requests, and manual dispatch from the default branch.

Exit criteria:

- normal Completed speech reaches App and VRChat without truncating later
  pages, exceeding layout limits, or skipping messages at the sustained send
  cadence;
- audio capture and cloud transcription do not wait on OSC pacing;
- Stop produces no late App or Chatbox text from the stopped generation;
- common audio, cloud, config, and OSC failures remain visible and diagnosable;
- the documented checks and one native-bundle matrix run are recorded as
  passing.

## Phase 2: Frontend And Contract Test Foundation

Status: partially started. Rust tests and the pull-side runtime status snapshot
exist; Vitest and shared frontend behavior tests do not.

Goal: build the regression net before replacing the current transcript wire
contract.

- add Vitest to the frontend quality gates;
- extract caption/lifecycle state into framework-free modules;
- run one behavior suite against both the preview backend and the Tauri gateway
  so they cannot drift;
- test webview reload resynchronization through the existing runtime status
  snapshot;
- test duplicate, out-of-order, and late-after-Stop current-wire events;
- test settings round trips and the Phase 1 legacy-config compatibility path.

Exit criteria:

- the preview and Tauri gateways pass the same lifecycle behavior suite;
- reload, event reordering, duplicate delivery, and late-after-Stop delivery have
  deterministic tested outcomes;
- frontend tests run through the normal project quality gate.

## Phase 3: Normalized Recognition Sessions And Live-Capable Publisher

Status: not started.

Goal: establish the deep runtime seams shared by bounded cloud, Realtime cloud,
and later local providers, without adding translation or a second recognizer.

- version the UI-facing caption contract with generation, session/stream
  correlation, optional caption-unit identity, source/translation lane,
  monotonic revision, full text, and ongoing/completed state;
- preserve separate runtime lifecycle and diagnostic events, including
  `runtime.status`, utterance start, and utterance end without a completed
  transcript;
- remove the unused `stable` reservation instead of assigning it new meaning;
- expose one recognition-session seam: provider-independent audio in and
  normalized full snapshots out;
- move the existing bounded OpenAI transcription path behind its own concrete
  adapter;
- describe capability for the complete provider path, including input shape,
  boundary owner, per-lane update/completion behavior, and revision behavior;
- extend the Phase 1 publisher with Completed and Live policies rather than
  letting providers publish directly;
- implement the per-unit one-second Live observation window and, for a
  unitless ongoing-only stream, a one-second first-non-empty stream-start delay
  that never fabricates completion;
- implement the latest-wins rolling viewport, final correction where completion
  exists, and no queue of obsolete ongoing revisions;
- for recognition paths that own real caption units, implement natural-
  boundary-first segmentation plus a hard maximum for long uninterrupted
  speech; choose timings from recorded tests rather than an ordinary user
  slider;
- for a unitless continuous path, bound buffers, backpressure, reconnect, and
  session lifetime without turning a timer or silence into a completed unit;
- add persisted Completed/Live selection and UI controls that explain each
  timing choice and the selected path's actual update behavior;
- when a selection is incompatible, offer two explicit directions: keep the
  model/provider and choose a supported publication mode, or keep the desired
  experience and choose a compatible model/provider;
- add fake bounded, ongoing-plus-completed, and ongoing-only adapters;
- extend the Phase 1 fake-time/OSC harness to pin the observation window, Live
  coalescing, correction, and interactions with inherited pacing and Stop
  behavior;
- test stale generations, capability resolution, persisted mode migration, and
  both explicit incompatibility choices.

Exit criteria:

- the existing bounded OpenAI path still produces Completed output;
- fake ongoing-plus-completed input supports both modes, while fake
  ongoing-only input supports Live without fabricated completion;
- all OSC candidates are non-blocking and every actual text-send attempt stays
  at least 1000 ms after the previous attempt;
- unsupported combinations remain selected until the user explicitly chooses
  one of the explained alternatives;
- Stop at any provider, snapshot, publisher, or send boundary emits no
  caption text and accepts no late App snapshot.

## Phase 4: First Real Live Recognition Path

Status: not started and gated by protocol evidence; first-party documentation
research is recorded in
[research/openai-speech-streaming-options.md](./research/openai-speech-streaming-options.md).

Goal: determine whether a real cloud provider can deliver the accepted Live
experience rather than assuming the fake-provider path proves it.

- run an authenticated protocol spike for OpenAI Realtime transcription;
- resolve the documented model/session compatibility conflict for
  `gpt-4o-transcribe` and `gpt-4o-mini-transcribe`, using
  `gpt-realtime-whisper` as the currently documented dedicated Live candidate;
- capture timestamped raw event traces for VAD and manual-commit behavior,
  revisions, item completion, final correction, usage, and disconnects;
- implement one real Realtime recognition adapter only after its observed
  protocol has been recorded and mapped to the normalized contract;
- keep provider/session failure explicit; never silently switch to the bounded
  provider, another model, or another publication mode;
- validate short speech, long uninterrupted speech, code-switching, network
  interruption, and Stop on Windows with VRChat running;
- record test duration and hardware plus first-useful-text latency,
  speech-end-to-completion p50/p95, revision count, correction size, disconnect
  outcome, CPU/RAM/network use, and observed VRChat frame-time impact.

Exit criteria:

- a short unit that completes in the first second publishes only its completed
  text, while a long unit begins useful rolling output and keeps the newest
  content visible;
- item completion and corrections are correlated without duplicate Chatbox
  publication;
- reconnect or session failure cannot turn an incomplete stream into a false
  completion or change the user's selected path;
- either the real Live path passes thresholds chosen and recorded from the
  measured Windows/VRChat session, or a no-go record defers cloud Live, leaves
  it unavailable for current providers, and prevents the first release from
  advertising it; the public release itself may continue with Completed.

## Phase 5: Completed Translation MVP-B

Status: not started.

Goal: ship the smallest reliable text-driven translation path before adding
simultaneous or token-streaming translation behavior.

- select and implement one concrete translator for completed normalized source
  units; the first path must not require a local model download;
- link every target result to its source generation, unit, and revision;
- add independent source-only, translation-only, and bilingual content
  settings;
- deliver translation-only Completed output first, then source-above-target
  bilingual Completed output;
- share the 144-character and nine-line budget dynamically, keep both available
  lanes visible, and give remaining capacity a modest default preference toward
  translation;
- paginate bilingual Completed content through the ordered bounded publisher;
- add bounded translation work, timeout, cancellation, retry, and explicit
  pending/degraded/failed/recovered diagnostics;
- on failure, keep the configured content choice unchanged, keep source text in
  the App, and never label an old target as the translation of newer source;
- expose translation provider, target language, and relevant timeout/retry
  settings without coupling them to STT language or UI locale.

Exit criteria:

- translation never blocks microphone capture, recognition, or Chatbox pacing;
- late or retried target results cannot overwrite another source revision or
  caption unit;
- Completed target-only and bilingual pages obey the real Chatbox layout model;
- translation failure is visible, does not silently change content mode, and
  leaves every still-valid source caption usable.

## Phase 6: Live Translation Evaluation And Expansion

Status: not started and conditional. This phase is a measured product decision,
not a promise that every translator will be presented as Live.

Goal: determine which real translation update shapes are useful and honest in
VRChat, then implement only the paths that pass that test.

- benchmark provider-native simultaneous target revisions, token streaming
  after a completed source, and one-shot completed translation separately;
- treat the mapping from translation update shape to the public Live experience
  as provisional until those tests are complete;
- test the asynchronous bilingual viewport where source can lead target and
  every send recomputes one newest useful combined view;
- never repeatedly submit every unstable ASR revision to an ordinary text
  translator merely to simulate Live;
- run a separate protocol spike for `/v1/realtime/translations`; direct-audio
  translation is promoted only if its continuous, ongoing-only semantics and
  quality are suitable for the product;
- before any direct-audio path is promoted, extend the persistent cloud-audio
  disclosure so users know microphone audio is uploaded for translation;
- test lag, explicit failure, recovery, Stop, and stale-target suppression with
  both local-sender and remote-observer users.

Exit criteria:

- either at least one concrete target lane ships with clearly described Live
  timing and deterministic failure behavior, or a recorded no-go decision
  defers Live translation without blocking the first public release;
- a complete-result-only translator is never presented as updating during
  speech;
- normal lag does not move the whole Live viewport backward, and explicit
  failure never places stale target text under newer source.

## Phase 7: Localization, Diagnostics, And Headset UX

Status: groundwork started. A typed English frontend catalog exists; Chinese
locale selection and complete diagnostic mapping do not.

Goal: make the enabled paths understandable and operable by English- and
Chinese-speaking VRChat users.

- add a locale setting and a complete `zh-CN` catalog;
- render user-facing diagnostics from stable codes instead of backend English
  prose;
- add a copyable, redacted diagnostic report with provider/model/backend,
  configured preference, effective choice, and visible fallback reason;
- keep caption language, UI locale, translation target, publication mode,
  content selection, model, and compute preference independent;
- test headset-friendly start, stop, and error visibility; compare global
  hotkeys, auto-start with VRChat, and a later overlay before selecting the
  smallest first-release control surface;
- implement and verify the selected first-release headset control surface;
- resolve or explicitly defer the remaining launch-relevant questions in
  [product.md](./product.md), including manual approval, end-to-end latency, and
  Chatbox reading-time policy.

Exit criteria:

- every first-release surface can switch between English and Chinese;
- users can distinguish configured choices from effective runtime choices and
  copy useful diagnostics without leaking secrets;
- a headset user can start, stop, and notice a failure without technical error
  text being published to the public Chatbox.

## Phase 8: Windows Public Release Readiness

Status: not started.

Goal: ship an installable and supportable Windows Tier 1 release containing
only the provider paths that passed their earlier gates.

- configure code signing and updater key handling without committing private
  material;
- finish versioning and release-note flow;
- validate system-proxy behavior, common Clash-style setups, timeouts, and the
  network-unreachable diagnostic on real Windows machines;
- audit the current explicit Tauri allowlist and keep only permissions required
  by the APIs the App actually uses;
- run long-session Windows tests for every enabled Completed, Live, and
  translation path with VRChat active;
- test install, update-failure recovery, and uninstall;
- produce final workflow artifacts for Windows, macOS arm64, and Linux x86_64
  AppImage, while keeping Windows as the only platform with claimed complete
  real-machine validation.

Exit criteria:

- the Windows release checklist, installer, updater, and uninstall path pass on
  clean machines;
- every advertised provider path has a corresponding Windows/VRChat validation
  record and clear network/provider failure behavior;
- Tier 2 compilation, automated tests, and package builds are green without
  claiming macOS or Linux real-machine validation.

## Phase 9: Local STT CPU Foundation

Status: not started; model/runtime research may continue earlier.

Goal: deliver one reliable, single-model local Completed path without Python,
PyTorch, Conda, or a silent cloud fallback.

- decide the distribution shape before building it: installer-bundled,
  first-run download, or a managed component catalog; record download size,
  license, update, verification, repair, and removal consequences;
- define a narrow Rust worker protocol, bounded audio/result queues, health
  checks, and crash isolation;
- implement sherpa-onnx plus SenseVoiceSmall on CPU as the first bounded local
  adapter;
- record a first English, Chinese, mixed-language, latency, CPU/RAM, and VRChat
  frame-time baseline for that exact CPU path;
- implement the minimum verified model/runtime install and self-test flow
  required by the chosen distribution decision;
- add distinct diagnostics for missing files, incompatible runtime, load
  failure, backlog, and worker crash;
- keep one STT model resident and unload it before changing models;
- stop the session on worker failure and let the user explicitly retry; never
  upload audio to cloud because the local path failed.

Exit criteria:

- local Completed recognition runs for a sustained Windows/VRChat session and
  a worker crash cannot destabilize the Tauri process;
- all IPC queues stay bounded and an inference backlog is visible rather than
  replayed indefinitely;
- the local component can be installed, verified, repaired or reinstalled, and
  removed according to the recorded distribution decision;
- cloud remains the default until the local program passes the Phase 10
  comparison gate.

## Phase 10: NVIDIA CUDA And Local Live

Status: not started.

Goal: complete the local backend choice, add real local Live candidates, and
decide the long-term default from measured VRChat results.

- package and validate the SenseVoice path with the supported NVIDIA CUDA
  runtime before comparing it with CPU;
- expose one global CPU or prefer-NVIDIA-CUDA setting and show the effective
  backend for the active model/session;
- preserve the preference when a model lacks CUDA; an unsupported combination
  or CUDA startup failure may use CPU only with a clear visible reason;
- never change backend automatically after a running worker crashes; stop and
  offer explicit same-backend retry or a user-selected alternative;
- implement Streaming Paraformer and Streaming Zipformer as independent local
  Live candidates;
- bring every added model/runtime/backend package through the distribution,
  integrity check, self-test, repair/reinstall, and removal flow selected in
  Phase 9, whether that flow is bundled or downloaded;
- integrate installed local models into the model picker with their supported
  publication modes, languages, sizes, licenses, and effective backends;
- benchmark accuracy, mixed Chinese/English speech, first useful text, endpoint
  latency, CPU/RAM/GPU/VRAM, VRChat frame time, thermals, and long-session
  stability for every supported model/runtime/backend combination;
- publish recommendations only from the recorded data and switch the long-term
  default speech path to local only after a candidate meets the accepted
  quality, resource, and reliability thresholds.

Exit criteria:

- at least one local Completed path and one local Live path pass real
  Windows/VRChat testing;
- at least one CUDA combination passes its declared support tests, or a recorded
  no-go decision disables CUDA selection and removes CUDA support claims;
- CPU/CUDA and model recommendations state the tested hardware and tradeoffs
  instead of claiming one universal best choice;
- switching model or backend never leaves two STT models resident or changes a
  running session silently;
- no model appears selectable until its files and runtime have passed the
  chosen install, verification, and self-test flow, and every optional pack can
  be repaired/reinstalled and removed;
- the local-default decision is recorded with benchmark evidence.

Candidate details are in
[research/local-inference-notes.md](./research/local-inference-notes.md).

## Later

Promote these only after their prerequisites are mature and a concrete user
need justifies the cost:

- local translation worker and model packs;
- optional translation-alignment-priority display if observer testing justifies
  it;
- automatic CPU/GPU recommendation only if a reliable, explainable selector is
  demonstrated;
- optional two-pass recognition after single-pass speech, translation, model
  management, and benchmarks are mature;
- investigate DirectML or other non-CUDA Windows GPU paths without assuming a
  runtime supports them;
- incoming caption from system or VRChat audio;
- persistent history and export;
- interpretation, TTS, virtual microphone, speaker diarization, and plugins.
