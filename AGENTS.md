# Agent Instructions

## Working agreement

- Do not modify files unless the user explicitly asks. Discuss requests that
  would change product direction before editing.
- Keep changes scoped and preserve unrelated work already in the worktree.
- Change lockfiles and generated files only through their owning tools. This
  includes `pnpm-lock.yaml`, `src-tauri/Cargo.lock`, and `src-tauri/gen/`.
- Use pnpm for package commands; do not substitute npm or Yarn.
- Explain any new or upgraded production dependency and why existing
  dependencies are insufficient.
- After substantive project-documentation changes, summarize the result in
  Chinese so the maintainer can review it quickly.
- Use Conventional Commits. Review the staged diff before committing and
  pre-wrap every commit-message line to at most 100 characters (`git commit -m`
  does not wrap text). Keep any body concise and limited to non-obvious context.

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
- For code, test, setup, or contribution work, read `CONTRIBUTING.md` for test
  placement, required verification, and workflow.

## Sources of truth

- GitHub Issues are the work and triage surface. Give each issue one type label:
  `bug`, `enhancement`, `documentation`, `dependencies`, `maintenance`, or
  `question`. While it is open, add one state label: `needs-triage`,
  `needs-info`, `ready-for-agent`, or `ready-for-human`. Remove the state label
  when the issue closes, and close rejected work with `wontfix`.
- `contracts/` owns shared Rust/TypeScript fixtures and manifests. Extend the
  normalized caption contract before adding a path whose lane, revision, or
  completion semantics it cannot represent.

## Issues, pull requests, and commits

- Follow "Issues and pull requests" in `CONTRIBUTING.md` and the templates in
  `.github/`. They take precedence over the formats in skills such as `to-spec`,
  `to-tickets`, and `triage`.
- `gh issue create --body` and `gh pr create --body` skip the templates, so use
  their section headings yourself. Pass `--label`, and use `--parent`,
  `--blocked-by`, or `--blocking` instead of "Parent" or "Blocked by" sections.
- Map skill sections onto the template: "What to build" or "Desired behavior"
  goes under "Expected behavior", and acceptance criteria go under "Done when".
  A spec becomes a short parent issue that links the roadmap, product spec, or
  ADR instead of repeating them.
- Keep progress notes, CI run links, test counts, and hashes out of issue and
  pull request bodies. Add a comment only when someone needs that evidence.

## Code guardrails

- Rust lint policy is authoritative in `src-tauri/Cargo.toml`. Do not weaken it
  to accommodate a change.
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
