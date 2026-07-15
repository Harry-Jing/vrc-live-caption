# Agent Instructions

## Purpose
- Keep this file practical and project-specific. Prefer instructions that prevent
  real mistakes in this Tauri/Vue/Rust app over broad style preferences.
- If these instructions conflict with a direct user request, follow the user's
  latest request and call out the conflict.

## Working Rules
- Do not modify files unless the user explicitly asks.
- If a request is unclear or changes project direction, discuss the approach first.
- Keep edits scoped; do not rewrite unrelated code, docs, or formatting.
- Preserve existing user changes in the worktree.
- Never hand-edit lockfiles or generated files (`pnpm-lock.yaml`,
  `src-tauri/Cargo.lock`, `src-tauri/gen/`). Change them only through pnpm,
  Cargo, or Tauri tooling.
- After making substantive changes to project docs, summarize what changed in
  Chinese in the conversation so the maintainer can review it quickly.
- When adding or upgrading a production dependency, explain why it is needed and
  why existing dependencies are not enough. Small dev-only tooling may be added
  when it directly supports checks the project already expects.

## Read First
- Start with `docs/README.md`.
- For substantial work, read `docs/CONTEXT.md`, `docs/product.md`,
  `docs/architecture.md`, `docs/decisions.md`, and `docs/roadmap.md`.
- For Chatbox layout, wrapping, clipping, or OSC behavior, read
  `docs/research/vrchat-chatbox-reference.md`.
- For local inference work, read `docs/research/local-inference-notes.md`.
- For Tauri 2 behavior or configuration questions, verify against the Tauri 2
  docs rather than relying on memory.

## Project Invariants
- The implemented Phase 1 baseline is microphone -> bounded cloud STT -> App
  preview -> completed-only VRChat Chatbox output. This is current adapter
  behavior, not a global provider contract.
- Frontend should not process raw audio.
- Provider raw events should be normalized before reaching UI-facing consumers.
- Chatbox is an output sink, not the runtime center.
- Chatbox publication is resolved from the selected provider path's per-lane
  capabilities, selected content lanes, and the user's publication mode; do not
  impose one global rolling or completed-only policy.
- The public publication modes are Completed and Live. Do not reintroduce a
  public Automatic mode or treat a timer/checkpoint as provider completion.
- Normalize provider output into full source/translation snapshots with
  monotonic revisions and ongoing/completed state. Do not give the unused
  `stable` value new semantics.
- Rolling Chatbox revisions are coalesced latest-wins per active publication;
  Chatbox pacing must not block audio, provider, or translation processing.
- Keep text-send attempts at least 1000 ms apart from the previous actual
  attempt. Do not exploit initial burst capacity or immediately retry a failed
  OSC attempt.
- Completed Chatbox pages remain ordered in a bounded queue; Live output is one
  recomputed recent-content viewport, not a queue of historical screens.
- Runtime Stop is a hard generation boundary: no late caption or translation
  result reaches either App or Chatbox after Stop.
- Translation should not block audio capture or STT.
- Never silently change provider, model, backend, publication mode, content
  selection, or local/cloud path. Expose both local backend preference and the
  effective backend when they differ.
- Two-pass recognition is a low-priority future topology, not a current model
  capability, normal setting, or implementation requirement.
- API keys and secrets must not be written to normal config files or logs.

## Rust Rules
- Prefer safe Rust in app/runtime code. Do not introduce `unsafe` without a
  narrow technical reason and prior discussion.
- Avoid `unwrap()` and `expect()` in recoverable runtime paths, especially Tauri
  commands, audio/STT/OSC/config code, provider adapters, diagnostics, and secret
  handling. Return `AppResult<T>` / `Result<T, AppError>` for those paths.
- Tauri process-startup code may either return `Result` from `main`/`run`, or use
  one clear `expect` around unrecoverable app startup failure. Prefer `Result`
  when it stays simple.
- Non-panicking fallbacks such as `unwrap_or` and `unwrap_or_else` are allowed
  when the fallback value is correct.
- Do not leave `panic!`, `todo!`, `unimplemented!`, or `dbg!` in production Rust
  under `src-tauri/src/`.
- Clones are acceptable when ownership must cross an event, async, thread, UI, or
  handle boundary. Avoid casual clones of large buffers, audio frames, provider
  payloads, or hot-path data; justify those explicitly.
- Small `cfg` attributes or tiny platform guards are fine inline. Move
  platform-specific behavior into a `cfg(...)` module when it grows beyond a
  small local branch or changes public behavior.

## Tauri Rules
- Use Tauri v2 APIs only.
- Keep permissions and capabilities tied to APIs the app actually uses.
- The current desktop capability uses an explicit command/event allowlist. Keep
  it narrow; do not reintroduce `core:default` without an API-specific reason
  and review.
- Do not use wildcard permissions unless official Tauri docs require them for a
  specific feature and the reason is documented.
- Do not commit updater private signing keys, passwords, tokens, or `.env` files
  containing secrets. Updater private keys must come from the environment or an
  external secure store.
- Remove template plugins, npm packages, Cargo crates, and permissions that are
  no longer used.

## Runtime Contracts
- Keep UI-facing runtime events normalized; provider raw events should not leak
  into Vue components or output sinks.
- Tauri event names must be valid Tauri event identifiers. If architecture docs
  use semantic names such as `transcript.partial`, keep the mapping to concrete
  Tauri event names explicit in code or docs.
- The current wire contract can represent partial/final transcript semantics.
  The current OpenAI bounded-request adapter emits only final transcripts and
  publishes completed text. Extend the normalized contract before adding a
  provider path with different revision, completion, or source/target-lane
  semantics.
- Never forward every provider delta directly to VRChat or queue stale rolling
  revisions. Publication eligibility and OSC pacing are separate decisions.

## Build And Test
- Use the package scripts as the normal quality gates:
  - `pnpm check:frontend` for Prettier, ESLint, Vue typecheck, and Vite build.
  - `pnpm check:rust` for Rust fmt, check, clippy, and tests.
  - `pnpm check` before pushing or when a change crosses frontend/Rust/Tauri
    contracts.
  - `pnpm check:ci` for CI-style locked dependency checks.
- When running Cargo directly, work from the Tauri Rust project directory:
  - `cd src-tauri && cargo fmt --all`
  - `cd src-tauri && cargo check --workspace --all-targets`
  - `cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings`
  - `cd src-tauri && cargo test --workspace`
- Pre-commit hooks should stay fast enough to run on every commit. Pre-push and
  CI should run the full quality gate.
- Rust lint policy lives in `src-tauri/Cargo.toml` under `[lints]`. Do not
  weaken those lints unless the project rule itself changes.
- The Rust toolchain is pinned in `rust-toolchain.toml`. Upgrading Rust is an
  explicit change: update `rust-toolchain.toml`, the toolchain version in both
  GitHub workflows, and `rust-version` in `src-tauri/Cargo.toml` together.
- In CI, use locked installs and locked Cargo resolution:
  - `pnpm install --frozen-lockfile`
  - `pnpm check:ci`
- Use pnpm for all package scripts; never npm or yarn. There is no standalone
  typecheck script: `pnpm build` runs Vite first so generated Nuxt UI declarations
  are current, then runs `vue-tsc --build`. Lint and format have their own scripts
  (`pnpm lint`, `pnpm format:check`).
- For docs-only changes, run no build checks unless the change affects commands,
  configuration, or documented behavior; state that checks were skipped because
  the change was docs-only.
- Markdown files are prose and project guidance. Keep them clear and readable,
  but do not add Markdown formatting to automated checks unless explicitly
  requested.
