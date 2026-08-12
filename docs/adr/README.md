# Architecture Decision Records

ADRs record costly-to-reverse choices whose rationale is not obvious from code.
They explain **why**; [architecture](../architecture.md) owns system boundaries,
the [roadmap](../roadmap.md) owns implementation status, and code owns mechanics.

After merging to `main`, ADR numbers are stable identifiers.

## Foundations and trust

- [0001 — Use Tauri, Vue, and Rust](./0001-use-tauri-vue-and-rust.md)
- [0002 — Windows is Tier 1; macOS and Linux are Tier 2](./0002-windows-is-tier-1.md)
- [0003 — Local inference is the long-term default](./0003-local-inference-is-the-long-term-default.md)
- [0004 — Keep secrets out of config and logs](./0004-keep-secrets-out-of-config-and-logs.md)
- [0005 — Keep caption history in memory only](./0005-keep-caption-history-in-memory-only.md)

## Product experience

- [0006 — Publication timing is Completed or Live](./0006-publication-timing-is-completed-or-live.md)
- [0007 — Bilingual output is one asynchronous view](./0007-bilingual-output-is-one-asynchronous-view.md)
- [0008 — Localize the UI in the frontend](./0008-localize-the-ui-in-the-frontend.md)

## Runtime and contracts

- [0009 — Drivers emit full snapshots, not deltas](./0009-drivers-emit-full-snapshots-not-deltas.md)
- [0010 — Stop is a hard cutoff](./0010-stop-is-a-hard-cutoff.md)
- [0011 — Saved settings are not the runtime generation](./0011-saved-settings-are-not-the-runtime-generation.md)
- [0012 — Event delivery is best-effort](./0012-event-delivery-is-best-effort.md)
- [0013 — Identify audio devices by stable id](./0013-identify-audio-devices-by-stable-id.md)
- [0014 — Recognition Modules own path execution](./0014-recognition-modules-own-path-execution.md)

## Cloud recognition

- [0015 — Cloud connections honor explicit routes and endpoints](./0015-cloud-connections-honor-explicit-routes-and-endpoints.md)
- [0016 — Use OpenAI Realtime transcription](./0016-use-openai-realtime-transcription.md)
- [0017 — Reconnect transient recognition failures within one runtime generation](./0017-reconnect-within-one-runtime-generation.md)

## Local inference

- [0018 — Keep local inference out of process](./0018-keep-local-inference-out-of-process.md)
- [0019 — Users choose the local backend preference](./0019-users-choose-the-local-backend.md)

## Translation boundary

- [0020 — Link translations to exact source snapshots](./0020-link-translations-to-exact-source-snapshots.md)
- [0021 — Use OpenAI Responses for completed translation](./0021-use-openai-responses-for-completed-translation.md)
