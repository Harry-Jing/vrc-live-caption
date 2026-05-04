# Testing

How to verify `VRC Live Caption` locally and keep changes aligned with CI.

## Scope

- This document covers offline checks, opt-in integration and live runs, coverage inspection, and manual runtime validation.
- Use `tests/integration/README.md` for the current integration-test inventory. Use this document for workflow, commands, and guardrails.

## Test organization

- Keep the top-level split to `tests/unit`, `tests/integration`, and explicit `live` markers.
- Prefer file names shaped like `test_<subject>_<behavior>.py`.
- Group related tests under `Test...` classes whose names describe the subject or behavior under test.
- Prefer method names shaped like `test_when_<condition>__then_<result>`.
- Test names must state the subject, triggering condition, and expected result without filler words such as `works`, `handles_case`, or `returns_expected_result`.
- When one test name starts describing multiple unrelated branches or outcomes, split it into separate tests.
- Parameterized tests should include explicit `id=` values.
- Within a file, keep tests ordered as: success path, validation, failure path, shutdown or cleanup, then regression coverage.

## Shared test support

- `tests/support/builders/` is for reusable config or object builders.
- `tests/support/fakes/` is for reusable test doubles such as audio, OSC, and STT fakes.
- `tests/support/harnesses/` is for async drivers, replay helpers, and live-test orchestration.
- Keep `tests/conftest.py` thin and limited to broadly shared fixtures.
- Keep `tests/fixtures/` for static files and sample data only.

## Default workflow

### Fast local loop

Start with the smallest affected test subset, then rerun the default offline flow:

```powershell
uv run pytest -q
```

Notes:

- The default `uv run pytest -q` flow excludes tests marked `integration` and `live`.
- Use marker expressions for targeted runs, for example `uv run pytest -q -m "integration and not live"` or `uv run pytest -q -m "live and openai_live"`.

### CI-aligned checks

Run these from the repository root before opening a PR:

```powershell
uv run pre-commit run --all-files
uv run pytest -q
uv run ruff check
uv run ruff format --check
uv run ty check
```

Notes:

- `uv run pre-commit install --install-hooks` installs the local `pre-commit`, `commit-msg`, and `pre-push` hooks.
- Use `uv run ruff format` only when formatting is intentionally part of the change.
- GitHub Actions `Checks` is the authoritative CI entrypoint.
- The GitHub Actions quality job installs `uv sync --dev --extra local-cpu` so `ty` can resolve the optional FunASR dependency tree.
- If your local environment was created with plain `uv sync`, rerun `uv sync --extra local-cpu` before `uv run ty check` when you need CI parity for the local STT typing paths.
- CI also runs Windows-specific validation, including a CLI smoke test.

## Opt-in integration and live checks

- `integration` marks non-default integration coverage. These tests may stay local and unbillable, such as fake sidecar integration.
- `live` marks networked provider checks. Treat them as opt-in and usually billable.
- Provider-specific markers such as `openai_live`, `iflytek_live`, `deepl_live`, and `google_translate_live` refine the shared `live` marker.
- GitHub Actions exposes one manual `.github/workflows/live-integration.yml` workflow with a `provider` choice of `all`, `openai`, or `iflytek`.

### STT provider checks

#### iFLYTEK

- Command: `uv run pytest -q tests/integration -m "live and iflytek_live"`
- Requires `IFLYTEK_APP_ID`, `IFLYTEK_API_KEY`, `IFLYTEK_API_SECRET`, and outbound network access.
- Replays `tests/fixtures/audio/test.wav` through the async STT runner and asserts readiness, graceful shutdown, and keyword-based transcript success.

#### OpenAI

- Command: `uv run pytest -q tests/integration -m "live and openai_live"`
- Requires `OPENAI_API_KEY` and outbound network access.
- Replays `tests/fixtures/audio/test.wav` through the async STT runner and asserts readiness, graceful shutdown, and keyword-based transcript success.

#### Local STT sidecar

- Command: `uv run pytest -q tests/integration/test_local_stt_sidecar.py -m integration`
- Starts a local websocket sidecar in-process with fake inference models and verifies the repository-local websocket protocol, transcript normalization, and shutdown behavior without loading real FunASR models.

### Translation checks

#### Cloud translation

- Commands:
  - `uv run pytest -q tests/integration/test_translation_live.py -m "live and deepl_live"`
  - `uv run pytest -q tests/integration/test_translation_live.py -m "live and google_translate_live"`
- DeepL live tests require `DEEPL_AUTH_KEY`.
- Google Cloud live tests require ADC plus `GOOGLE_TRANSLATE_PROJECT_ID`.

#### Local TranslateGemma sidecar

- Command: `uv run pytest -q tests/integration/test_local_translation_sidecar.py -m integration`
- Starts a lightweight fake websocket sidecar and exercises the real repository-local TranslateGemma websocket client backend without loading a real model.

## Coverage

Run coverage when you want to inspect gaps or review risky changes:

```powershell
uv run coverage run -m pytest -q
uv run coverage report -m
uv run coverage html
```

- Coverage is package-scoped for `vrc_live_caption` with branch coverage enabled.
- `htmlcov/index.html` is the generated HTML report.

## Manual runtime validation

Use a native Windows terminal for microphone and audio-device validation:

```powershell
uv run vrc-live-caption devices
uv run vrc-live-caption doctor
uv run vrc-live-caption local-stt serve
uv run vrc-live-caption local-translation serve
uv run vrc-live-caption osc-test "OSC test"
uv run vrc-live-caption record-sample --seconds 10
uv run vrc-live-caption run
```

Notes:

- Before `osc-test` or `run`, ensure VRChat has OSC enabled and is listening on the configured host and port.
- Before `doctor` or `run`, ensure the credentials required by the selected `stt.provider` are available, or start `vrc-live-caption local-stt serve` when using `funasr_local`.
- Before `doctor` or `run`, start `vrc-live-caption local-translation serve` when using `translation.provider = "translategemma_local"`.
- `osc-test` does not require STT credentials.
- `local-stt serve` reads the main app config file and uses `[stt.providers.funasr_local]` plus `[stt.providers.funasr_local.sidecar]`.
- `local-translation serve` reads the main app config file and uses `[translation.providers.translategemma_local]` plus `[translation.providers.translategemma_local.sidecar]`.
- Install `uv sync --extra local-cpu` for CPU-only local validation, or `uv sync --extra local-cu130` on Windows/NVIDIA machines for GPU validation.
- `[stt.providers.funasr_local.sidecar].device = "auto"` prefers `cuda:0` when `torch.cuda.is_available()` is true and otherwise falls back to `cpu`.
- `[translation.providers.translategemma_local.sidecar].device = "auto"` and `dtype = "auto"` resolve to `cuda:0` plus `bfloat16` when `torch.cuda.is_available()` is true and otherwise fall back to `cpu` plus `float32`.
- Use `vrc-live-caption.toml.example` as the config reference when preparing manual validation.

## Config touchpoints

- Capture settings live under `[capture]`.
- Pipeline queue and shutdown settings live under `[pipeline]`.
- Retry policy lives under `[stt.retry]`.
- Provider-specific STT settings live under `[stt.providers.<provider>]`.
- Translation settings live under `[translation]`, `[translation.chatbox_layout]`, and `[translation.providers.<provider>]`.
