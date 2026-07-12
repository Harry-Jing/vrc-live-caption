# Roadmap

The roadmap tracks near-term implementation phases only. Longer-term product
ideas stay in `Later` until the MVP path is stable.

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

## Phase 1: Outgoing MVP-A

Status: core path implemented; remaining work is listed below, and success
criteria are still being validated in real VRChat sessions.

Goal: make the core user path work reliably.

- microphone selection
- microphone capture
- cloud STT path
- App preview for recognized speech
- final-only Chatbox output
- Chatbox pacing and length control
- basic settings
- basic diagnostics for audio, STT, config, and OSC

Success criteria:

- A user can speak into a microphone and see final text in VRChat Chatbox.
- Chatbox output does not attempt partial streaming.
- The App reports common setup failures clearly.

Remaining work:

- full Chatbox wrap model from
  [research/vrchat-chatbox-reference.md](./research/vrchat-chatbox-reference.md);
  the current width model is a simplified approximation
- clear in-App disclosure before microphone audio is uploaded to a cloud
  provider (a MUST in [product.md](./product.md))
- get the Windows quality gate green; the quality workflow now runs on
  ubuntu-latest and windows-latest for every branch push
- execute the Tauri bundle build workflow at least once; it triggers only on
  pushes to main, pull requests, and manual dispatch from the default branch,
  none of which have happened yet
- validation in real Windows VRChat sessions

## Phase 2: Frontend Test Infrastructure

Status: not started; the runtime status snapshot shipped early during Phase 1
reliability work.

Goal: protect MVP-A behavior before building on it. The Rust runtime already
has unit tests; the frontend has none.

- Vitest wired into the quality gates
- caption state machine extracted into framework-free modules and unit tested
- one behavior suite run against both the preview backend and the Tauri
  backend gateway so the two cannot drift
- runtime status snapshot command so a reloaded webview can resync state, as
  the pull-side companion to best-effort event delivery (implemented early
  during Phase 1 reliability work; see [decisions.md](./decisions.md))

## Phase 3: Outgoing MVP-B

Status: not started.

Goal: add translation without destabilizing the MVP-A path.

- final-only translation stage
- target-language Chatbox output
- optional bilingual rendering if it stays simple
- translation timeout and fallback behavior
- translation settings
- diagnostic state for translation failures

Success criteria:

- Translation does not block microphone capture or STT.
- Source text remains visible when translation fails.
- Chatbox output remains paced and length-limited.

## Phase 4: UI Localization

Status: not started.

Goal: ship the UI in English and Chinese (see
[decisions.md](./decisions.md)).

- locale setting and a frontend string catalog
- diagnostics rendered from stable codes instead of backend text
- caption language, UI locale, and translation target stay independent

## Phase 5: First Public Release

Status: not started.

Goal: ship installable builds users can trust. The first release platform is
Windows (see [decisions.md](./decisions.md)).

- code signing and updater key handling
- versioning and release notes flow
- resolve the cloud reachability open question in [product.md](./product.md);
  a cloud-only release that many target users cannot connect to is not
  shippable
- review and narrow Tauri capabilities and permissions to the APIs the app
  actually uses; `core:default` was only acceptable during the foundation
  phase
- validation on real Windows VRChat setups

## Phase 6: Local STT Path

Status: not started.

Goal: make local STT the default path (see [decisions.md](./decisions.md)).

- engine research: candidate engines (see
  [research/local-inference-notes.md](./research/local-inference-notes.md)),
  accuracy for English and Chinese, streaming versus segmented input, and
  resource usage measured on a Windows machine that is also running VRChat
- input-side provider contract: where segmentation lives once a provider
  consumes a continuous audio stream
- model distribution: bundled with the installer versus first-run download
- local STT sidecar implementation
- switch the default provider to local once validated

Engine research may start before earlier phases finish; the implementation
must not destabilize the released outgoing path.

## Later

Later capabilities should be promoted only after the outgoing path is stable.

- incoming caption from system or VRChat audio
- local translation sidecar
- model and component management beyond the local STT path
- persistent history and export
- interpretation workflows
- TTS
- virtual microphone output
- speaker diarization
- plugin system
