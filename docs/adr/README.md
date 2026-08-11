# Architecture Decision Records

ADRs record costly-to-reverse choices whose rationale is not obvious from code.
They explain **why**; [architecture](../architecture.md) owns system boundaries,
the [roadmap](../roadmap.md) owns implementation status, and code owns mechanics.

After merging to `main`, ADR numbers are stable identifiers.

## Foundations and trust

- [0001 — Use Tauri, Vue, and Rust](./0001-use-tauri-vue-and-rust.md)
- [0003 — Windows is Tier 1; macOS and Linux are Tier 2](./0003-windows-is-tier-1.md)
- [0004 — Local inference is the long-term default](./0004-local-inference-is-the-long-term-default.md)
- [0005 — Keep secrets out of config and logs](./0005-keep-secrets-out-of-config-and-logs.md)
- [0006 — Keep caption history in memory only](./0006-keep-caption-history-in-memory-only.md)

## Product experience

- [0007 — Publication timing is Completed or Live](./0007-publication-timing-is-completed-or-live.md)
- [0008 — Bilingual output is one asynchronous view](./0008-bilingual-output-is-one-asynchronous-view.md)
- [0009 — Localize the UI in the frontend](./0009-localize-the-ui-in-the-frontend.md)

## Runtime and contracts

- [0010 — Drivers emit full snapshots, not deltas](./0010-drivers-emit-full-snapshots-not-deltas.md)
- [0011 — Stop is a hard cutoff](./0011-stop-is-a-hard-cutoff.md)
- [0012 — Saved settings are not the runtime generation](./0012-saved-settings-are-not-the-runtime-generation.md)
- [0013 — Event delivery is best-effort](./0013-event-delivery-is-best-effort.md)
- [0015 — Identify audio devices by stable id](./0015-identify-audio-devices-by-stable-id.md)
- [0016 — Recognition Modules own path execution](./0016-recognition-modules-own-path-execution.md)

## Cloud recognition

- [0017 — Cloud connections honor the selected proxy route](./0017-cloud-connections-honor-the-selected-proxy-route.md)
- [0018 — Use OpenAI Realtime transcription](./0018-use-openai-realtime-transcription.md)
- [0019 — Reconnect transient recognition failures within one runtime generation](./0019-reconnect-within-one-runtime-generation.md)

## Local inference

- [0020 — Keep local inference out of process](./0020-keep-local-inference-out-of-process.md)
- [0021 — Users choose the local backend preference](./0021-users-choose-the-local-backend.md)

## Translation boundary

- [0022 — Link translations to exact source snapshots](./0022-link-translations-to-exact-source-snapshots.md)
