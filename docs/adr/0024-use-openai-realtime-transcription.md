# Use OpenAI Realtime transcription

Date: 2026-08

Status: accepted, implemented, and validated on native Windows/VRChat.

Supersedes
[ADR 0018](./0018-default-to-openai-for-cloud-stt.md).

## Context

The previous OpenAI implementation uploaded application-bounded WAV segments
to the transcriptions HTTP endpoint with `gpt-4o-mini-transcribe`. That path
proved the Completed product experience, but it was a shallow provider seam:
capture segmentation, WAV encoding, HTTP upload, model choice, and recognition
semantics were coupled in one implementation.

The release needs one honest OpenAI choice for Completed captions and one that
can also drive Live captions. Future local STT must join the same recognition
pipeline without inheriting OpenAI transport concepts.

OpenAI documents both [`gpt-transcribe`](https://developers.openai.com/api/docs/models/gpt-transcribe)
and [`gpt-live-transcribe`](https://developers.openai.com/api/docs/models/gpt-live-transcribe)
for Realtime transcription. The
[Realtime transcription guide](https://developers.openai.com/api/docs/guides/realtime-transcription)
defines the audio-buffer and transcript-event protocol, and the
[WebSocket guide](https://developers.openai.com/api/docs/guides/realtime-websocket)
defines the native/server connection path.

## Decision

The OpenAI recognition catalog for the release has exactly two entries:

| Model | Recognition behavior | Supported publication timing |
|---|---|---|
| `gpt-transcribe` | transcription starts for a committed audio item and eventually completes it | Completed |
| `gpt-live-transcribe` | continuous transcript deltas followed by item completion | Completed or Live |

Every OpenAI recognition connection uses the Realtime transcription WebSocket.
One runtime generation selects exactly one model, and that selection is
immutable until Stop. A transient failure may replace the provider connection
under [ADR 0025](./0025-reconnect-within-one-runtime-generation.md), but the
runtime does not combine or switch models.

The general recognition Module owns a provider-independent active Interface:
continuous audio and lifecycle control enter; normalized ongoing or completed
full-text snapshots, lifecycle signals, and categorized errors leave. Provider
endpoint names, JSON events, commits, and identifiers do not cross this seam.
ADR 0026 later sharpened ownership of unitization, attempts, admission, and I/O
without changing the two selected OpenAI paths.

The OpenAI Module implements that Interface with behavior-specific Adapters.
It hides 24 kHz PCM append/commit operations, reconciles interleaved events by
`item_id`, converts raw deltas to monotonic full snapshots, and configures both
models with optional protocol language hints through `languages[]`. A committed
`gpt-transcribe` item emits no fabricated ongoing text; its completed event is
the only caption snapshot for that item. Input hints are not reported as
detected language: a caption receives a singular language label only when the
provider's completed event reports exactly one detected language.

The capability catalog and planner are backend-owned. They accept only the two
exact OpenAI model identifiers above and preserve an incompatible user request
while reporting alternatives. Legacy OpenAI model identifiers are rejected
explicitly; there is no compatibility route or silent migration. If an
existing settings file cannot be parsed as the current strict schema, the app
may load editable defaults but Start remains blocked until the user reviews
and saves the current settings.

The release has no REST/WAV recognition fallback and no production Mock
provider. Deterministic scripted Adapters remain test-only. A failure is
surfaced instead of changing model, timing, or provider; explicitly transient
failures may reconnect the same selection within the current generation.

Future local recognizers implement the same active Recognition Interface
through a local-worker driver. They do not emulate the OpenAI wire protocol
and never fall back to OpenAI silently.

## Consequences

- Completed/Live remains a publication policy selected from declared path
  capabilities, not a transport name.
- Provider event order is normalized before the caption store sees it. A
  completed item is emitted once; later deltas for that item are ignored, and
  Stop still rejects every event from the stopped generation.
- A committed item cannot block the ordered output queue forever. A bound item
  that remains incomplete for 30 seconds ends explicitly with a visible
  per-item diagnostic so later completed items can advance; a commit that was
  never assigned an `item_id` instead terminates the session and requires a
  clean reconnect because later identities cannot be attached safely.
- Reconnect never replays ambiguous audio. The old connection is retired,
  unconfirmed units end visibly, and capture resumes only after a fresh
  provider session is ready ([ADR 0025](./0025-reconnect-within-one-runtime-generation.md)).
- The WebSocket transport follows the selected system HTTP proxy without
  silently bypassing a failed or invalid proxy. Explicit environment proxy
  values, Windows protocol-mapped settings, and macOS manual proxy settings are
  parsed before any direct connection is permitted. Unsupported SOCKS,
  PAC/WPAD, or HTTPS-to-proxy transports fail visibly. The planned
  relay/base-URL option remains later work under
  [ADR 0019](./0019-follow-system-proxy-plan-relay-api.md).
- The bounded HTTP multipart/WAV implementation, its production command path,
  and its otherwise-unused direct HTTP/WAV dependencies are removed rather than
  retained as a fallback.
- Protocol-level tests cover normalization, ordering, bounded queues, Stop,
  proxy routing, and error mapping. An authenticated OpenAI adapter smoke has
  covered both release models, and the maintainer completed the native
  Windows/VRChat validation matrix with a real microphone on 2026-08-10.
- Adding another provider or local runtime requires a catalog entry and a
  concrete Adapter with explicit capabilities; it does not widen the OpenAI
  model field into an arbitrary string.
