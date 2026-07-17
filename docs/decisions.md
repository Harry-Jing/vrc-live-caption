# Decisions

This is a lightweight decision log. It records accepted decisions only. Open
questions belong in [product.md](./product.md).

## Use Tauri, Vue, TypeScript, and Rust

Date: 2026-05

Decision: build the rewrite with Tauri 2, Vue 3, TypeScript, Vite, and a Rust
runtime.

Reason: the app needs a desktop shell, reliable Windows distribution, audio and
OSC runtime work, config, diagnostics, and future sidecar management without
requiring users to install Python tooling.

Consequence: the Python prototype is not ported directly.

Revisit if: Tauri or Rust blocks core audio, packaging, or runtime requirements.

## Treat The Python Prototype As Reference Only

Date: 2026-05

Decision: use the old Python prototype for behavior and testing lessons, not as
a new architecture constraint.

Reason: the prototype validated features, but its Python, Qt, asyncio, local
model, and sidecar boundaries are not the right long-term product base.

Consequence: new docs should extract principles instead of copying old module
boundaries or protocols.

Revisit if: a prototype behavior is required for user compatibility and needs a
formal compatibility contract.

## Make Outgoing Caption The MVP

Date: 2026-05

Decision: the MVP is microphone input to normalized App preview and paced
VRChat Chatbox output. The first implemented provider path uses completed-only
publication, while later paths may publish rolling text when their capabilities
and the user's mode allow it.

Reason: this is the smallest product path that validates the rewrite and gives
users a useful experience.

Consequence: incoming caption, local inference, TTS, virtual microphone output,
and persistent history are future capabilities. The MVP validates one simple
path without making that path's completion behavior a global runtime rule.

Revisit if: user validation shows incoming caption is more important than
outgoing caption for the first release.

## Expose Completed And Live Publication Modes

Date: 2026-07

Decision: Chatbox exposes two timing choices: Completed and Live. Completed
publishes real completed caption units only. Live may also publish ongoing
revisions. There is no public Automatic mode and no general soft-checkpoint
state. Provider path, publication mode, and source/translation/bilingual content
selection remain independent.

Reason: users understand the choice between waiting for a completed unit and
seeing revisable text. Automatic hides behavior changes when a model or endpoint
changes, while a soft checkpoint cannot make a provider's incomplete stream
complete. VRChat can replace visible text, so revisable providers remain useful
without inventing another completion category.

Consequence: the current segmented OpenAI path supports Completed. A streaming
path with ongoing and completed snapshots supports both. A continuous path with
no real per-unit completion supports Live only. An incompatible explicit choice
is explained with two directions: keep the model/provider and choose a
supported mode, or keep the requested experience and choose a compatible
model/provider. The App never silently changes model or mode. Bilingual Live
may let one lane progress ahead of the other.

The translation-only mapping is provisional until real translators are
benchmarked. A complete-result-only translator cannot update during speech;
whether token streaming that starts only after a completed source should be
presented as Live remains an explicit product test rather than a settled model
capability rule.

Revisit if: in-game testing shows that controlled Live replacement is unreadable
or if VRChat changes Chatbox replacement semantics.

## Pace Chatbox At One Second And Separate Live From Completed Backlog

Date: 2026-07

Decision: text-send attempts are separated by at least `1000 ms` from the last
actual attempt, including a failed attempt. The publisher does not consume the
initial leaky-bucket burst. Live keeps one latest-wins rolling viewport;
Completed uses an ordered bounded page queue.

Reason: real-client continuous-send tests showed skipped messages at 200, 250,
500, 800, and sustained 900 ms cadences, while 1000 ms delivered 120 numbered
messages without a skip. The result is consistent with a bucket that starts
with about five messages and replenishes about one message per second. A single
queue policy is wrong: queued Live revisions replay obsolete guesses, while
dropping completed pages normally loses real speech.

Consequence: on a path with real caption units, Live observes the unit's first
second; a short unit sends only its completion, while a longer unit begins
rolling at the newest snapshot. On an ongoing-only unitless path, the publisher
waits one second after the stream's first non-empty snapshot, then stays Live
without treating silence or a timer as completion. Completed pages and units
stay ordered. Only sustained exceptional overload may discard the oldest whole
units that have not begun publication; the App retains complete text and emits
a diagnostic. Exact queue limits remain measured parameters.

