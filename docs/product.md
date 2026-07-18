# Product

## Vision

VRC Live Caption is a desktop voice tool for VRChat: you speak, and other
players see your words — as captions, as a translation, or both. The project
started because the maintainer wanted to talk with players who speak other
languages; real-time captions for same-language listeners turned out to
matter just as much. The goal is one complete, polished voice tool, not a
minimal single-feature app.

The product is designed for long always-on sessions: start it once and keep
playing. English and Chinese are the first language priorities. Cloud STT is
the current baseline; the long-term default is a validated local path that
needs no account or per-minute payment
([ADR 0004](./adr/0004-local-stt-is-the-long-term-default.md)).

## Current scope

The implemented path today:

```text
microphone
  -> application-bounded speech segment
  -> gpt-4o-mini-transcribe
  -> App preview
  -> Completed VRChat Chatbox output
```

This is one working provider path, not a product-wide rule. Implementation
status and what comes next live in [roadmap.md](./roadmap.md).

## User choices

Users answer three independent questions.

### When to publish

- **Completed / 停顿后发送**: publish only completed caption units. Long
  speech is still bounded by natural-boundary-first segmentation plus a hard
  maximum, so one monologue cannot suppress output forever.
- **Live / 实时更新**: also publish ongoing revisions while you speak, when
  the selected model produces them.

If the selected model cannot deliver the chosen mode, the app explains why
and offers two directions: keep the model and pick a supported mode, or keep
the mode and pick a compatible model. It never switches anything silently
([ADR 0006](./adr/0006-publication-timing-is-completed-or-live.md)).

Live describes publication timing, not a promise that every provider begins
text at the same moment. Provider-specific timing must be described honestly
in the UI.

### What to publish

Source only, translation only, or bilingual. Bilingual renders source above
translation in one message and shares the space flexibly, leaning toward the
translation
([ADR 0007](./adr/0007-bilingual-output-is-one-asynchronous-view.md)).

### Where local inference runs

One global preference: CPU or prefer NVIDIA GPU (CUDA), defaulting to CPU.
The app shows the effective backend whenever it differs from the preference,
with the reason, and never switches silently
([ADR 0021](./adr/0021-users-choose-the-local-backend.md)).

## User scenarios

### Source-only Live

A streaming recognizer produces revisable snapshots. The App updates as soon
as useful text exists. Chatbox sends at most one current view per second,
keeps the newest text visible, and skips obsolete revisions. A final
correction is sent only when it differs from the published view.

### Source-only Completed

A real caption unit closes, then Chatbox publishes the completed text. Text
that exceeds one Chatbox view is paginated in order; it is never truncated to
the first or last page.

### Bilingual Live

Source and translation progress independently in one rolling view, and source
may lead. Normal delay can leave the translation one unit behind. If
translation fails, the bilingual choice stays selected, the App shows a
degraded state, and stale translations are dropped until translation
recovers.

### Translation-only Live (provisional)

Whether a translator that starts streaming only after a pause should be
presented as "Live" is an open product test (roadmap Phase 6). The app never
fakes Live by repeatedly re-translating unstable partial text.

### Long uninterrupted speech

A path that owns caption units prefers a natural boundary and enforces a hard
maximum (30 seconds on the current cloud path,
[ADR 0017](./adr/0017-bounded-cloud-units-cap-at-30-seconds.md)), then
immediately continues with a new unit. Exact durations are benchmark
parameters, not user settings.

### Stop

Stop is a hard trust action: capture stops, queued work is discarded, and no
text from the stopped session reaches the App or the Chatbox afterward
([ADR 0011](./adr/0011-stop-is-a-hard-cutoff.md)).

## Requirements

- Provider output is normalized before the UI or Chatbox sees it
  ([ADR 0010](./adr/0010-adapters-emit-full-snapshots-not-deltas.md)).
- Chatbox output is paced, coalesced, length-limited, and layout-aware
  ([ADR 0015](./adr/0015-pace-chatbox-sends-at-one-second.md),
  [research](./research/vrchat-chatbox-reference.md)).
- Capture, recognition, and translation never wait on Chatbox pacing.
- The app never silently changes provider, model, backend, mode, or content
  selection, and never falls back to cloud when a local path fails.
- Secrets never enter ordinary config or logs
  ([ADR 0005](./adr/0005-keep-secrets-out-of-config-and-logs.md)).
- The app discloses when microphone audio is uploaded to a cloud provider
  ([ADR 0009](./adr/0009-cloud-audio-disclosure-lives-in-settings.md)).
- Diagnostics separate audio, provider, translation, worker, backend, OSC,
  config, and network failures, and should be exportable as a redacted
  report.
- The UI is localizable; English and Chinese come first
  ([ADR 0008](./adr/0008-localize-the-ui-in-the-frontend.md)).

## Non-goals in the current scope

Not in the current implementation plan: system-audio capture, speaker
diarization, TTS, virtual microphone output, plugins, mobile support, and
persistent searchable history. Incoming captions, local STT, local
translation, and two-pass recognition are scheduled roadmap or Later items,
not open-ended ideas — see [roadmap.md](./roadmap.md).

Open product questions are tracked as GitHub issues, not as a list in this
file.
