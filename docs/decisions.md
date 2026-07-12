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

Decision: the MVP is microphone input to App preview to final-only VRChat
Chatbox output.

Reason: this is the smallest product path that validates the rewrite and gives
users a useful experience.

Consequence: incoming caption, local inference, TTS, virtual microphone output,
and persistent history are future capabilities.

Revisit if: user validation shows incoming caption is more important than
outgoing caption for the first release.

## Keep Chatbox Final-Only In The MVP

Date: 2026-05

Decision: VRChat Chatbox receives final text only in the MVP.

Reason: Chatbox cannot behave like a high-frequency real-time subtitle surface.
Partial output would be slow, noisy, and visible to other players before it is
stable.

Consequence: partial and stable transcript events are for App preview,
diagnostics, and future workflows, not MVP Chatbox output. Final-only applies
to transcript text: the typing indicator is a presence signal, not text, and
may run during an active utterance.

Revisit if: a later experiment proves stable or semi-final output improves UX
without causing flicker, spam, or incorrect public text.

## Default To OpenAI For Cloud STT

Date: 2026-06

Decision: the first default STT provider is the OpenAI transcriptions API with
`gpt-4o-mini-transcribe`, uploading each completed speech segment as one
blocking request.

Reason: it validates the cloud-first MVP path with simple credential handling
and no streaming protocol work.

Consequence: the default provider emits final transcripts only; the App
preview shows listening state and final text. Provider neutrality lives in the
normalized event contract, not in avoiding a default. Cloud stays the MVP
default; the long-term default direction is local STT (see "Make Local STT The
Long-Term Default").

Revisit if: per-segment latency or cost fails real usage, or a streaming
provider is added.

## Support partial / stable / final Event Semantics

Date: 2026-05

Decision: the architecture supports `partial`, `stable`, and `final`
transcript semantics.

Reason: MVP providers may only emit partial and final, but the product needs a
clean path for two-pass recognition, incoming caption, and future
interpretation.

Consequence: UI and output sinks should consume normalized transcript events
instead of provider raw messages.

Revisit if: provider behavior makes `stable` impossible to define consistently.

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
newest status and lifecycle events.

Revisit if: an event appears whose loss corrupts UI state irrecoverably.

## Make Runtime Stop A Hard Cutoff

Date: 2026-06

Decision: stop releases the microphone within one receive timeout, discards
buffered and queued speech, and sends no Chatbox output after the stop
request; only an STT request already in flight is awaited. State-clearing
signals are the one exception: stop may still send a typing-indicator off
message so other players are not left with a stuck indicator, but never
transcript text.

Reason: stop is a trust action. "Stop listening" must mean nothing further is
uploaded or published.

Consequence: speech captured just before stop is lost by design and reported
as a diagnostic.

Revisit if: users ask for a stop mode that finishes the current utterance.

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

Decision: the MVP keeps bounded in-memory history only: recent final
transcripts and diagnostics for UI state. Nothing is persisted.

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

Consequence: the main app stays small and keeps a working cloud path even when
local inference is missing or broken.

Revisit if: a native local runtime becomes small and stable enough to include in
the main app without increasing install or support burden.

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

## Target Windows As The First Release Platform

Date: 2026-06

Decision: the first public release targets Windows.

Reason: VRChat has no macOS client, so audio devices, OSC, and real VRChat
sessions can only be validated on Windows. Current macOS development is a
temporary convenience, not a release target.

Consequence: Windows CI builds and real-machine VRChat validation move ahead of
release work instead of waiting for the release phase.

Revisit if: VRChat ships on another desktop platform that the user base
actually plays on.

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

Decision: while an utterance is active, the app sends the VRChat typing
indicator on; the indicator turns off when final text is sent, when the
utterance ends without a final, and on runtime stop.

Reason: the default cloud provider emits final-only transcripts, so other
players would otherwise see nothing between the start of speech and the final
text. The typing indicator is VRChat's native affordance for exactly this gap
and masks recognition latency at almost no cost.

Consequence: stop must send one clearing typing-off message (the exception
recorded in the stop decision). Final-only continues to apply to transcript
text; the indicator is a presence signal.

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