Revisit if: a future VRChat client changes observed delivery behavior. Re-run
the numbered-message test before reducing the interval.

## Render Bilingual Live As One Asynchronous View

Date: 2026-07

Decision: bilingual Chatbox output renders source above translation in one
message. The 144-character and nine-line budget is shared dynamically, both
lanes are visible once both have text, and remaining capacity modestly favors
translation. In Live, each send recomputes the newest useful view of both lanes;
source may lead translation and strict sentence alignment never blocks fresher
text.

Reason: a rigid half split wastes capacity, while replaying a late translation
as a separate old screen pulls a real-time conversation backward. Viewers can
tolerate translation lag more readily than stale source, but they assume a
displayed target is still valid.

Consequence: unit and source-revision identities preserve exact linkage inside
the App. Normal delay may leave the target one unit behind. If translation
explicitly fails, the bilingual selection remains configured, the App reports
degraded translation, and newer Chatbox snapshots omit stale target text until
translation is healthy again.

Revisit if: observer testing demonstrates that loose Live alignment is more
confusing than the added latency of an alignment-priority mode.

## Default To OpenAI For Cloud STT

Date: 2026-06

Decision: the first default STT provider is the OpenAI transcriptions API with
`gpt-4o-mini-transcribe`, uploading each completed speech segment as one
blocking request.

Reason: it validates the cloud-first MVP path with simple credential handling
and no streaming protocol work.

Consequence: the default provider emits completed source captions only; the App
preview shows listening state and completed text. Its bounded request now sits
behind a concrete recognition-session adapter and enters the backend-owned V1
caption-session aggregate, while its existing Completed Chatbox behavior stays
unchanged. Provider neutrality lives in the normalized contract, not in
avoiding a default. This adapter's completed-only behavior is not a constraint
on other provider paths. Cloud stays the MVP default; the long-term default
direction is local STT (see "Make Local STT The Long-Term Default").

Revisit if: per-segment latency or cost fails real usage, or a streaming
provider is added.

## Use A 30-Second Hard Maximum For The Bounded Cloud Path

Date: 2026-07

Decision: the current segmented OpenAI path closes normal utterances after the
existing `1.2`-second silence boundary and uses `30` seconds as the absolute
maximum for uninterrupted speech. The maximum is an internal adapter parameter,
not a user setting or a product-wide provider contract.

Reason: Phase 1 real-client testing found that the previous `12`-second maximum
split an approximately `20`-second thought into two ordered caption units even
though no speech was lost. Thirty seconds reduces premature mid-thought splits
while keeping audio, upload size, latency, and failure impact bounded.

Consequence: ordinary pauses still complete well before the hard maximum. A
continuous monologue may now wait up to thirty seconds plus recognition time for
its first Completed result, and a failed request can affect a larger unit.
Chatbox page capacity, including future bilingual layout, remains downstream
and does not define recognition boundaries.

Revisit if: real-machine latency approaches the request timeout, longer units
hurt recognition quality or recovery, or a future provider path owns different
natural or streaming boundaries.

## Normalize Full Ongoing And Completed Snapshots

Date: 2026-07

Decision: concrete adapters reconcile provider deltas into full caption
snapshots with a monotonic revision and one application state: ongoing or
completed. Snapshots identify their source or translation lane and their
session/stream correlation. They also identify a caption unit when the concrete
path has real units; an ongoing-only continuous path does not fabricate one.
Provider-specific stable-prefix behavior stays inside the adapter.

Reason: downstream consumers need the current text, its identity, and whether a
real unit closed. A general `stable` state is ambiguous, and forcing every
consumer to replay provider deltas duplicates fragile protocol logic.

Consequence: the V1 wire contract now carries explicit generation, stream
correlation, optional unit, lane, revision, full text, and ongoing/completed
fields in a backend-owned aggregate. The unused `stable` reservation and the
partial/stable/final wire ladder were removed. The bounded OpenAI adapter maps
its results to completed source captions, while one shared fixture and the
TypeScript runtime decoder pin the Rust/frontend wire shape. Future two-pass
work may add authority as a separate dimension; it must not overload
completion.

