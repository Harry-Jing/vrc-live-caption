# Architecture Decision Records

ADRs record costly-to-reverse choices whose rationale is not obvious from code.
They explain **why**; [architecture](../architecture.md) owns system boundaries,
the [roadmap](../roadmap.md) owns implementation status, and code owns mechanics.

Numbers are stable identifiers. Gaps are intentional; the next ADR is 0029,
and removed numbers are not reused.

## Foundations

- [0001 — Use Tauri, Vue, and Rust](./0001-use-tauri-vue-and-rust.md)
- [0002 — Build outgoing captions first](./0002-build-outgoing-captions-first.md)
- [0003 — Windows is Tier 1](./0003-windows-is-tier-1.md)
- [0004 — Local inference is the long-term default](./0004-local-inference-is-the-long-term-default.md)
- [0005 — Keep secrets out of config and logs](./0005-keep-secrets-out-of-config-and-logs.md)

## Product experience

- [0006 — Publication timing is Completed or Live](./0006-publication-timing-is-completed-or-live.md)
- [0007 — Bilingual output is one asynchronous view](./0007-bilingual-output-is-one-asynchronous-view.md)
- [0008 — Localize the UI in the frontend](./0008-localize-the-ui-in-the-frontend.md)

Chatbox pacing, typing, layout, and wrapping follow the measured
[VRChat Chatbox reference](../research/vrchat-chatbox-reference.md) rather than
separate parameter ADRs.

## Runtime and contracts

- [0010 — Drivers emit full snapshots, not deltas](./0010-drivers-emit-full-snapshots-not-deltas.md)
- [0011 — Stop is a hard cutoff](./0011-stop-is-a-hard-cutoff.md)
- [0012 — Saved settings are not the runtime generation](./0012-saved-settings-are-not-the-runtime-generation.md)
- [0013 — Event delivery is best-effort](./0013-event-delivery-is-best-effort.md)
- [0014 — Diagnostic codes are category.detail](./0014-diagnostic-codes-are-category-detail.md)
- [0025 — Reconnect within one runtime generation](./0025-reconnect-within-one-runtime-generation.md)
- [0026 — Recognition Modules own path execution](./0026-recognition-modules-own-path-execution.md)
- [0027 — Link translations to exact source snapshots](./0027-link-translations-to-exact-source-snapshots.md)

## Cloud and local inference

- [0019 — Cloud connections honor the selected proxy route](./0019-cloud-connections-honor-the-selected-proxy-route.md)
- [0020 — Keep local inference out of process](./0020-keep-local-inference-out-of-process.md)
- [0021 — Users choose the local backend](./0021-users-choose-the-local-backend.md)
- [0024 — Use OpenAI Realtime transcription](./0024-use-openai-realtime-transcription.md)

## Platform identity and privacy

- [0022 — Identify audio devices by stable id](./0022-identify-audio-devices-by-stable-id.md)
- [0023 — Keep caption history in memory only](./0023-keep-caption-history-in-memory-only.md)
