# Product

## Vision

VRC Live Caption is a desktop voice tool for VRChat: the user speaks, and other
players read the words as captions, a translation, or both. It began as a way to
talk across languages, but same-language accessibility and clarity are equally
important.

The intended experience is a polished, always-on companion: start captioning,
put the headset on, and keep playing. English and Chinese are the first language
priorities. Windows is the primary platform.

Cloud recognition establishes the product experience first. The long-term
default becomes local only after a local path is validated for accuracy,
latency, stability, and resource use while VRChat is running
([ADR 0003](./adr/0003-local-inference-is-the-long-term-default.md)).

## Product principles

- **User choices stay explicit.** The app explains incompatible combinations;
  it does not silently change a recognition path, translation path, publication
  mode, content selection, or local backend.
- **Stop means stop.** Stop discards current and queued work rather than
  finishing the current utterance; no later caption reaches the app or Chatbox
  ([ADR 0010](./adr/0010-stop-is-a-hard-cutoff.md)).
- **Failures are visible.** Recovery may discard uncertain speech, but it must
  not duplicate, mis-correlate, or secretly reroute it.
- **Cloud use is honest.** Settings disclose when microphone audio or completed
  Source text is uploaded and which selected path receives it. Credentials stay
  out of ordinary config, logs, and diagnostics.
- **VRChat constraints shape output.** Pacing, layout, and pagination follow the
  measured Chatbox behavior in the
  [VRChat reference](./research/vrchat-chatbox-reference.md).

## Target user choices

This section defines the product's stable choice model, not current feature
availability. The [roadmap](./roadmap.md) remains authoritative for what is
implemented.

### Recognition path

A user selects one recognition path for the next Start. Saved changes do not
mutate a running generation. A removed or unsupported path remains visible as a
choice that needs attention; the app does not silently migrate it.

The supported catalog is closed and capability-driven rather than accepting
arbitrary model strings ([ADR 0016](./adr/0016-use-openai-realtime-transcription.md)).

### Publication timing

- **Completed / 停顿后发送** publishes a caption after its source unit completes.
- **Live / 实时更新** may also replace the visible Chatbox text while the user is
  speaking, when the selected path produces honest ongoing revisions.

Model capability and publication timing are independent choices. When they are
incompatible, the app offers explicit alternatives instead of selecting one
([ADR 0006](./adr/0006-publication-timing-is-completed-or-live.md)).

### Content

The content choices are Source-only, Translation-only, and Bilingual output.
Bilingual output places Source and Translation in one asynchronous view, with
space leaning toward Translation rather than a fixed 50/50 split
([ADR 0007](./adr/0007-bilingual-output-is-one-asynchronous-view.md)).

Content that includes Translation requires an explicit target. UI locale,
recognition hints, and detected Source language never infer it. The first
catalog is English (`en`) and Simplified Chinese (`zh-Hans`).

### Local backend

Local inference will use an explicit CPU or prefer-NVIDIA-CUDA preference. The
app will show the effective backend and the reason whenever it differs from
that preference ([ADR 0019](./adr/0019-users-choose-the-local-backend.md)).

## Target behavior by scenario

### Source-only Live

The app shows useful ongoing text and Chatbox publishes a latest-wins view. It
may skip obsolete intermediate revisions rather than replaying old guesses.

### Source-only Completed

Completed speech is published in order. Text that exceeds one Chatbox view is
paginated without truncating the beginning or end.

### Bilingual output

When Translation succeeds, Completed Chatbox publication pairs it with its exact
Source and renders Source above Translation. Across pages, each selected lane's
text appears losslessly once; an exhausted shorter lane is not repeated, and
the longer lane may continue alone.

On terminal Translation failure, the App shows the failed unit and degraded
state. Translation-only omits it; Bilingual publishes Source as a partial
result, stays selected, and tries Translation again for later units. Live
alignment remains a separate decision.

### Long speech

A unit-based path prefers natural speech boundaries and also applies a hard
internal bound, then continues immediately in a new unit. Exact timing is a
path-specific benchmark parameter, not a user setting.

### Temporary outage

A retryable recognition outage may reconnect within the same runtime generation
without replaying ambiguous audio or changing the selected path. Speech near the
outage may be lost, but not duplicated ([ADR 0017](./adr/0017-reconnect-within-one-runtime-generation.md)).

## User-facing requirements

- A local microphone probe never contacts a recognition or translation service.
- The UI is localizable; caption language, UI locale, and translation target are
  independent choices.
- Caption and diagnostic history stays bounded and in memory unless a future
  persistence design explicitly changes that privacy boundary
  ([ADR 0005](./adr/0005-keep-caption-history-in-memory-only.md)).

## Scope

The first public release is an outgoing-caption product: microphone speech to
VRChat Chatbox. Incoming captions do not gate it.

System-audio capture, speaker diarization, TTS, virtual microphone output,
plugins, mobile support, and persistent searchable history are outside the
first-release scope. Incoming captions, local recognition, local translation,
and other later work remain ordered in the [roadmap](./roadmap.md).
