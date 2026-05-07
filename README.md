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
```

Git hooks are managed by Lefthook. `prepare` installs the hooks after dependency
installation, `commit-msg` enforces Conventional Commits, and `pre-push` runs
the full quality gate.

Direct Cargo commands should be run from `src-tauri`, following the Tauri
project layout:

```sh
cd src-tauri
cargo fmt --all
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Markdown documentation is intentionally kept out of automated formatting checks.
Keep docs concise and readable, and reserve build checks for docs changes that
alter commands, configuration, or documented behavior.
