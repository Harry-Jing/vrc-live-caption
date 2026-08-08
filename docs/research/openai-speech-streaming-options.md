# OpenAI Realtime Transcription Options

Status: official protocol facts and authenticated adapter behavior rechecked
2026-08-07. The code cutover is implemented; Windows/VRChat validation remains.

## Scope and sources

This note records the OpenAI recognition facts used by the release design. It
does not cover ordinary file transcription, Realtime conversation output, or
Realtime translation.

Primary sources:

- [`gpt-transcribe` model](https://developers.openai.com/api/docs/models/gpt-transcribe)
- [`gpt-live-transcribe` model](https://developers.openai.com/api/docs/models/gpt-live-transcribe)
- [Realtime transcription](https://developers.openai.com/api/docs/guides/realtime-transcription)
- [Realtime WebSocket connection](https://developers.openai.com/api/docs/guides/realtime-websocket)
- [Realtime client events](https://developers.openai.com/api/reference/resources/realtime/client-events)

The model and protocol names below are intentionally exact. Older OpenAI
transcription model names in repository history describe the implementation
being replaced; they are not aliases for these paths.

OpenAI's current provider catalog also lists other transcription models. The
release catalog remains the two-model product decision in ADR 0024; a broader
provider catalog does not silently widen saved config or adapter behavior.

## Documented recognition behavior

Both selected models support Realtime transcription over a WebSocket. The
client appends 24 kHz PCM audio to the input buffer and commits audio items.
Transcript events identify their item with `item_id`, which is required to
correlate events when work on multiple items overlaps.

| Model | When recognition produces text | Transcript events | Language configuration |
|---|---|---|---|
| `gpt-transcribe` | after the client commits an audio item | the provider may emit post-commit deltas; the project intentionally publishes only the item's `conversation.item.input_audio_transcription.completed` result | optional `languages[]`; completed results can report detected `languages` |
| `gpt-live-transcribe` | continuously as audio arrives | `conversation.item.input_audio_transcription.delta` updates followed by `.completed` | optional `languages[]` hints |

`completed` contains the provider's completed transcript for that item. A
delta is not a normalized caption: the OpenAI Adapter must accumulate it and
emit the complete current text with a monotonic local revision. Completion of
one item says nothing about the completion state of another item.

For `gpt-transcribe`, commit is the start signal for recognition of the item.
Any provider delta therefore arrives after the application has already closed
the speech unit, not continuously while the user speaks. The project does not
market that as Live and intentionally suppresses those post-commit deltas while
waiting for `completed`. For `gpt-live-transcribe`, continuous deltas are real
ongoing recognition, so both publication timings are honest.

Language hints and detected language are different facts. The current product
requires at least one nonempty, unique hint and sends `languages[]` for either
model. It never copies a hint into caption metadata. A `gpt-transcribe`
completed result supplies the singular normalized `language` value only when
the provider reports exactly one nonempty detected language; no detection or
multiple detections remain unlabeled in the current V1 caption contract.

The concrete session setup uses
`wss://api.openai.com/v1/realtime?intent=transcription` with Bearer
authentication. The transcription model belongs only in
`session.audio.input.transcription.model`; it is not the top-level Realtime
session model. The session uses `type: "transcription"`, 24 kHz mono PCM, and
`turn_detection: null` so the application owns the unit boundary. Client event
IDs on session update and commit make provider errors diagnosable.

The general WebSocket guide documents authentication and handshake mechanics,
but its URL example creates a normal Realtime conversation session. The exact
transcription-intent route above was confirmed against the live API on
2026-08-07. The current OpenAI documentation conflicts about `delay` support:
the model-specific guide shows it for `gpt-live-transcribe`, while the general
client-event schema describes it as restricted to `gpt-realtime-whisper` in GA
Realtime sessions. The implementation therefore omits `delay` until a later
representative-audio evaluation resolves the effective product setting.

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

Deterministic protocol tests establish the Interface mapping. An authenticated,
repo-external harness then compiled the production transport, PCM encoder,
adapter, and `RecognitionSession` and ran them against the live API on
2026-08-07.

The first unmodified connection attempt exposed two release-blocking defects:

- the WebSocket URL supplied a transcription model as the top-level Realtime
  session model, which the provider rejected with `invalid_model`; and
- tungstenite enabled rustls without selecting a crypto provider, so TLS setup
  panicked before the handshake.

With the candidate transcription-intent route and rustls `ring` provider
enabled in the temporary harness, both release models passed Chinese, English,
and mixed-language samples from the normalized synthetic corpus:

| Model | Ongoing output | Completed output | Detected language |
|---|---:|---:|---|
| `gpt-transcribe` | 0 for all three samples | exactly 1 per sample | `zh`, `en`, `zh` |
| `gpt-live-transcribe` | 18, 17, and 16 revisions | exactly 1 per sample | none, as documented |

All six completed transcripts matched their expected text after normalization
for punctuation, case, and spacing. A separate empty-commit probe returned
`input_audio_buffer_commit_empty`; the production adapter mapped it to a
terminal STT error without leaking a later caption. The harness did not log the
Bearer token or audio payload, and it did not retain the temporary key after
the run.

This closes the paid wire-shape question for session update, 24 kHz PCM append,
commit, delta/completion normalization, language hints, detected language, and
one provider error. It does not establish recognition quality or latency for a
real microphone, Windows network behavior, proxy interception, long sessions,
disconnect/reconnect behavior, or the complete Tauri/VAD/Stop/OSC path. Before
Phase 4 exits, validate:

1. native system-proxy and TLS trust behavior on the Windows Tier 1 path;
2. short and long real-microphone speech in English, Chinese, and mixed language
   with VRChat active;
3. network interruption plus the chosen reconnect policy;
4. hard Stop while audio is buffered, committed, in flight, and pending; and
5. bounded buffering and explicit diagnostics under sustained backpressure.

The existing synthetic WAV corpus can remain a fixed recognition input fixture;
it does not need a TTS-quality benchmark or further voice-model evaluation.
Use a small representative subset for API smoke and retain the rest for manual
regression. It cannot replace one real microphone/room/VRChat session.

Measured timing belongs in a validation record, not in the capability catalog:
Completed and Live are semantic guarantees, while latency distributions are
operational evidence.
