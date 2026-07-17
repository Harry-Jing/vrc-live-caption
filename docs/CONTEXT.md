# VRC Live Caption Context

VRC Live Caption is a local desktop tool for real-time speech understanding,
caption preview, translation, and output routing for VRChat and desktop voice
communication. These terms are the shared language for discussing provider
behavior without tying the product to one model or endpoint.

The rewrite is not a direct port of the Python prototype. The old prototype is
useful for behavior, testing, and product lessons, but it does not define the
new architecture.

## Current Direction

- App shell: Tauri 2.
- Frontend: Vue 3, TypeScript, and Vite.
- Runtime: Rust.
- Implemented baseline: microphone input to application-bounded OpenAI cloud
  STT behind a concrete bounded recognition-session adapter, backend-owned
  `CaptionSessionSnapshotV1` state, App preview, and completed-only VRChat
  Chatbox output.
- Implemented Phase 3 contract slice: full caption-session aggregates use
  backend-authoritative generation and stream identity, push plus pull
  resynchronization, and one runtime-decoded Rust/TypeScript V1 wire shape.
  The current OpenAI path still emits completed source captions only.
- Target publication choices: Completed and Live, resolved from the selected
  provider path and content lanes rather than one global final-only rule.
- First translation implementation: completed normalized source text into one
  translator; direct-audio and translation-only Live remain separate measured
  candidates.
- Long-term default speech path: validated single-pass local STT behind an
  isolated Rust worker. Two-pass remains a low-priority later experiment.
- First public release and complete real-machine validation: Windows x86_64.

## Where The Rules Live

Each document has a different authority:

- Product scope, requirements, user scenarios, and open questions:
  [product.md](./product.md)
- Current and target runtime boundaries, event semantics, and data flow:
  [architecture.md](./architecture.md)
- Accepted decisions, including defaults, security, and platform choices:
  [decisions.md](./decisions.md)
- Ordered implementation phases and exit criteria:
  [roadmap.md](./roadmap.md)
- Factual research and measured behavior:
  [research/](./research/)

## Language

**Provider path**:
A concrete combination of provider, endpoint or session mode, model, runtime,
backend, and relevant configuration whose behavior is evaluated together.
_Avoid_: Model, when endpoint- or runtime-dependent behavior is intended

**Caption unit**:
An application-correlated span of speech and text. A concrete adapter decides
how the unit ends: local VAD, provider endpointing, an application hard limit,
or another boundary supported by that path.
_Avoid_: Sentence, because a forced boundary may not be a grammatical sentence

**Caption lane**:
An ordered sequence of normalized source or translated text snapshots.
_Avoid_: Transcript, when the lane contains translated text

**Caption snapshot**:
The complete current text for one caption lane at one revision. It carries a
caption-unit id when that path has real units; an ongoing-only continuous path
instead remains correlated to its session/stream and must not invent a unit
completion. Raw provider deltas are accumulated or reconciled inside the
adapter before a snapshot reaches the App or an output sink.
_Avoid_: Delta, outside a provider adapter

**Ongoing snapshot**:
A revisable caption snapshot that has not completed. On a unit-based path, its
unit is still open. On an ongoing-only continuous path, it remains attached to
the session/stream without implying that a unit exists. Earlier text may be
replaced unless the concrete provider path documents a stronger rule.
_Avoid_: Stable, provisional final, soft final

**Completed snapshot**:
The adapter's final normalized text for one caption unit. Completion can come
from a provider item final or from a real application-owned segment boundary;
it is never inferred from a display timer.
_Avoid_: Provider final, unless referring to the provider's exact event

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
The rule that combines the selected provider path's per-lane capabilities,
content selection, publication mode, and the constraints of one output sink.
_Avoid_: Provider output mode

**Backend preference**:
The user's global local-inference preference: CPU or prefer NVIDIA GPU (CUDA).
It is stored separately from the effective backend used by a concrete session.
_Avoid_: Auto backend, guaranteed GPU execution

**Effective backend**:
The backend actually used by one local-inference session. If the preferred
backend is unavailable, the App keeps the preference, exposes the reason, and
never hides the effective backend.
_Avoid_: Fallback, without also reporting the reason

**Two-pass pipeline**:
Optional future orchestration that runs a low-latency recognizer and a separate
correction recognizer over correlated audio. It is not a speech-model mode and
is not part of the first local implementation.
_Avoid_: Two-pass model, stable mode

## Documentation Rules

Authoritative project docs are in English. Chinese notes use the `.zh-CN.md`
suffix. Accepted choices belong in `decisions.md`; unsettled product behavior
stays in `product.md` or a research note instead of being written as an
implemented contract.
