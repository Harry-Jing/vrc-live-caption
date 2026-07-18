# Product

## Positioning

VRC Live Caption is a local desktop tool for real-time speech understanding,
caption preview, translation, and output routing. The first usable path targets
VRChat, but the product does not make VRChat Chatbox the center of the runtime.

The product is designed for long always-on sessions: users start it once and
keep it running while they play. English and Chinese are the first language
priorities. The implemented baseline is cloud STT; the long-term direction is a
validated local STT path that does not require an account or per-minute payment.

## Current Scope

The implemented MVP-A baseline is:

```text
microphone
  -> application-bounded speech segment
  -> gpt-4o-mini-transcribe
  -> App preview
  -> completed VRChat Chatbox output
```

This is one working provider path, not a product-wide final-only contract.

MVP-A includes:

- microphone selection and capture;
- cloud STT baseline;
- App caption preview;
- completed output for the current segmented provider path;
- basic settings and diagnostics;
- paced, length-limited Chatbox output.

The planned MVP-B prioritizes the smallest text-driven translation path:

- completed normalized source text into one concrete translator;
- source-only, translation-only, and bilingual content where supported;
- translation health, timeout, and diagnostic behavior;
- Chatbox publication resolved from provider capabilities and user choice.

Direct-audio Realtime Translation remains a separate research candidate. It is
not required to complete MVP-B and should be promoted only after its continuous
stream semantics and user experience are validated.

Current OpenAI endpoint behavior is documented in
[research/openai-speech-streaming-options.md](./research/openai-speech-streaming-options.md).

The following User Choices, User Scenarios, and Requirements describe accepted
target product behavior, not current implementation status. The implementation
gates are tracked in [roadmap.md](./roadmap.md).

## User Choices

### Publication timing

The product exposes two timing choices:

- **Completed / 停顿后发送**: publish only completed caption units. Long speech
  is still bounded by natural-boundary-first segmentation plus a hard maximum,
  so one uninterrupted monologue cannot suppress output forever.
- **Live / 实时更新**: allow ongoing revisions when the selected content lane
  produces them. The App previews immediately. A unit-based path lets Chatbox
  observe the first second, sends only a completed short utterance, and starts
  rolling only while the unit remains active. An ongoing-only unitless path
  instead waits one second after its first non-empty stream snapshot and then
  stays Live without inventing completion.

There is no public Automatic mode. If the selected model, endpoint, content,
and publication mode are incompatible, the App explains why and offers
two explicit directions: keep the model/provider and choose a supported mode,
or keep the requested experience and choose a compatible model/provider. It
never silently swaps model or mode.

Live describes a publication policy, not a promise that every provider begins
text at the same moment. A native simultaneous translator may update during
speech; an LLM may start streaming target text only after a source segment is
complete; a final-only translator has no ongoing target revisions to publish.
Provider-specific timing must be described honestly in the UI.

### Content

Content choice is independent of publication timing:

- source only;
- translation only;
- bilingual.

For bilingual Chatbox output, source appears above translation. The renderer
shares the 144-character and nine-line budget dynamically, guarantees both
lanes visible once both contain text, and gives remaining capacity a modest
default preference toward translation. It does not reserve a rigid 50/50 split.

### Local compute backend

Local inference uses one global preference:

- CPU;
- prefer NVIDIA GPU (CUDA).

Missing configuration defaults to CPU. The preference is not an automatic
performance decision. The App also shows the effective backend for the current
model/session. Unsupported GPU combinations use CPU with a visible reason;
runtime failures never silently switch backend or cloud provider.

The global preference is preserved when the current model lacks CUDA support.
That model's GPU capability is shown unavailable rather than rewriting the
preference; switching later to a supported model can use the same preference.
On a machine with no compatible NVIDIA GPU, the CUDA choice is unavailable; a
preference imported from another machine resolves to CPU with an explanation.

## User Scenarios

### Source-only Live

A streaming recognizer produces full revisable snapshots. The App updates as
soon as useful text exists. Chatbox sends at most one current view per second,
keeps the newest text visible, and skips obsolete revisions. A final correction
is sent only when it differs from the published view.

A final-only recognizer cannot provide this experience. The App offers either
Completed or a compatible streaming recognizer.

### Source-only Completed

The provider or application closes a real caption unit, then Chatbox publishes
the completed text. Text that exceeds one Chatbox view is paginated in order;
it is not truncated to the first or last page.

### Bilingual Live

Source and target progress independently in one rolling Chatbox view. Source
may lead translation. Every send recomputes the newest useful source context
above the newest available target context instead of replaying an old target
completion as a separate screen.

Strict sentence-by-sentence alignment does not block fresher source text. The
App retains unit/revision linkage so source and target remain correctly paired
in application state even when the constrained Chatbox viewport shows
different progress points.

Normal translation delay may leave the target one unit behind. If translation
explicitly fails, the user's bilingual choice stays selected, the App displays
a clear degraded state, and newer Chatbox messages temporarily omit the stale
target. When translation becomes healthy again, bilingual output resumes.

### Translation-only Live (provisional)

The public compatibility rule for translation-only Live remains provisional
until concrete local and cloud translators are benchmarked with users. A
translator that returns only one complete result cannot update during speech.
A translator that streams tokens after a completed source could publish newest
target snapshots at the safe Chatbox cadence, but the UI must say that it begins
after a pause and testing must determine whether users consider that Live.

The first implementation does not repeatedly submit unstable ASR partials to a
normal text translator to simulate Live. That behavior creates visible rewrites,
request amplification, cost, and out-of-order races.

### Long uninterrupted speech

