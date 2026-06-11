# VRC Live Caption Context

VRC Live Caption is a local desktop tool for real-time speech understanding,
caption preview, translation, and output routing for VRChat and desktop voice
communication. It is designed for long always-on sessions.

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
- MVP default STT path: cloud. Long-term default: local STT behind a sidecar
  (see [decisions.md](./decisions.md)).
- First release platform: Windows.

## Where The Rules Live

Principles are not restated here; each has one authoritative home:

- Product scope, requirements, user scenarios, and open questions:
  [product.md](./product.md)
- Runtime boundaries, event semantics, and data flow:
  [architecture.md](./architecture.md)
- Accepted decisions, including defaults, security, and platform choices:
  [decisions.md](./decisions.md)
- Implementation phases: [roadmap.md](./roadmap.md)
- VRChat Chatbox layout and OSC facts:
  [research/vrchat-chatbox-reference.md](./research/vrchat-chatbox-reference.md)

## Documentation Rules

Authoritative docs are in English. Chinese notes use the `.zh-CN.md` suffix.