Revisit if: an implemented provider exposes information that cannot remain
inside its adapter and materially improves publication behavior.

## Name Diagnostic Codes `<category>.<detail>`

Date: 2026-06

Decision: every diagnostic event and serialized error carries a stable
machine-readable code shaped `<category>.<detail>`, where the prefix equals
the serialized diagnostic category. Error-to-category mapping is an exhaustive
match in code.

Reason: codes are the stable contract for filtering, tests, and future UI
localization, and the prefix rule is cheap to enforce while nothing consumes
codes yet.

Consequence: `message` and `detail` are English fallback text, not a contract.
Renaming a code becomes a breaking change once the frontend consumes codes.

Revisit if: a consumer needs structured diagnostic payloads beyond a flat code.

## Treat Event Delivery As Best-Effort

Date: 2026-06

Decision: runtime-to-UI events are at-most-once. Emit failures are logged and
never propagated, and the runtime lifecycle never depends on whether an event
reached the webview; the app stops the runtime explicitly on exit instead.

Reason: an emit only fails while the webview is being torn down, and no caller
can act on the failure. The capture-to-Chatbox pipeline must not die because
the view is gone.

Consequence: the UI must tolerate missed events and derive state from the
newest status and lifecycle events. Caption changes use a monotonic full
aggregate for both best-effort push and authoritative pull, so the frontend can
resynchronize after reload or a suspected gap and ignore an older copy.

Revisit if: an event appears whose loss corrupts UI state irrecoverably.

## Make Runtime Stop A Hard Cutoff

Date: 2026-06

Decision: stop releases the microphone within one receive timeout, discards
buffered and queued speech, cancels work where possible, and rejects every late
caption or translation result from the stopped generation for both App and
Chatbox. State-clearing signals are the one exception: stop may still send a
typing-indicator off message, but never caption text.

Reason: stop is a trust action and a state boundary. "Stop listening" must mean
nothing further is uploaded, displayed as new session text, or published.

Consequence: speech captured just before stop is lost by design and reported as
a diagnostic. An uncancellable in-flight request may finish during cleanup, but
its result is ignored rather than emitted to the App.

Revisit if: users ask for a stop mode that finishes the current utterance.

## Separate Saved Settings From Effective Runtime Sessions

Date: 2026-07

Decision: saved configuration is desired state for the next Start. Each Start
captures an immutable, generation-scoped selection of audio, recognition,
Chatbox, and provider-credential state. Saving runtime-bound settings during an
active session neither mutates that session nor restarts it automatically. Pure
UI preferences may apply immediately.

Rust exposes one revisioned, redacted control snapshot containing desired
configuration, runtime status, the active session selection, provider-secret
status, and derived pending-change categories. The frontend displays that
snapshot instead of inferring the active session from its editable config form.

Reason: a successful save previously made the UI look as if a running session
had changed even though Rust had already cloned its configuration and secret.
A local sticky restart flag could not survive reload, could not clear when the
user reverted a value, and could not account safely for credential changes.

Consequence: users can distinguish "saved for next Start" from "currently in
use." Runtime-bound changes are compared structurally with the active session;
reverting a non-secret setting clears the pending state. Credential identity is
represented only by redacted metadata and a process-local revision, so any
credential mutation remains pending without comparing plaintext. Commands that
mutate control state return the resulting authoritative snapshot, and missed
events can be repaired by pulling it again. Stop bypasses slow desired-state
I/O and invalidates any earlier Start that has not committed a runtime
generation; it is never queued behind config persistence or credential-store
access.

Revisit if: a future provider supports a specifically designed hot-reconfigure
operation. That must be an explicit capability and generation transition, not
an incidental side effect of Save.

## Identify Audio Devices By Stable Id

Date: 2026-06

Decision: config stores CPAL device ids, never display names.

Reason: duplicate device names and reconnects are common, especially on
Windows, so names are not stable identity.