A recognition path that owns caption units prefers a natural boundary but has
an application hard limit. After a target duration it looks for a brief low-
energy or linguistic boundary; at the absolute maximum it closes a real caption
unit and immediately continues with a new one. An ongoing-only continuous path
still needs bounded audio, memory, reconnect, and backpressure behavior, but it
must not call a timer boundary a completed caption unit. Exact durations are
benchmark parameters, not ordinary user settings.

### Stop

Stop is a hard trust action. It releases capture, discards queued work, blocks
all App and Chatbox text from the stopped generation, and ignores late provider
or translation results. A typing-off cleanup message is the only allowed output
after Stop.

## Chatbox Experience

- All text-send attempts are separated by at least `1000 ms` from the previous
  actual attempt; the publisher does not exploit burst capacity.
- Live is a latest-wins rolling viewport. It retains recent context when space
  permits but always keeps the newest content.
- Completed uses an ordered, bounded page queue. Normal operation attempts to
  show all completed pages; sustained overload may drop only whole oldest units
  that have not started publication.
- Every message obeys the 144-character input cap, nine visible lines, real
  glyph-width wrapping, and grapheme-safe boundaries.
- The App retains the complete normalized text even when the constrained
  Chatbox viewport drops an obsolete Live revision or an overloaded completed
  unit.

Detailed constraints and the current-client pacing experiment live in
[research/vrchat-chatbox-reference.md](./research/vrchat-chatbox-reference.md).

## Local Inference Direction

Local STT uses an isolated Rust worker and packaged native runtime, with no
Python, PyTorch, or Conda requirement. The first implementation order is an
engineering sequence, not a product ranking:

1. SenseVoiceSmall on CPU to validate the bounded local worker path;
2. the same path on NVIDIA CUDA for comparable packaging and performance tests;
3. streaming Paraformer and streaming Zipformer for Live behavior;
4. real VRChat benchmarks before recommending a model/backend combination.

Only one STT model is active in a normal session. Two-pass is a very low-
priority future experiment and is not a normal setting, model requirement, or
automatic behavior. Local translation may add a separate translation model;
its resource cost must be disclosed and measured rather than assumed small.

Candidate and backend notes live in
[research/local-inference-notes.md](./research/local-inference-notes.md).

## Requirements

MUST:

- The MVP must support outgoing caption from microphone input.
- Provider raw events must be normalized before UI or output consumers see
  them.
- Every provider path must declare enough per-lane capability information to
  validate Completed and Live; the App must never invent completion.
- Chatbox output must be paced, coalesced, length-limited, and layout-aware.
- Live revisions must be latest-wins rather than queued as stale screens.
- Completed pages must be ordered and bounded without arbitrary middle-page
  loss.
- Capture, provider processing, and translation must never block on Chatbox
  pacing.
- Stop must reject all late caption text from the stopped generation.
- Silent provider, model, backend, mode, content, or cloud fallback is
  prohibited.
- API keys and secrets must not enter ordinary config or logs.
- The App must disclose when microphone audio is uploaded to a cloud provider.

SHOULD:

- The App should show ongoing text whenever the selected path produces it.
- Incompatible choices should remain visible with an explanation and explicit
  alternatives instead of disappearing without context.
- If direct-audio translation is added, it and transcript-driven translation
  should remain separate provider paths.
- Diagnostics should separate audio, provider, translation, worker, backend,
  OSC, config, and network failures.
- Settings should show both backend preference and the effective backend.
- Ordinary config should carry a schema version for migrations.
- Diagnostics should be exportable as a redacted report.
- The first cloud path and first public release should work without requiring a
  local model download.
- The App UI should be localizable; English and Chinese are first.

MAY:

- Later versions may support incoming caption, persistent history, export,
  interpretation, TTS, virtual microphone output, and two-pass recognition.
- Local translation may be added after the primary speech and model-management
  paths are stable.

## Non-Goals

The current MVP does not include:

- unpaced forwarding of provider deltas;
- system audio capture or speaker diarization;
- local model download and management;
- local STT or local translation;
- two-pass recognition;
- automatic CPU/GPU performance selection;
- automatic cloud fallback;
- TTS, virtual microphone, plugin system, mobile support, or persistent
  searchable history.

Local STT and component management are planned work rather than open-ended
ideas. Two-pass remains Later even after local single-pass STT lands.

## Open Questions And Measured Parameters

Resolved product choices belong in [docs/adr/](./adr/). The remaining
items require implementation evidence rather than more abstract modes:

- benchmark the natural-boundary search point and hard maximum for long speech;
- set completed-queue page and age limits from real speech and translation load;
- compare first useful text, speech-end completion, CPU/RAM/GPU/VRAM, and VRChat
  frame-time effects for each model/backend pair;
- re-evaluate the provisional treatment of token-streamed translation after
  concrete local and cloud translators are integrated;
- decide whether later translation remains cloud-first after a small local
  translation benchmark;
- decide whether users need manual approval before publishing a completed unit;
  keep automatic Completed publication as the baseline unless testing shows a
  strong need;
- set end-to-end latency targets separately for first useful Live text and
  speech-end completion instead of choosing one number for every provider;
- choose the local model distribution shape: installer-bundled, first-run
  download, or managed component catalog;
- compare global hotkeys, auto-start with VRChat, and a later OVR overlay for
  headset-friendly start, stop, and error visibility without putting technical
  diagnostics into public Chatbox messages;
- measure local-sender and remote-observer display duration, then choose whether
  Completed pages need a text-length-scaled minimum hold, adjacent-unit merging,
  a longer fixed interval, or only the measured one-second safety floor.
