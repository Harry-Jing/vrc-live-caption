# Agent Instructions

## Rules
- Do not modify files unless the user explicitly asks.
- If a request is unclear or changes project direction, discuss the approach first.
- Keep edits scoped; do not rewrite unrelated code, docs, or formatting.
- Preserve existing user changes in the worktree.

## Read First
- Start with `docs/README.md`.
- For substantial work, read `docs/CONTEXT.md`, `docs/product.md`, `docs/architecture.md`, `docs/decisions.md`, and `docs/roadmap.md`.
- For Chatbox layout, wrapping, clipping, or OSC behavior, read `docs/research/vrchat-chatbox-reference.md`.
- For Tauri 2 questions, you may refer to https://github.com/tauri-apps/tauri-docs/tree/v2/src/content/docs.

## Project Invariants
- MVP focus: microphone -> STT -> App preview -> final-only VRChat Chatbox output.
- Frontend should not process raw audio.
- Provider raw events should be normalized before reaching UI-facing consumers.
- Chatbox is an output sink, not the runtime center.
- Translation should not block audio capture or STT.
- API keys and secrets must not be written to normal config files or logs.

## Build And Test
- TODO
