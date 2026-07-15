# OpenAI Speech Streaming Options

Research snapshot for `VRC Live Caption`, verified against first-party OpenAI
documentation on 2026-07-15. This note separates published API behavior from
project recommendations. OpenAI can add models, fields, and event types without
changing this file, so re-check the linked references before implementation.

## Decision Summary

The project should not impose one global `final-only` or one global rolling
policy on every speech provider. The provider contract and the output policy
are separate decisions:

- `gpt-4o-transcribe` and `gpt-4o-mini-transcribe` on
  `/v1/audio/transcriptions` consume a completed file or an application-bounded
  audio segment. Optional SSE streams the transcription result after that audio
  has been uploaded; it does not turn the endpoint into a continuous microphone
  transport.
- `gpt-realtime-whisper` is the current documented path for low-latency live
  transcription. It emits partial transcript deltas and an explicit completed
  transcript for each committed item.
- `gpt-realtime-translate` continuously emits append-only translated transcript
  deltas and can optionally emit source transcript deltas when input
  transcription is configured. The current Translation event reference does
  not define a per-utterance transcript `done` or `completed` event. It can
  therefore expose ongoing Live snapshots, but the project must not fabricate a
  completed unit.
- Future local final-only and streaming engines map onto the same ongoing /
  completed snapshot contract. Two-pass is deferred pipeline orchestration, not
  a provider capability or current requirement.

## Terminology

These terms are project vocabulary, not all OpenAI API field names.

- **Delta / partial**: a provider-wire text fragment or revision. It remains
  inside the adapter, which exposes a full normalized snapshot.
- **Append-only**: each delta is appended exactly as received; previously
  emitted characters are not withdrawn by a later delta in that stream. This
  says nothing about whether the phrase is semantically complete or correct.
- **Provider final**: an explicit provider event such as
  `transcript.text.done` or
  `conversation.item.input_audio_transcription.completed`. An adapter may map it
  to a completed caption unit when it closes that path's real unit.
- **Ongoing snapshot**: the adapter's full current text for an unclosed caption
  unit or continuous lane. It may be useful in Live but is not completion.
- **Completed snapshot**: the adapter's full final text for one real caption
  unit. The project never creates it from a timer or quiet period alone.
- **Session terminal**: the whole stream has been drained and closed. For
  Realtime Translation, `session.closed` is session-wide termination, not an
  utterance boundary.

## Endpoint Comparison

| Path | Documented model | Audio input | Text events | Explicit item final | Recommended project policy |
|---|---|---|---|---|---|
| `/v1/audio/transcriptions` | `gpt-4o-transcribe`, `gpt-4o-mini-transcribe` | Completed file or app-bounded segment uploaded in one request | One complete response, or SSE `transcript.text.delta` then `transcript.text.done` | Yes, for the request | Completed, or show SSE in the App and publish the request final |
| Realtime transcription session | `gpt-realtime-whisper` | WebSocket/WebRTC live audio buffer; manual commit for this model | `conversation.item.input_audio_transcription.delta` then `.completed` | Yes, for the committed item | Rolling App preview; Chatbox can choose Live (rolling, then final) or Completed |
| Realtime transcription with `gpt-4o-transcribe` / mini | Public documentation is internally inconsistent; see below | Do not assume until a protocol spike verifies the current GA session | Do not freeze a contract until verified | Do not assume until verified | Keep behind capability detection or an experimental flag |
| `/v1/realtime/translations` | `gpt-realtime-translate` | Continuous WebSocket/WebRTC audio, including silence | Append-only `session.output_transcript.delta`; optional append-only `session.input_transcript.delta` | No per-utterance final documented | Experimental Live ongoing snapshots; no fabricated Completed support |

