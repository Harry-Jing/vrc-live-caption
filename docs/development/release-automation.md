# Release Automation

How `VRC Live Caption` publishes versions, changelogs, tags, and GitHub Releases.

## Current setup

- `main` is the only automated release branch.
- GitHub Actions runs `.github/workflows/release-please.yml` on pushes to `main` and on manual dispatch.
- GitHub Actions runs `.github/workflows/pr-title.yml` on pull requests to validate Conventional PR titles.
- The workflow uses `googleapis/release-please-action@v4` with:
  - `release-please-config.json`
  - `.release-please-manifest.json`
- Release Please PR titles use `chore(release): 🔧 release ${version}` from `release-please-config.json`.
- `release-please` manages:
  - `pyproject.toml` version updates
  - `CHANGELOG.md`
  - Git tags
  - GitHub Releases

## Repository setup

- Configure a repository secret named `RELEASE_PLEASE_TOKEN`.
- Use a PAT for `RELEASE_PLEASE_TOKEN` so release PRs, tags, and releases can trigger other workflows.
- In GitHub repository settings, enable **Allow GitHub Actions to create and approve pull requests**.
- If you use squash merges, enable **Default to PR title for squash merge commits** so merged PRs keep the validated Conventional title.

## First release baseline

- The repository has **not** published a release yet.
- `.release-please-manifest.json` stays empty until the first release PR is merged.
- `release-please-config.json` uses `bootstrap-sha = b89259cc817b56c8a610f1831de62ec8c88093c4`, which is the `main` branch's initial commit.
- The first release PR will be based on releasable commits that land on `main` after that initial commit.
- We do **not** backfill a historical `v0.1.0` tag or GitHub Release before the first automated release.

## Working rules

- Use Conventional Commits so `release-please` can derive the next version and changelog entries.
- Human-authored PR titles use the same `<type>[optional scope][!]: <emoji> <description>` format as local commit messages.
- PR title and commit-message validation expect gitmoji.dev's canonical forms, for example `📦️`, `⚡️`, `♻️`, and `⏪️`.
- Release Please PRs are exempt from PR title validation when GitHub applies `autorelease: pending` or `autorelease: triggered`.
- For the first release, an empty manifest is expected and correct.
- If you squash merge release-worthy work into `main`, keep the squash commit releasable, for example `feat`, `fix`, or `docs`.
- `release-please-config.json` controls changelog presentation only; changelog sections do not change which commit types trigger a release PR.
