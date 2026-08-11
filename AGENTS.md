# Agent Instructions

## Working agreement

- Do not modify files unless the user explicitly asks. Discuss requests that
  would change product direction before editing.
- Keep changes scoped and preserve unrelated work already in the worktree.
- Change lockfiles and generated files only through their owning tools. This
  includes `pnpm-lock.yaml`, `src-tauri/Cargo.lock`, and `src-tauri/gen/`.
- Explain any new or upgraded production dependency and why existing
  dependencies are insufficient.
- After substantive project-documentation changes, summarize the result in
  Chinese so the maintainer can review it quickly.
- Use Conventional Commits. For a non-trivial commit, write a body that explains
  the motivation, important behavior or contract changes, and verification;
  do not merely repeat the subject.

## Read by task

- Start documentation work from `docs/README.md`, which identifies the audience
  and source of truth for each document.
- For domain-language changes, read `CONTEXT.md`. It is a glossary, not a spec.
- For product behavior, read `docs/product.md` and the relevant ADR. For runtime
  boundaries, read `docs/architecture.md`. For implementation status, read
  `docs/roadmap.md`.
- For Chatbox OSC, pacing, wrapping, clipping, or layout, read
  `docs/research/vrchat-chatbox-reference.md`. Treat its sourced facts and
  measured results as evidence, not incidental prose cleanup.
- For local recognition, read
  `docs/research/local-recognition-evaluation.md` and the local-inference ADRs.
- For Tauri build or packaging behavior, read
  `docs/research/tauri-build-integration.md` and verify version-sensitive claims
  against the Tauri 2 documentation or source.
- For development setup, checks, contribution workflow, or test placement, read
  `CONTRIBUTING.md`.

## Sources of truth

- GitHub Issues are the work and triage surface. Use the canonical labels
  `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, and
  `wontfix`.
- Code, tests, manifests, and configuration own exact commands, versions, wire
  values, and implementation mechanics. Documentation should preserve the
  cross-cutting model, non-obvious reason, or external constraint instead of
  copying an easy lookup.
- `contracts/` owns shared Rust/TypeScript fixtures and manifests. Extend the
  normalized caption contract before adding a path whose lane, revision, or
  completion semantics it cannot represent.
- Accepted decisions live in `docs/adr/`. Add an ADR only for a costly-to-reverse
  choice with a non-obvious trade-off.

## Code guardrails

- Rust lint policy is authoritative in `src-tauri/Cargo.toml`. Do not weaken it
  to accommodate a change.
- Put a non-trivial Rust unit-test module in a descriptive sibling
  `*_tests.rs` file loaded with `#[cfg(test)]` and `#[path = "..."]`. Reserve
  `src-tauri/tests/` for crate-boundary integration tests.
- Comment intent, invariants, ownership, concurrency, and external constraints;
  let names and tests explain mechanics. Give suppressions a local reason, and
  link temporary workarounds to the issue that removes them.
- Use Tauri v2 APIs. Keep the desktop capability on explicit API permissions;
  do not add `core:default` or wildcard permissions without a documented,
  API-specific requirement.
- Keep credentials, signing keys, tokens, passwords, and secret-bearing `.env`
  files out of the repository. Plaintext credentials must not enter ordinary
  config, logs, diagnostics, or frontend-readable state.
- Keep concrete Tauri event identifiers in the IPC manifest; architecture prose
  may use dotted semantic names.

## Verification

- Use pnpm and the scripts in `package.json`; do not substitute npm or yarn.
- Run the narrowest relevant check while iterating and `pnpm check` for a normal
  completed code change. Use `pnpm check:ci` when reproducing the locked CI gate.
- For docs-only changes, skip builds unless commands, configuration, contracts,
  or documented runtime behavior changed. State what was skipped.