OpenAI's current overview makes the high-level distinction explicit: request
APIs are for files or bounded requests, while Realtime sessions are for live
audio and low-latency events. See [Realtime and audio](https://developers.openai.com/api/docs/guides/realtime#understand-different-architectures).

## `/v1/audio/transcriptions`

### Published behavior

- The Speech-to-Text guide describes this as a file-upload and bounded-request
  API. It supports `gpt-4o-transcribe` and `gpt-4o-mini-transcribe`; the upload
  limit is currently 25 MB. See [Speech to text](https://developers.openai.com/api/docs/guides/speech-to-text).
- With `stream=true`, an already completed recording can return SSE
  `transcript.text.delta` events followed by `transcript.text.done`, whose event
  includes the full transcript. OpenAI directs ongoing microphone, call, or
  media audio to the Realtime transcription path instead. See
  [Streaming transcriptions](https://developers.openai.com/api/docs/guides/speech-to-text#streaming-the-transcription-of-a-completed-audio-recording).
- The request can include a prompt for vocabulary and context with the 4o
  transcription models. See
  [Prompting](https://developers.openai.com/api/docs/guides/speech-to-text#prompting).

### Project interpretation

For a microphone application, the app must first create a bounded segment by
local VAD, push-to-talk, or another segmenter and then upload it. `stream=true`
can reduce *result delivery* latency after upload, but it cannot reveal words
while the still-open audio segment is being recorded.

The provider gives a hard final for each request. That makes this endpoint a
straightforward Completed path or later completed-text translation source. If
SSE deltas are exposed in the App, they should still be associated with the
request ID and replaced by
`transcript.text.done`.

## Realtime Transcription

### Current documented path: `gpt-realtime-whisper`

The current guide says Realtime transcription sessions stream deltas from live
audio and recommends `gpt-realtime-whisper` for the lowest-latency path. A
WebSocket fits a native/server audio pipeline; WebRTC fits browser capture. For
PCM input, the example uses 24 kHz mono PCM. See
[Realtime transcription](https://developers.openai.com/api/docs/guides/realtime-transcription).

For `gpt-realtime-whisper`, the guide documents these controls and events:

- latency/accuracy levels `minimal`, `low`, `medium`, `high`, and `xhigh`;
- `turn_detection` must be omitted or `null`, so the client commits the audio
  buffer manually;
- a delta arrives as
  `conversation.item.input_audio_transcription.delta`;
- a committed item's provider final arrives as
  `conversation.item.input_audio_transcription.completed` and contains the full
  `transcript`;
- completion ordering across different speech turns is not guaranteed, so
  reconcile them with `item_id`.

See [session fields](https://developers.openai.com/api/docs/guides/realtime-transcription#session-fields)
and [transcript events](https://developers.openai.com/api/docs/guides/realtime-transcription#handle-transcript-events).

The guide says sessions stream deltas as audio arrives, but it also instructs
clients with disabled turn detection to commit when they want transcription to
begin. The public text does not state a precise guarantee for whether useful
`gpt-realtime-whisper` deltas can arrive before the first manual commit. Treat
pre-commit delta timing as an empirical question and record the event trace in
the protocol spike.

### VAD and commit are boundaries, not output policies

OpenAI's VAD guide defines `server_vad` as silence-based automatic chunking and
`semantic_vad` as model-inferred utterance completion, for sessions and models
that support VAD. It emits `input_audio_buffer.speech_started` and
`input_audio_buffer.speech_stopped`. See
[Voice activity detection](https://developers.openai.com/api/docs/guides/realtime-vad#overview).

A provider VAD boundary or a manual commit creates an item boundary. The App can
still show partials before completion, and an output sink can independently
choose Live (rolling snapshots followed by the completed item) or Completed. A
commit is therefore not a reason to hard-code every sink as final-only.

### Documentation conflict for the 4o transcription models

The current public sources do not agree completely:

- the [`gpt-4o-transcribe` model page](https://developers.openai.com/api/docs/models/gpt-4o-transcribe)
  and [`gpt-4o-mini-transcribe` model page](https://developers.openai.com/api/docs/models/gpt-4o-mini-transcribe)
  list both `transcription` and `realtime` as supported endpoints;
- the current Realtime Transcription guide describes
  `gpt-4o-transcribe` as a higher-accuracy choice where streaming is not
  required, and configures `gpt-realtime-whisper` for live streaming;
- the current data-residency endpoint/model table lists only
  `gpt-realtime-whisper` for `/v1/realtime/transcription_sessions`, while it
  lists the 4o transcription models under the Audio API. See
  [API endpoint and model support](https://developers.openai.com/api/docs/guides/your-data#api-endpoint-tool-and-model-support).

The data-residency table is supporting evidence in its own scope, not a general
compatibility matrix. The model pages and Realtime Transcription guide are the
primary source of the implementation uncertainty.

This conflict is material. Before implementing a 4o model inside a GA
transcription-only session, run a small authenticated protocol test against the
actual endpoint and record:

1. whether session creation accepts the model;
2. which GA session shape it accepts;
3. whether it supports server VAD, manual commit, or both;
4. whether deltas arrive before or only after commit;
5. the exact completion and usage events.

Until that passes, the reliable documented mapping is 4o transcription models
on `/v1/audio/transcriptions` and `gpt-realtime-whisper` for dedicated live
transcription.

## Realtime Translation

### Published behavior

Realtime Translation is a separate continuous interpreter architecture on
`/v1/realtime/translations` with `gpt-realtime-translate`. It does not use the
normal assistant response lifecycle and the client does not call
`response.create`. With WebSockets, the client appends 24 kHz PCM16 audio
continuously, including silence between phrases, and receives translated audio
plus transcript deltas. See
[Realtime translation](https://developers.openai.com/api/docs/guides/realtime-translation#how-translation-sessions-differ)
and [the WebSocket flow](https://developers.openai.com/api/docs/guides/realtime-translation#create-a-websocket-session).

The current event reference defines:

- `session.output_transcript.delta` for translated text;
- optional `session.input_transcript.delta` for source-language text when
  `audio.input.transcription` is configured;
- both transcript streams as append-only fragments;
- optional `elapsed_ms` as stream-alignment metadata derived from translation
  frames. It advances in 200 ms increments when available, and multiple deltas
  can have the same value, so it is not a unique delta ID.

See the
[output transcript event](https://developers.openai.com/api/reference/resources/realtime/translation-server-events#session.output_transcript.delta)
and
[input transcript event](https://developers.openai.com/api/reference/resources/realtime/translation-server-events#session.input_transcript.delta).
The official Translation cookbook currently recommends
`gpt-realtime-whisper` for the optional source transcript. See
[Realtime Translation key differences](https://developers.openai.com/cookbook/examples/voice_solutions/realtime_translation_guide#key-differences).

The current Translation server-event reference documents transcript delta
events but no per-utterance transcript `done`, `completed`, or final event. It
does document `session.closed`, which applies to the whole session. At the end
of a WebSocket source stream, `session.close` tells the service to flush pending
input and emit the remaining output; the client must continue reading until
`session.closed` before closing the socket. See
[closing a Translation session](https://developers.openai.com/api/docs/guides/realtime-translation#close-a-websocket-session).

### What append-only does and does not guarantee

Append-only gives a useful character-level property: the app can concatenate
the deltas exactly as received without replacing an earlier prefix. It does not
prove that a phrase is complete. For example, a stable prefix can still be a
sentence fragment whose meaning depends on later audio.

`elapsed_ms` also cannot be used as a final signal. It is for alignment, can be
missing or shared by multiple deltas, and has no documented relationship to an
utterance-final event.

### Provisional project interpretation: honest ongoing output

Because there is no provider utterance final, a Translation adapter should
expose the append-only stream honestly. The adapter mapping below is a safe
research recommendation; whether users should see every such path as public
Live behavior remains provisional until in-game testing. Recommended adapter
behavior is:

1. append every source and target delta to the normalized stream state;
2. publish the newest normalized snapshot toward the App immediately, while
   retaining best-effort UI delivery semantics;
3. let output sinks coalesce to their own safe cadence and keep only the latest
   snapshot rather than queueing every intermediate version;
4. expose the target as ongoing for Live and do not advertise Completed for
   this path while no real item completion exists;
5. on graceful source end, use `session.close`, then wait for `session.closed`
   and publish any drained tail.

OpenAI does not publish a fixed "silence for N milliseconds means translation
is complete" guarantee. Network delay or a temporarily slow model can look like
output quiet, so quiet time alone must not be represented as completion. If a
future product needs application-owned units for this continuous path, that
behavior requires a separate design and protocol test rather than a general
soft-checkpoint state.

Runtime Stop is different from graceful source end. Stop may close or cancel
the connection for cleanup, but the application's hard-cutoff rule discards
every drained tail and other late caption event instead of publishing it.

## Normalized Capability Model

Provider adapters should expose semantics, not force a sink policy. Capabilities
belong to the full provider path (provider, endpoint or session mode, model, and
relevant configuration), not the model name alone. A minimal capability
description needs to distinguish at least:

| Capability | Example values | Why it matters |
|---|---|---|
| Input shape | completed segment, committed items, continuous stream | Determines capture, buffering, reconnect, and backpressure behavior |
| Update cadence | final-only, streaming | Determines whether a Live mode has intermediate text to publish |
| Streaming revision contract | revisable snapshot, append-only | Determines how the adapter reconciles raw provider updates |
| Item completion | provider final available, no item final | Determines whether Completed is supportable on that path |
| Produced lanes | source only, target only, both | Supports source, translated, and bilingual content modes |

Input capabilities normally apply to the whole provider path. Output cadence,
revision, and item-completion capabilities are recorded per lane when source
and target differ. A bilingual policy cannot infer that both lanes are complete
merely because one lane has an item final. Provider-specific prefix guarantees
may remain inside the adapter.

Two-pass is future pipeline orchestration, not an intrinsic model capability or
a `supports_two_pass` flag.

Provider delta and replacement operations stay inside the adapter. A normalized
caption update carries the complete current text snapshot, a monotonic revision,
caption-unit identity when one exists, source or translation lane, and ongoing
or completed state. Session termination is a separate lifecycle event and never
stands in for an item completion.

An output sink then selects a policy from capabilities plus user preference:

| Provider behavior or topology | Useful output policy |
|---|---|
| No incremental text plus a real item completion | Completed |
| Ongoing revisions plus a real item completion | Live (rolling followed by completion) or Completed |
| Append-only stream without item completion | Live latest-wins only |

### VRChat boundary

This capability model is independent of VRChat. OpenAI events do not define
OSC rate limits, Chatbox display time, message length, or replacement cadence.
Those are output-sink concerns. A snapshot is not made complete by being sent
to or displayed by the Chatbox.

## Required Protocol Spikes

Before promoting any Realtime path from experimental to default, capture raw
timestamped event traces for representative microphones, languages, accents,
code-switching, background noise, long speech, and brief pauses. At minimum:

1. verify the 4o Realtime transcription model/session compatibility conflict
   described above;
2. for `gpt-realtime-whisper`, measure first-delta timing relative to append and
   commit at every delay level;
3. confirm how the completed transcript differs from collected partials and
   reconcile by `item_id`;
4. for Translation, measure local speech-end to last related target delta and
   build p50/p95/p99 drain-delay distributions;
5. measure punctuation, local speech-end, and output-quiet timing without
   treating any heuristic as provider completion;
6. test source and target lanes independently, including target-only and
   bilingual rendering;
7. verify that `session.close` drains the final translated tail before
   `session.closed`;
8. test reconnect behavior without treating a disconnected, unflushed stream
   as final.

Do not turn measured timings into a claimed OpenAI protocol guarantee. Store
them as tunable project parameters with diagnostics and conservative behavior.
