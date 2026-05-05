# VRC Live Caption Context

VRC Live Caption is a desktop app for local real-time speech understanding,
caption preview, translation, and output routing for VRChat and desktop voice
communication.

The rewrite is not a direct port of the Python prototype. The old prototype is
useful for behavior, testing, and product lessons, but it does not define the
new architecture.

## Current Direction

- App shell: Tauri 2.
- Frontend: Vue 3, TypeScript, and Vite.
- Runtime: Rust.
- MVP product scope: Outgoing Caption.
- MVP-A: microphone input to STT, App preview, and final-only VRChat Chatbox
  output.
- MVP-B: final-only translation after MVP-A is stable.

## Product Principles

- Ordinary users should not need Python, PyTorch, CUDA Toolkit, uv, pip, or a
  development environment.
- The main app should stay small and stable.
- Cloud STT should provide the default usable path.
- Local inference should be optional and isolated behind sidecars or workers.
- App preview can use live transcript updates.
- VRChat Chatbox is not a real-time subtitle terminal and should receive final
  text by default.
- Audio input, speech processing, translation, and output sinks should remain
  separate.
- Diagnostics should make audio, STT, translation, OSC, config, network, and
  local worker failures understandable.
- API keys and secrets must not be written to normal config files or logs.

## Documentation Rules

Authoritative docs are in English.

The root `PROJECT_REWRITE_BRIEF.zh-CN.md` is retained as source material. It is
not the authoritative project plan after this split.
