# Contributing to VRC Live Caption

VRC Live Caption accepts issue-led contributions while the project is under
active development. Please coordinate the problem and scope before writing the
change; this keeps parallel work aligned with the product and architecture.

## Start with an issue

Every contribution starts in the
[issue tracker](https://github.com/Harry-Jing/vrc-live-caption/issues), including
small fixes:

1. Search for an existing issue.
2. If one exists, comment with the part you want to handle and wait for scope
   confirmation. Otherwise, open an issue describing the problem, user impact,
   and proposed direction.
3. Begin implementation after the issue is accepted and the scope is clear.
4. Link the issue from the pull request.

A pull request is not a substitute for an issue and may be closed when its scope
was not discussed first.

## Development setup

Install:

- Git;
- the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your
  platform;
- Rust through `rustup` (the repository selects its toolchain in
  [`rust-toolchain.toml`](./rust-toolchain.toml)); and
- the Node and pnpm versions declared in [`package.json`](./package.json).

Clone your fork, then run:

```sh
cd vrc-live-caption
pnpm install --frozen-lockfile
pnpm tauri dev
```

Use pnpm for every package command; do not use npm or Yarn. Dependency
installation also installs the repository's Git hooks.

## Quality gates

The package scripts are the supported entry points for checks:

| Command | Purpose |
|---|---|
| `pnpm check:frontend` | Formatting, lint, frontend tests, type checking, and the Vite build |
| `pnpm check:rust` | Rust formatting, compilation, Clippy, and tests |
| `pnpm check` | Normal full local gate |
| `pnpm check:ci` | Locked CI-style gate; run after `pnpm install --frozen-lockfile` |

Run focused checks while iterating and `pnpm check` before opening a pull
request. Run a frozen pnpm install before `pnpm check:ci` when reproducing CI.
Pre-commit checks formatting, lint, and frontend/Rust buildability; pre-push
runs the complete frontend gate and the locked Rust gate. Changes to platform
integration or user-visible runtime behavior may also need manual Windows/VRChat
testing; record exactly what you tested.

For documentation-only changes, build checks are unnecessary unless the change
alters a command, configuration, or description of runtime behavior. State that
the checks were skipped and why in the pull request.

## Change standards

- Keep a change focused on its issue; avoid unrelated rewrites or formatting.
- Add or update tests for changed behavior and failure paths.
- Preserve the project's explicit behavior: never introduce a silent provider,
  backend, publication-mode, or credential fallback.
- Keep Tauri capabilities and permissions limited to APIs the app uses.
- Explain every new production dependency and why the existing dependencies are
  insufficient.
- Use Conventional Commits. Keep the summary specific, and add a commit body
  when the reason, constraints, or user-visible behavior are not obvious from
  the summary.

Keep non-trivial Rust unit-test modules in a descriptive sibling `*_tests.rs`
file loaded with `#[cfg(test)]` and `#[path = "..."]`. Use `src-tauri/tests/`
only for crate-boundary integration tests; a small, focused inline test module
may remain beside its implementation.

Do not hand-edit lockfiles or generated files. Let pnpm, Cargo, or Tauri update
`pnpm-lock.yaml`, `src-tauri/Cargo.lock`, and `src-tauri/gen/`, and include only
the generated changes required by the issue.

## Documentation and contracts

Use the [documentation guide](./docs/README.md) to update one authoritative
source instead of copying status, commands, or rules.

Changes to persisted configuration or cross-language contracts require extra
care. Follow the cutoff and versioning rules in
[`contracts/README.md`](./contracts/README.md), update the matching fixtures and
tests, and do not make an incompatible V1 change in place.

## Security and user data

- Never commit API keys, tokens, passwords, updater signing keys, populated
  `.env` files, or other credentials.
- Keep credentials in the operating system credential store or process
  environment, never ordinary configuration, fixtures, diagnostics, or logs.
- Do not add microphone audio, caption text from private sessions, device
  identifiers, network targets, or unredacted diagnostic reports to tests or
  issues.
- Use synthetic data in fixtures and screenshots. Review screenshots for names,
  paths, keys, and other identifying information before attaching them.

For a suspected vulnerability, disclose no sensitive details publicly and ask
the maintainer to arrange a private channel.

## Pull request checklist

Before requesting review:

- use a Conventional Commit pull-request title; it becomes the commit subject
  when the project squash-merges the pull request;
- link the accepted issue and explain what changed and why;
- describe automated and manual validation, including anything not run;
- include screenshots for user-interface changes;
- update the one authoritative source for affected documentation;
- wait for the required `Quality Gate` and `Native Build Gate` checks; and
- confirm that the diff contains no secrets, unrelated changes, or hand-edited
  generated files.
