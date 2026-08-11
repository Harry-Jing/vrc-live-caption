# VRC Live Caption Context

VRC Live Caption turns speech into VRChat Chatbox captions and translations.
These terms are the shared language for discussing provider behavior without
tying the product to one model or endpoint.

## Language

**Speech recognition**:
The application subsystem that turns microphone audio into normalized source
caption snapshots. STT remains acceptable user shorthand and the stable
`stt.*` diagnostic namespace, but code-domain names use Recognition.
_Avoid_: STT, in new code-domain type and field names

**Recognition path**:
A concrete recognizer execution choice whose provider, model, protocol mode,
runtime, backend, and relevant configuration are evaluated together.
_Avoid_: Provider path, model when protocol- or runtime-dependent behavior is
intended

**Recognition Module**:
The generation-scoped application boundary that accepts continuous owned
audio, enforces bounded admission and hard Stop, and emits ordered normalized
signals. It delegates path-specific execution to one Recognition Driver.
_Avoid_: Provider session, recognition worker

**Recognition Driver**:
The concrete executor for one recognition path. It owns speech boundaries,
replaceable attempts, provider protocol or local-worker I/O, and normalization
without leaking those mechanisms into Runtime.
_Avoid_: Adapter, when the component owns execution and lifecycle rather than
only data conversion

**Translation path**:
A concrete translator execution choice that consumes an exact completed source
snapshot and produces correlated translation-lane snapshots.
_Avoid_: Recognition path, translation model when provider or protocol behavior
is intended

**Service provider**:
An external service identity, such as OpenAI, that may supply more than one
recognition or translation path and may share one credential across them.
_Avoid_: Recognition provider, when credential ownership or another service
capability is intended

**Service credential**:
An authentication identity for one service provider. Recognition and
translation paths may deliberately share it; local paths may require none.
_Avoid_: STT key, recognition credential when the identity is service-wide

**Runtime generation**:
One user-started captioning lifetime, ending at its hard Stop boundary. It may
contain more than one provider connection, but late output from an older
connection can never re-enter the generation after that connection is retired.
_Avoid_: Provider session or connection attempt, when the Start-to-Stop
lifetime is intended

**Recognition attempt**:
One replaceable execution lifetime inside a runtime generation, backed by one
cloud connection or one local worker session. Retiring an attempt discards its
unconfirmed audio and output without ending the runtime generation.
_Avoid_: Runtime generation, provider session

**Caption unit**:
An application-correlated span of speech and its source and translation lanes.
A recognition driver decides when source input ends: local VAD, provider
endpointing, an application hard limit, or another boundary supported by that
path. A closed source unit does not imply that correlated translation work has
settled.
_Avoid_: Sentence, because a forced boundary may not be a grammatical sentence

**Caption stream**:
The ordered application correlation scope inside one runtime generation. It can
remain stable across replacement recognition attempts and is not a provider
connection, WebSocket stream, or model streaming state.
_Avoid_: Provider stream, session

**Caption lane**:
An ordered sequence of normalized source or translated text snapshots.
_Avoid_: Transcript, when the lane contains translated text

**Caption snapshot**:
The complete current text for one caption lane at one revision. It carries a
caption-unit id when that path has real units; an ongoing-only continuous path
instead remains correlated to its caption stream and must not invent a unit
completion. Raw provider deltas are accumulated or reconciled inside the
driver before a snapshot reaches the App or an output sink.
_Avoid_: Delta, outside a provider transport or driver

**Caption Aggregate**:
The application-owned, revisioned view of the active caption stream, its open
Source units, and bounded recent completed captions. An open Source unit means
recognition is still active; it does not describe pending Translation work.
The aggregate may retain history from older runtime generations, so it is not
a provider or runtime session.
_Avoid_: Caption session, transcript history

**Ongoing snapshot**:
A revisable caption snapshot that has not completed. On a unit-based path, its
unit is still open. On an ongoing-only continuous path, it remains attached to
the caption stream without implying that a unit exists. Earlier text may be
replaced unless the concrete recognition path documents a stronger rule.
_Avoid_: Stable, provisional final, soft final

**Completed snapshot**:
The final normalized text for one caption lane's revision chain. Source
completion can come from a provider item final or a real application-owned
segment boundary; it is never inferred from a display timer and does not make
another lane terminal.
_Avoid_: Provider final, unless referring to the provider's exact event

**Source snapshot reference**:
The exact generation, caption stream, caption unit, and source revision consumed
by a translation snapshot. Translation never attaches by timing or display
position alone.
_Avoid_: Latest source, current caption

**Chatbox publication mode**:
The user's timing choice: **Completed** publishes completed caption units only;
**Live** may also publish ongoing revisions. This is independent of provider,
model, and source/translation/bilingual content choice.
_Avoid_: Automatic, translation mode, streaming toggle

**Content selection**:
The lanes the user wants to publish: source only, translation only, or
bilingual. Source and translation may progress at different speeds.
_Avoid_: Publication mode

**Publication policy**:
The rule that combines the active caption pipeline's per-lane capabilities,
content selection, publication mode, and the constraints of one output sink.
_Avoid_: Provider output mode

**Caption Pipeline Plan**:
The application-resolved compatibility result for the selected recognition
and translation paths, requested content lanes, publication mode, and output
constraints. It records incompatibility rather than silently changing a user
selection.
_Avoid_: Backend plan, provider capability plan

**Application gateway (`AppGateway`)**:
The frontend's primary typed boundary to caption-runtime, settings, audio, and
OSC host capabilities. Tauri and Preview are concrete adapters behind it;
narrow UI-only services such as confirmation or diagnostic-report clipboard
access may keep feature-specific host ports. The term backend remains reserved
for local-inference compute such as CPU or CUDA.
_Avoid_: Runtime backend, when referring to frontend-to-host IPC

**Backend preference**:
The user's global local-inference preference: CPU or prefer NVIDIA GPU (CUDA).
It is stored separately from the effective backend used by a concrete attempt.
_Avoid_: Auto backend, guaranteed GPU execution

**Effective backend**:
The compute backend actually used by one local-inference attempt. If the
preferred backend is unavailable, the App keeps the preference, exposes the
reason, and never hides the effective backend.
_Avoid_: Application gateway, fallback without also reporting the reason

**Two-pass pipeline**:
Optional future orchestration that runs a low-latency recognizer and a separate
correction recognizer over correlated audio. It is not a speech-model mode and
is not part of the first local implementation.
_Avoid_: Two-pass model, stable mode
