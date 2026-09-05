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

### Isolated test runs and CI evidence

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

- use a Conventional Commit pull-request title without issue or pull-request
  numbers; when squash-merging, make the final commit subject match the
  pull-request title exactly and remove GitHub's automatically appended
  `(#N)`;
- link the accepted issue and explain what changed and why;
- describe automated and manual validation, including anything not run;
- include screenshots for user-interface changes;
- update the one authoritative source for affected documentation;
- wait for the required `Quality Gate` and `Native Build Gate` checks; and
- confirm that the diff contains no secrets, unrelated changes, or hand-edited
  generated files.
