# VRC Live Caption

Desktop app foundation for the VRC Live Caption rewrite.

Authoritative project direction lives in [docs/](./docs/).

## Development

```sh
pnpm install
pnpm build
(cd src-tauri && cargo check)
pnpm tauri dev
```

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