Consequence: a saved but disconnected device stays selectable in the UI
instead of silently falling back to another microphone.

Revisit if: CPAL ids prove unstable across driver or OS updates.

## Keep Session History In Memory Only

Date: 2026-06

Decision: the MVP keeps bounded in-memory history only: recent completed caption
units and diagnostics for UI state. Nothing is persisted.

Reason: this covers preview and diagnosis without storage, retention, or
privacy design.

Consequence: history is lost on restart. Persistent searchable history stays a
Later capability.

Revisit if: users need post-session review or export.

## Keep Local Inference Isolated Behind Sidecars

Date: 2026-05

Decision: local STT and local translation run behind sidecars or workers, not
inside the main app process.

Reason: users should not need Python, PyTorch, CUDA Toolkit, or model-specific
development dependencies. Model crashes and GPU runtime failures should not
destabilize the main app.

Consequence: the main app stays small and keeps cloud paths independently
available. Local failure never uploads audio to cloud without explicit user
action. A running worker crash stops that recognition session and offers the
user explicit same-backend retry or backend change; it does not silently restart
on CPU or cloud.

Revisit if: a native local runtime becomes small and stable enough to include in
the main app without increasing install or support burden.

## Build Local STT One Pass At A Time And Let Users Choose The Backend

Date: 2026-07

Decision: the first local STT implementation is single-pass and loads one STT
model. CPU is implemented first as the compatibility path, followed by NVIDIA
CUDA in the same local-STT program after the worker boundary works. Local
compute uses one global preference:
CPU or prefer NVIDIA GPU (CUDA). No automatic performance selector is planned
now, and two-pass is deferred until the primary speech product is mature.

Reason: CPU is easiest to package for every Windows x64 machine, but neither CPU
nor GPU is universally best while VRChat is running. Utilization percentages do
not reveal main-thread, frame-time, memory-bandwidth, or VRAM contention. A
second recognizer also imposes resource cost that most users do not need.

Consequence: missing preference defaults to CPU. The App stores backend
preference separately from the effective backend and always displays both when
they differ. An unsupported CUDA/model combination uses CPU with a visible
reason; CUDA startup failure may use CPU only with a clear warning. A crash
during an active session never switches backend automatically. Model/backend
recommendations wait for real VRChat benchmarks rather than becoming universal
defaults from model marketing.

Revisit if: validated measurements justify an optional automatic selector or a
two-pass quality mode for a clearly identified user group.

## Keep Secrets Out Of Normal Config And Logs

Date: 2026-05

Decision: API keys, tokens, and credentials must not be stored in normal config
files or printed in logs. Provider keys live in the operating system credential
store, with an environment variable fallback for the OpenAI key.

Reason: STT and translation providers often require secrets, and diagnostics
must be safe to share.

Consequence: ordinary config stores non-sensitive settings only. The frontend
can save, delete, and inspect secret status, but can never read plaintext back.

Revisit if: a provider requires a credential flow that needs a separate secure
storage design.

## Localize The UI In The Frontend

Date: 2026-06

Decision: the app UI will be localized, starting with English and Chinese. The
backend never localizes: it emits stable codes plus English fallback text, and
the frontend owns all user-facing presentation. Effective immediately, new
user-facing text must be reachable from a stable code or key.

Reason: a large part of the target VRChat community is Chinese-speaking, and
retrofitting localization onto backend-generated display strings gets more
expensive with every new string.

Consequence: three language settings stay independent: caption language
(`stt.language`), UI locale, and the MVP-B translation target. Diagnostic
`message` and `detail` become debug fallback text once the UI maps codes. The
locale switch itself is scheduled separately from this groundwork rule.

Revisit if: a consumer outside the UI needs localized backend text.

## Define Desktop Platform Support Tiers

Date: 2026-07

Decision: Windows x86_64 is the Tier 1 and first public-release platform.
Windows 11 is the primary validation environment; Windows 10 22H2 remains in
Tier 1 while VRChat supports it. macOS arm64 and Linux x86_64 are runnable Tier
2 compatibility targets. Windows is the only platform that receives complete
real-machine end-to-end validation. Tier 2 coverage consists of CI compilation,
automated tests, and native package builds. Linux CI builds an x86_64 AppImage
on Ubuntu 22.04; this build baseline does not imply real-machine validation.

