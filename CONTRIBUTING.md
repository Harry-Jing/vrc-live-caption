# Contributing

Minimal contributor workflow for `VRC Live Caption`.

## Read First

- [Environment](./docs/development/environment.md)
- [Release Automation](./docs/development/release-automation.md)
- [Testing](./docs/development/testing.md)
- [Docstrings](./docs/development/docstrings.md)

## Setup

```shell
uv sync
uv run pre-commit install --install-hooks
```

## Before Opening a PR

```shell
pre-commit run --all-files
uv run pytest -q
uv run ruff check
uv run ruff format --check
uv run ty check
```

If you created your environment with plain `uv sync`, rerun `uv sync --extra local-cpu` before `uv run ty check` so local typing matches the GitHub Actions quality job for the optional FunASR paths.

## Commit Messages

Use [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/) with the same type list as [`@commitlint/config-conventional`](https://github.com/conventional-changelog/commitlint/tree/master/%40commitlint/config-conventional), and add a matching [gitmoji](https://gitmoji.dev/) after `: `.

```text
<type>[optional scope][!]: <emoji> <description>
```

Types:

- `build`: 📦️ build system, packaging, compiled assets, or dependency packaging changes
- `chore`: 🔧 maintenance, repo housekeeping, or non-user-facing config/script upkeep
- `ci`: 👷 CI workflow and automation changes
- `docs`: 📝 documentation changes
- `feat`: ✨ user-facing features
- `fix`: 🐛 bug fixes
- `perf`: ⚡️ performance improvements
- `refactor`: ♻️ refactors without behavior changes
- `revert`: ⏪️ revert previous changes
- `style`: 🎨 formatting and style-only changes
- `test`: ✅ tests and test harness changes

Notes:

- Use `!` for breaking changes, for example `feat(api)!: 💥 remove legacy auth`.
- Keep the emoji aligned with the intent of the type. The list above is the repository's recommended type-to-gitmoji pairing.

Example commit messages:

```text
feat(chatbox): ✨ add source-target layout mode
fix(config): 🐛 handle missing .env file
ci(release): 👷 add release-please workflow
style(cli): 🎨 normalize help text wrapping
feat(api)!: 💥 remove legacy auth
```

Release automation notes:

- `main` uses `release-please` to manage release PRs, versions, tags, and GitHub Releases.
- Before the first release, `.release-please-manifest.json` is intentionally empty.
- `release-please-config.json` uses the same commit type list for changelog sections, with gitmoji section titles.
- Release Please changelog presentation is broader than release triggering. For Python projects, release PR creation still mainly depends on releasable types such as `feat`, `fix`, and `docs`.

## Pull Requests

- Keep PRs small and focused.
- Explain why the change is needed.
- Make sure checks pass.
