# Contributing to VRC Live Caption

VRC Live Caption accepts issue-led contributions while the project is under
active development. Please coordinate the problem and scope before writing the
change; this keeps parallel work aligned with the product and architecture.

## Start with an issue

Changes start in the
[issue tracker](https://github.com/Harry-Jing/vrc-live-caption/issues):

1. Search for an existing issue.
2. If one exists, comment with the part you want to handle and wait for scope
   confirmation. Otherwise, open an issue that describes the problem and the
   result you want.
3. Begin implementation after the issue is accepted and the scope is clear.
4. Link the issue from the pull request.

Small, self-explanatory fixes, such as typos or broken links, can go straight to
a pull request; group several of them into one. Other pull requests whose scope
was not discussed first may be closed.

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
| `pnpm check:ci` | `pnpm check` with locked Cargo dependencies; run after `pnpm install --frozen-lockfile` |

Run focused checks while iterating and `pnpm check` before opening a pull
request. `pnpm check:ci` matches the Quality workflow's formatting, lint, build,
and test steps, except that CI runs the Rust tests with nextest (see below);
the dependency audits and native builds run only in CI.
Pre-commit checks formatting, lint, and frontend/Rust buildability; pre-push
runs the complete frontend gate and the locked Rust gate. Changes to platform
integration or user-visible runtime behavior may also need manual Windows/VRChat
testing; record exactly what you tested.

For documentation-only changes, build checks are unnecessary unless the change
alters a command, configuration, or description of runtime behavior. State that
the checks were skipped and why in the pull request.

### Isolated test runs and CI evidence

Concurrency tests must drive the lifecycle order they claim to cover. Use
module-owned fixtures with explicit admission, entry, completion, and
quiescence milestones; advance policy time from the test after the preceding
milestone is acknowledged. Wall-clock timeouts are bounded deadlock diagnostics,
not evidence that another thread has reached a state. Make negative assertions
after the relevant owner or effect is quiescent, and distinguish an empty
channel from a disconnected one. Test-owned threads, sockets, and blockers must
converge on cleanup when setup or an assertion fails. Keep outcomes typed and
diagnostics free of credentials, private captions, audio, and provider bodies.

Required Rust tests use `cargo-nextest` 0.9.143. Each test runs in its own
process, the repository assigns explicit timeout classes in
`src-tauri/.config/nextest.toml`, and the required run always uses four workers
and zero retries on Linux, Windows, and macOS. Rust documentation tests remain a
separate `cargo test --doc` step because nextest does not execute them.

Install the pinned runner and reproduce the required Rust test selection with:

```sh
cargo install cargo-nextest --version 0.9.143 --locked
cd src-tauri
cargo nextest run --workspace --locked --profile ci --retries 0
cargo test --workspace --doc --locked
```

Set `PROPTEST_RNG_SEED` to the value recorded by CI before running nextest to
replay the same property-test input stream. The following profiles are stable
entry points for narrower investigations; their definitions, rather than
shell-authored test-name substrings, own the selections:

| Profile | Selection |
|---|---|
| `risk-runtime-coordination` | Runtime ownership, lifecycle, and recognition coordination |
| `risk-translation` | Translation Module and Responses Adapter |
| `risk-loopback-network` | Host resolution, OSC, proxy, WebSocket, and Responses loopback tests |
| `risk-timeout` | Owner, deadline, and intentionally bounded wait paths |
| `risk-cancellation` | Stop, disconnect, cancellation, and cleanup paths |
| `risk-property-regression` | Chatbox property and regression suites |
| `risk-all` | Union used by the scheduled stress workflow |

When adding or moving an owner/concurrency, loopback/network, or
property/regression test, update its timeout group and every relevant risk
profile in the same change. Use `cargo nextest show-config test-groups` and
`cargo nextest list --profile <profile>` to verify the resulting selections.

For example:

```sh
cargo nextest run --workspace --locked --profile risk-translation --retries 0
```

When the required Rust run fails, CI starts two separately labeled diagnostic
runs over the named loopback/network, owner/concurrency, and
property/regression test groups: first with the same four-worker schedule, then
with one worker. They are new zero-retry runs, not retries that can change the
required result. Download the `rust-test-results-<os>` artifact for the original
JUnit report, captured failure output, environment and seed metadata, doctest
output, and any diagnostic JUnit reports. The `frontend-test-results-<os>` artifact
contains the required Vitest JUnit report.

Vitest's normal pool, isolation, worker count, timeouts, retry count, and
unshuffled order are defined in `vitest.config.ts`. To replay a scheduled
frontend order, use the seed from the `frontend-stress` artifact:

```sh
pnpm exec vitest run --retry=0 --maxWorkers=4 \
  --sequence.shuffle.files --sequence.shuffle.tests --sequence.seed=12345
```

The weekly `Test Stress` workflow repeats only repository-owned fake-driver,
loopback, coordination, Translation, timeout/cancellation, and property groups.
It runs them with one and eight nextest workers on all three desktop platforms;
it does not use credentials, live providers, a microphone, or VRChat.

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

Keep large Rust-only regression inputs under `src-tauri/testdata/<area>/`, with
their provenance and update procedure documented beside them. Reserve
`contracts/` for formats that cross a runtime, persistence, or language
boundary.

Do not hand-edit lockfiles or generated files. Let pnpm, Cargo, or Tauri update
`pnpm-lock.yaml`, `src-tauri/Cargo.lock`, and `src-tauri/gen/`, and include only
the generated changes required by the issue.

## Dependency updates

Dependabot opens grouped weekly updates. Keep its ignore rules in
[`.github/dependabot.yml`](./.github/dependabot.yml), not in `@dependabot ignore`
comments, so they stay reviewable. GitHub's dependency graph reads
`package.json` but not `pnpm-lock.yaml`, so the `pnpm audit --audit-level high`
step in CI is the only check for transitive npm advisories.

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

Report a suspected vulnerability privately, as described in
[SECURITY.md](./SECURITY.md).

## Issues and pull requests

Write for someone who has not read the code: say what users notice first, use
plain words, and keep it short. Most issues and pull requests fit in about 150
words. Write in English; bug reporters may use Chinese. Do not hard-wrap issue or
pull request text, because GitHub shows every line break.

Put each fact in one place:

| Content | Where |
|---|---|
| The problem and the result we want | Issue |
| How it was solved, what was tested, and what was not | Pull request |
| Why the change was needed, for `git log` readers | Squash commit body |
| Decisions and rules that outlive the change | ADR or docs |
| Implementation status | [Roadmap](./docs/roadmap.md) |
| Progress, CI runs, and other evidence | Comments, only when useful |

### Issues

- Title: plain words, no `feat:`-style prefix, at most 70 characters. A bug names
  the symptom ("Captions containing a NUL character are cut off in VRChat");
  other work names the outcome ("Keep each Chatbox page visible long enough to
  read").
- Body: the template's "Current behavior", "Expected behavior", and "Done when"
  sections. Leave out implementation plans, file names, and progress logs.
- Labels: one type label (`bug`, `enhancement`, `documentation`, `dependencies`,
  `maintenance`, or `question`) and, while the issue is open, one state label.
- Large work: a parent issue with sub-issues, linked with GitHub's "blocked by"
  where order matters.

### Pull requests

- Title: a Conventional Commit, `type(scope): summary`, without issue or pull
  request numbers. It becomes the squash commit subject. Common scopes are listed
  in [`commitlint.config.mjs`](./commitlint.config.mjs).
- Body: `Closes #N`, a short summary, and testing notes that say what was not
  tested. Add screenshots for user-interface changes. Skip file-by-file change
  lists, test names, and CI run IDs.
- Before requesting review, update the one authoritative document for anything
  you changed, wait for the required checks, and make sure the diff has no
  secrets, unrelated changes, or hand-edited generated files.
- Merge with a squash. Keep the subject equal to the title without GitHub's
  `(#N)`, and use the summary as the commit body, followed by `Closes #N`.
