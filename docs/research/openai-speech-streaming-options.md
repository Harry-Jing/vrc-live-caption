# OpenAI Realtime Transcription Options

Status: official protocol facts rechecked 2026-08-07. The code cutover is
implemented; authenticated OpenAI and Windows/VRChat validation remain.

## Scope and sources

This note records the OpenAI recognition facts used by the release design. It
does not cover ordinary file transcription, Realtime conversation output, or
Realtime translation.

Primary sources:

- [`gpt-transcribe` model](https://developers.openai.com/api/docs/models/gpt-transcribe)
- [`gpt-live-transcribe` model](https://developers.openai.com/api/docs/models/gpt-live-transcribe)
- [Realtime transcription](https://developers.openai.com/api/docs/guides/realtime-transcription)
- [Realtime WebSocket connection](https://developers.openai.com/api/docs/guides/realtime-websocket)

The model and protocol names below are intentionally exact. Older OpenAI
transcription model names in repository history describe the implementation
being replaced; they are not aliases for these paths.

## Documented recognition behavior

Both selected models support Realtime transcription over a WebSocket. The
client appends 24 kHz PCM audio to the input buffer and commits audio items.
Transcript events identify their item with `item_id`, which is required to
correlate events when work on multiple items overlaps.

| Model | When recognition produces text | Transcript events | Language configuration |
|---|---|---|---|
| `gpt-transcribe` | after the client commits an audio item | the project consumes the item's `conversation.item.input_audio_transcription.completed` result | optional `languages[]`; completed results can report detected `languages` |
| `gpt-live-transcribe` | continuously as audio arrives | `conversation.item.input_audio_transcription.delta` updates followed by `.completed` | optional `languages[]` hints |

`completed` contains the provider's completed transcript for that item. A
delta is not a normalized caption: the OpenAI Adapter must accumulate it and
emit the complete current text with a monotonic local revision. Completion of
one item says nothing about the completion state of another item.

For `gpt-transcribe`, commit is the start signal for recognition of the item.
The project therefore does not market it as Live and does not synthesize
ongoing captions while waiting for `completed`. For `gpt-live-transcribe`,
continuous deltas are real ongoing recognition, so both publication timings
are honest.

Language hints and detected language are different facts. The current product
requires at least one nonempty, unique hint and sends `languages[]` for either
model. It never copies a hint into caption metadata. A `gpt-transcribe`
completed result supplies the singular normalized `language` value only when
the provider reports exactly one nonempty detected language; no detection or
multiple detections remain unlabeled in the current V1 caption contract.

The concrete session setup uses
`wss://api.openai.com/v1/realtime?model=<exact-model>` with Bearer
authentication, `type: "transcription"`, 24 kHz mono PCM, and
`turn_detection: null` so the application owns the unit boundary. Client event
IDs on session update and commit make provider errors diagnosable. The current
OpenAI documentation conflicts about `delay` support: the model-specific guide
shows it for `gpt-live-transcribe`, while the general client-event schema still
describes it as restricted to an older model. The implementation therefore
omits `delay` until an authenticated smoke resolves the effective behavior.

## Release capability mapping

The backend catalog, not UI string matching, owns this mapping:

| Catalog path | Input shape | Normalized update behavior | Completed | Live |
|---|---|---|---|---|
| `openai/gpt-transcribe` | 24 kHz PCM append plus committed items | completed snapshot only | yes | no |
| `openai/gpt-live-transcribe` | continuous 24 kHz PCM append with item completion | ongoing full snapshots plus completed snapshot | yes | yes |

The selected model is immutable for a running session. These are alternative
paths, not two passes of one request. If the user requests Live with
`gpt-transcribe`, planning fails with explicit compatible alternatives while
preserving the user's selection.

## Adapter mapping

The OpenAI Module hides the provider wire protocol behind the general
`RecognitionSession` Interface:

1. convert provider-independent captured audio to the required 24 kHz PCM;
2. serialize buffer append and commit commands in protocol order;
3. associate each accepted or committed `item_id` with one stable local unit;
4. route every delta and completion to that unit even when events for different
   items interleave;
5. turn raw deltas into full-text ongoing snapshots with increasing revisions;
6. emit at most one completed snapshot per unit and reject later updates for
   that completed unit;
7. represent an item that produces no caption or fails recoverably with an
   explicit unit-ended event so later units cannot be misattributed; and
8. map authentication, connection, protocol, rate-limit, and provider errors to
   categorized application errors without changing model or publication mode.

Provider `item_id` values remain internal reconciliation keys. The caption
store, UI contract, publishers, and future local worker never depend on them.
Stop closes or abandons the connection as needed for cleanup, but the runtime
generation gate rejects every resulting late event.

## Options not carried forward

- **Bounded REST/WAV recognition:** the production Adapter and direct
  dependencies are removed. It is not a release fallback.
- **Legacy OpenAI transcription models:** saved identifiers are rejected as
  unsupported and require an explicit user selection; there is no alias,
  migration, or compatibility Adapter.
- **Two simultaneous OpenAI models:** one session owns one model. Two-pass
  recognition remains separate future orchestration.
- **Production Mock provider:** deterministic scripts remain test fixtures, not
  a catalog entry or runtime fallback.
- **Provider-agnostic WebSocket abstraction:** WebSocket setup and framing are
  OpenAI Module Implementation. The reusable seam is the semantic
  `RecognitionSession`, which can also admit a local-worker Adapter.

## Dependency implications

The WebSocket/TLS, Base64, and proxy libraries are transport dependencies
private to the OpenAI Module. The transport selects the OS/system HTTP proxy,
uses an HTTP CONNECT tunnel when selected, never forwards the OpenAI Bearer
token to that proxy, and never silently falls back to a direct connection after
a selected proxy fails. Connection, CONNECT, WebSocket frame, write-buffer,
protocol-item, event, and transcript-memory bounds are explicit.

The former direct `reqwest` multipart and `hound` WAV dependencies are removed
after the repository-wide usage audit. A transitive HTTP library used by Tauri
is not a recognition fallback. The custom relay/base-URL option remains later
work under [ADR 0019](../adr/0019-follow-system-proxy-plan-relay-api.md).

## Phase 4 validation

Deterministic protocol tests establish the Interface mapping; they do not
establish paid-service behavior, recognition quality, latency, or Windows
network behavior. Before Phase 4 exits, run a small authenticated smoke for
both models and verify:

1. session configuration, 24 kHz PCM append, and commit ordering;
2. first-delta and completion latency for short, long, English, Chinese, and
   mixed-language speech;
3. `languages[]`, detected-language, and code-switching behavior without
   treating input hints as detection;
4. interleaved and out-of-order events correlated by `item_id` without unit or
   publication-order corruption;
5. empty items, item errors, authentication failure, rate limits, disconnects,
   and reconnect policy;
6. bounded buffering and explicit diagnostics under network backpressure;
7. hard Stop while audio is buffered, an item is committed, a delta is in
   flight, and a completion is pending; and
8. system-proxy behavior on the Windows Tier 1 path.

The existing synthetic WAV corpus can remain a fixed recognition input fixture;
it does not need a TTS-quality benchmark or further voice-model evaluation.
Use a small representative subset for API smoke and retain the rest for manual
regression. It cannot replace one real microphone/room/VRChat session.

Measured timing belongs in a validation record, not in the capability catalog:
Completed and Live are semantic guarantees, while latency distributions are
operational evidence.
