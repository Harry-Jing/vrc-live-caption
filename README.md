# VRC Live Caption

Desktop app that turns microphone speech into VRChat Chatbox captions:
audio is captured locally, sent through the selected recognition path
(currently OpenAI), normalized into captions, and published to VRChat over
OSC.

Authoritative project direction lives in [docs/](./docs/).

## Development

Install Git, the platform-specific
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/), and Rust
through `rustup`. The repository's `rust-toolchain.toml` selects the Rust
version plus the Clippy and rustfmt components.

The frontend requires pnpm 11. The repository selects the pnpm version and a
Node 24 runtime in `package.json`; `pnpm-lock.yaml` pins the exact project Node
build and checksums. A normal machine may provide Node `>=24.11.0 <25.0.0` for
the first install. A standalone-pnpm environment with no system Node can
bootstrap one first:

```sh
pnpm runtime set node 24 --global
```

Then install and build with the locked project toolchain:

```sh
pnpm install --frozen-lockfile
pnpm build
(cd src-tauri && cargo check)
pnpm tauri dev
```

After installation, pnpm scripts and Git hooks use the project Node runtime;
they do not depend on whichever Node version happens to be first on the shell
`PATH`.

## Quality Gates

Use the project scripts instead of spelling out tool commands in day-to-day
work:

```sh
pnpm check:frontend
pnpm check:rust
pnpm check
pnpm check:ci
```

Git hooks are managed by Lefthook. `prepare` installs the hooks after dependency
installation, `pre-commit` runs the fast local gate, `commit-msg` enforces
Conventional Commits, and `pre-push` runs the full quality gate.

GitHub Actions runs the CI gate on push and pull request. CI uses frozen pnpm
installs, locked Cargo resolution, dependency caching, frontend checks, and Rust
checks.

Direct Cargo commands should be run from `src-tauri`, following the Tauri
project layout:

```sh
cd src-tauri
cargo fmt --all
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Rust lint policy is configured in `src-tauri/Cargo.toml` under `[lints]` so the
same restrictions apply locally and in CI.

Markdown documentation is intentionally kept out of automated formatting checks.
Keep docs concise and readable, and reserve build checks for docs changes that
alter commands, configuration, or documented behavior.
