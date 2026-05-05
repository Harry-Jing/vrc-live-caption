# Roadmap

The roadmap tracks near-term implementation phases only. Longer-term product
ideas stay in `Later` until the MVP path is stable.

## Phase 0: Project Foundation

Goal: establish the smallest Tauri/Vue/Rust base that can support the runtime.

- Tauri 2, Vue 3, TypeScript, and Rust skeleton
- basic config shape
- basic logging and error model
- minimal runtime event path
- minimal App shell
- OSC test path
- short diagnostics surface

## Phase 1: Outgoing MVP-A

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

## Phase 2: Outgoing MVP-B

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

## Later

Later capabilities should be promoted only after the outgoing path is stable.

- incoming caption from system or VRChat audio
- local STT sidecar
- local translation sidecar
- model download and component management
- persistent history and export
- interpretation workflows
- TTS
- virtual microphone output
- speaker diarization
- plugin system
