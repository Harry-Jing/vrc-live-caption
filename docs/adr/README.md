# ADR Index

Numbers were assigned in the 2026-07 restructure by how foundational each
decision is: lower numbers shape more of the project. A new ADR takes the
next free number (scan for the highest and add one) and gets a line in the
right group below; do not renumber existing ADRs.

## Foundations

- [0001 — Use Tauri, Vue, and Rust](./0001-use-tauri-vue-and-rust.md): the
  app shell, frontend, and runtime stack
- [0002 — Build outgoing captions first](./0002-build-outgoing-captions-first.md):
  the user speaks, others read; incoming is late-stage
- [0003 — Windows is Tier 1](./0003-windows-is-tier-1.md): the only fully
  validated platform; macOS/Linux stay green in CI
- [0004 — Local inference is the long-term default](./0004-local-stt-is-the-long-term-default.md):
  cloud now; local STT, and eventually local translation, once validated
- [0005 — Keep secrets out of config and logs](./0005-keep-secrets-out-of-config-and-logs.md):
  keys live in the OS credential store

## Product experience

- [0006 — Publication timing is Completed or Live](./0006-publication-timing-is-completed-or-live.md):
  the two timing modes and the no-silent-switching rule
- [0007 — Bilingual output is one asynchronous view](./0007-bilingual-output-is-one-asynchronous-view.md):
  source above translation, capacity leans toward translation
- [0008 — Localize the UI in the frontend](./0008-localize-the-ui-in-the-frontend.md):
  English and Chinese; the backend emits codes only
- [0009 — Cloud audio disclosure lives in Settings](./0009-cloud-audio-disclosure-lives-in-settings.md):
  a persistent line instead of dialogs or banners

## Runtime core

- [0010 — Adapters emit full snapshots, not deltas](./0010-adapters-emit-full-snapshots-not-deltas.md):
  ongoing/completed full-text snapshots with revisions
- [0011 — Stop is a hard cutoff](./0011-stop-is-a-hard-cutoff.md): no late
  text after Stop, ever
- [0012 — Saved settings are not the running session](./0012-saved-settings-are-not-the-running-session.md):
  desired state versus the immutable active selection
- [0013 — Event delivery is best-effort](./0013-event-delivery-is-best-effort.md):
  the UI resynchronizes from revisioned snapshots
- [0014 — Diagnostic codes are category.detail](./0014-diagnostic-codes-are-category-detail.md):
  stable codes; prose is fallback only
- [0025 — Reconnect transient recognition failures within one runtime generation](./0025-reconnect-within-one-runtime-generation.md):
  fresh attempts, visible backoff, and no ambiguous audio replay
- [0026 — Recognition Modules own attempt execution](./0026-recognition-modules-own-attempt-execution.md):
  continuous audio outside; unitization, attempts, and I/O inside

## Chatbox output

- [0015 — Pace Chatbox sends at one second](./0015-pace-chatbox-sends-at-one-second.md):
  the measured 1000 ms floor between actual attempts
- [0016 — Signal speech activity with the typing indicator](./0016-signal-speech-activity-with-the-typing-indicator.md):
  reasserted every four seconds while active
- [0017 — The bounded cloud path caps units at 30 seconds](./0017-bounded-cloud-units-cap-at-30-seconds.md):
  silence boundary plus a hard maximum

## Cloud path

- [0018 — Default to OpenAI for cloud STT](./0018-default-to-openai-for-cloud-stt.md):
  bounded `gpt-4o-mini-transcribe` requests; superseded by ADR 0024
- [0019 — Follow the system proxy; plan a relay API option](./0019-follow-system-proxy-plan-relay-api.md):
  China-friendly network access, honestly scoped
- [0024 — Use OpenAI Realtime transcription](./0024-use-openai-realtime-transcription.md):
  two exact model paths behind the recognition-session seam

## Local path

- [0020 — Keep local inference out of process](./0020-keep-local-inference-out-of-process.md):
  workers and sidecars; crashes stay contained
- [0021 — Users choose the local backend](./0021-users-choose-the-local-backend.md):
  CPU or prefer-CUDA, no auto-selection

## Mechanics

- [0022 — Identify audio devices by stable id](./0022-identify-audio-devices-by-stable-id.md):
  CPAL ids, never display names
- [0023 — Keep session history in memory only](./0023-keep-session-history-in-memory-only.md):
  nothing persists until history/export is built