Reason: Windows is the project's only complete VRChat test environment. Keeping
macOS and Linux green catches portability and packaging regressions without
claiming validation the project cannot perform.

Consequence: Windows release readiness requires validating the current
microphone to segmented cloud STT to App preview to completed-only VRChat
Chatbox path on real hardware. Each later Live or translation path needs its own
real-machine validation. A Tier 2 compilation, test, or package failure blocks
merging. Platform-specific
Tier 2 runtime issues may be deferred unless they affect shared core behavior,
security, secrets, or data integrity. Tier 2 compatibility remains best-effort,
and its CI bundles are test artifacts rather than a public-release commitment.

Revisit if: repeatable real-machine validation becomes available for a Tier 2
platform, or distribution requirements change.

## Make Local STT The Long-Term Default

Date: 2026-06

Decision: local STT is the long-term default speech path. The MVP keeps cloud
STT as the default; the default switches to local only after a local engine is
validated on real Windows machines running VRChat.

Reason: the default path should not require an OpenAI account, payment setup,
or regional API access. The target community is global, with English and
Chinese as the first priorities, and cloud key acquisition is a hard barrier
for many of those users.

Consequence: cloud STT becomes a quality option instead of the only usable
path. Local STT moves from a Later idea to a planned roadmap phase that starts
with engine research: accuracy for English and Chinese, streaming versus
segmented input, model distribution, and resource usage measured while VRChat
is running. Model download and management become planned work instead of
non-goals.

Revisit if: no local engine reaches acceptable accuracy and resource usage on
machines that are also running VRChat.

## Signal Speech Activity With The Typing Indicator

Date: 2026-06
Updated: 2026-07

Decision: while normalized speech or publication activity is active, the app
sends the VRChat typing indicator on. It turns off when that activity is
resolved, on failure, and on runtime stop; it is not semantically tied to a
provider final.

Reason: the default bounded cloud provider leaves a gap before completed text,
while streaming paths may leave gaps between rolling publications. The typing
indicator is VRChat's native presence signal for both cases.

Consequence: real-client validation confirmed that VRChat hides an unrefreshed
indicator after about five seconds, so the publisher reasserts typing-on every
four seconds while activity remains active. These control-state packets do not
consume process-wide text pacing opportunities. Stop must send one clearing
typing-off message (the exception recorded in the stop decision). Provider
completion, Chatbox publication, and typing cleanup remain independently
testable.

Revisit if: in-game validation shows the indicator confuses or annoys other
players, or VRChat changes its semantics.

## Follow System Proxy Configuration For Cloud Requests

Date: 2026-07

Decision: cloud requests follow the operating system's manual proxy
configuration and report connection and timeout failures with a dedicated
network-unreachable diagnostic. Do not add an in-app proxy setting yet.

Reason: many target users need a Clash-style system proxy to reach OpenAI. The
system route supports that common setup without creating a second proxy
configuration surface inside the App.

Consequence: proxy settings are read when a runtime creates its HTTP client, so
users must stop and restart the runtime after changing them. The Windows path
covers current-user WinINet-style manual proxy settings; PAC/WPAD, WinHTTP,
machine policy, and uncommon per-protocol proxy formats are not assumed to work.

Revisit if: Windows validation or user reports show that system proxy support is
insufficient, especially for PAC, enterprise policy, or per-protocol setups.

## Keep Cloud Audio Disclosure In Settings

Date: 2026-07

Decision: show a persistent disclosure line in the cloud STT section of
Settings explaining that microphone audio is uploaded to OpenAI for
transcription.

Reason: this satisfies the product's cloud-upload disclosure requirement while
the maintainer prioritizes keeping the primary interface clean.

Consequence: a startup confirmation dialog and a disclosure on the main Live
page were considered and intentionally rejected. The disclosure requires no
confirmation action and remains visible whenever OpenAI cloud STT is selected.

Revisit if: users report that they did not understand audio was uploaded, or a
distribution channel imposes stricter compliance requirements.
