# Agent Instructions

## Agent skills

- Issue tracker: this repo's GitHub Issues, driven by the `gh` CLI. External
  PRs are not a request surface for triage.
- Triage labels: the five canonical defaults, used as-is (`needs-triage`,
  `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`).
- Domain docs: single-context — the glossary is `CONTEXT.md` at the repo
  root and decisions live in `docs/adr/`.

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
- For substantial work, read `CONTEXT.md`, `docs/product.md`,
  `docs/architecture.md`, the ADRs in `docs/adr/`, and `docs/roadmap.md`.
- For Chatbox layout, wrapping, clipping, or OSC behavior, read
  `docs/research/vrchat-chatbox-reference.md`.
- For local inference work, read `docs/research/local-inference-notes.md`.
- For Tauri 2 behavior or configuration questions, verify against the Tauri 2
  docs rather than relying on memory.

## Where The Rules Live
- Runtime boundaries and data flow: `docs/architecture.md`.
- Accepted decisions and their reasons: the ADRs in `docs/adr/`. Read the
  ones touching the area you are changing.
- Chatbox layout, pacing, and OSC facts:
  `docs/research/vrchat-chatbox-reference.md`.
- Do not change publication, pacing, Stop, secret-handling, or
  backend-selection behavior without reading the matching ADR first.

## Comment Rules
- Comment non-obvious intent, invariants, ownership, concurrency, safety, and
  external constraints. Use module docs for boundary intent; prefer clear names
  and tests for mechanics.
- Keep project-wide behavior in `CONTEXT.md`, `docs/product.md`,
  `docs/architecture.md`, the relevant research doc, or an ADR. Keep local
  comments scoped to their code boundary and update them whenever that
  boundary's behavior or ownership changes.
- Keep suppressions narrow and reasoned: use
  `eslint-disable-next-line ... -- reason`, Rust lint attributes with `reason`,
  and a local `// SAFETY:` immediately before every `unsafe` block.
- Track deferred work in GitHub Issues. Workaround comments cite the issue and
  state when the workaround can be removed.

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

## Rust Test Organization
- Put new non-trivial unit-test modules in a descriptive sibling `*_tests.rs`
  file instead of growing the implementation file. Existing small, focused
  inline test modules may remain, but move one when its fixtures, fakes, or test
  cases obscure the production code; roughly 200 lines is a review signal, not
  a rigid threshold.
- Load a sibling test module explicitly from the implementation file:

  ```rust
  #[cfg(test)]
  #[path = "audio_tests.rs"]
  mod tests;
  ```

  The sibling file contains the body of `tests`; do not wrap it in another
  `mod tests`. It may continue to use `super` to access private implementation
  details without widening the production API.
- Reserve `src-tauri/tests/` for crate-boundary integration tests. Test-only
  seams that production-module unit tests require may stay next to the
  implementation behind `#[cfg(test)]`.

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
- Tauri event names must be valid Tauri event identifiers. Architecture docs
  may use dotted semantic names; the mapping to concrete event names stays
  explicit in `docs/architecture.md`.
- Extend the normalized caption contract before adding a provider path whose
  revision, completion, or lane semantics the current wire shape cannot
  represent.

## Build And Test
- Use the package scripts as the normal quality gates:
  - `pnpm check:frontend` for Prettier, ESLint, Vue typecheck, and Vite build.
  - `pnpm check:rust` for Rust fmt, check, clippy, and tests.
  - `pnpm check` for the normal full local quality gate.
  - `pnpm check:ci` for the combined, locally reproducible CI-style locked gate.
- When running Cargo directly, work from the Tauri Rust project directory:
  - `cd src-tauri && cargo fmt --all`
  - `cd src-tauri && cargo check --workspace --all-targets`
  - `cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings`
  - `cd src-tauri && cargo test --workspace`
- Pre-commit hooks should stay fast enough to run on every commit.
- Pre-push and CI must preserve the full frontend and Rust command sets. They
  may run the component scripts as parallel jobs: pre-push uses
  `pnpm check:frontend` plus `pnpm check:rust`; CI uses `pnpm check:frontend`
  plus `pnpm check:rust:locked` on Windows, macOS, and Linux.
- Rust lint policy lives in `src-tauri/Cargo.toml` under `[lints]`. Do not
  weaken those lints unless the project rule itself changes.
- The Rust toolchain is pinned in `rust-toolchain.toml`. Upgrading Rust is an
  explicit change: update `rust-toolchain.toml`, the toolchain version in both
  GitHub workflows, and `rust-version` in `src-tauri/Cargo.toml` together.
- The Node development-runtime range and pnpm version live in `package.json`;
  the exact Node runtime and checksums live in `pnpm-lock.yaml`. Update them
  through pnpm, and let `pnpm/setup` read `devEngines.runtime` in CI instead of
  hard-coding a second Node version outside an intentional test matrix.
- In CI, install frontend dependencies with
  `pnpm install --frozen-lockfile` before the frontend gate and use locked
  Cargo resolution for the Rust gate. `pnpm check:ci` remains the combined
  local reproduction of those CI checks.
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
