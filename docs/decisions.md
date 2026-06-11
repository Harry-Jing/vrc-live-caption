# Decisions

This is a lightweight decision log. It records accepted decisions only. Open
questions belong in [product.md](./product.md).

## Use Tauri, Vue, TypeScript, and Rust

Decision: build the rewrite with Tauri 2, Vue 3, TypeScript, Vite, and a Rust
runtime.

Reason: the app needs a desktop shell, reliable Windows distribution, audio and
OSC runtime work, config, diagnostics, and future sidecar management without
requiring users to install Python tooling.

Consequence: the Python prototype is not ported directly.

Revisit if: Tauri or Rust blocks core audio, packaging, or runtime requirements.

## Treat The Python Prototype As Reference Only

Decision: use the old Python prototype for behavior and testing lessons, not as
a new architecture constraint.

Reason: the prototype validated features, but its Python, Qt, asyncio, local
model, and sidecar boundaries are not the right long-term product base.

Consequence: new docs should extract principles instead of copying old module
boundaries or protocols.

Revisit if: a prototype behavior is required for user compatibility and needs a
formal compatibility contract.

## Make Outgoing Caption The MVP

Decision: the MVP is microphone input to App preview to final-only VRChat
Chatbox output.

Reason: this is the smallest product path that validates the rewrite and gives
users a useful experience.

Consequence: incoming caption, local inference, TTS, virtual microphone output,
and persistent history are future capabilities.

Revisit if: user validation shows incoming caption is more important than
outgoing caption for the first release.

## Keep Chatbox Final-Only In The MVP

Decision: VRChat Chatbox receives final text only in the MVP.

Reason: Chatbox cannot behave like a high-frequency real-time subtitle surface.
Partial output would be slow, noisy, and visible to other players before it is
stable.

Consequence: partial and stable transcript events are for App preview,
diagnostics, and future workflows, not MVP Chatbox output.

Revisit if: a later experiment proves stable or semi-final output improves UX
without causing flicker, spam, or incorrect public text.

## Default To OpenAI For Cloud STT

Decision: the first default STT provider is the OpenAI transcriptions API with
`gpt-4o-mini-transcribe`, uploading each completed speech segment as one
blocking request.

Reason: it validates the cloud-first MVP path with simple credential handling
and no streaming protocol work.

Consequence: the default provider emits final transcripts only; the App
preview shows listening state and final text. Provider neutrality lives in the
normalized event contract, not in avoiding a default.

Revisit if: per-segment latency or cost fails real usage, or a streaming
provider is added.

## Support partial / stable / final Event Semantics

Decision: the architecture supports `partial`, `stable`, and `final`
transcript semantics.

Reason: MVP providers may only emit partial and final, but the product needs a
clean path for two-pass recognition, incoming caption, and future
interpretation.

Consequence: UI and output sinks should consume normalized transcript events
instead of provider raw messages.

Revisit if: provider behavior makes `stable` impossible to define consistently.

## Name Diagnostic Codes `<category>.<detail>`

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

Decision: stop releases the microphone within one receive timeout, discards
buffered and queued speech, and sends no Chatbox output after the stop
request; only an STT request already in flight is awaited.

Reason: stop is a trust action. "Stop listening" must mean nothing further is
uploaded or published.

Consequence: speech captured just before stop is lost by design and reported
as a diagnostic.

Revisit if: users ask for a stop mode that finishes the current utterance.

## Identify Audio Devices By Stable Id

Decision: config stores CPAL device ids, never display names.

Reason: duplicate device names and reconnects are common, especially on
Windows, so names are not stable identity.

Consequence: a saved but disconnected device stays selectable in the UI
instead of silently falling back to another microphone.

Revisit if: CPAL ids prove unstable across driver or OS updates.

## Keep Session History In Memory Only

Decision: the MVP keeps bounded in-memory history only: recent final
transcripts and diagnostics for UI state. Nothing is persisted.

Reason: this covers preview and diagnosis without storage, retention, or
privacy design.

Consequence: history is lost on restart. Persistent searchable history stays a
Later capability.

Revisit if: users need post-session review or export.

## Keep Local Inference Optional And Isolated

Decision: local STT and local translation are future optional capabilities that
run behind sidecars or workers.

Reason: users should not need Python, PyTorch, CUDA Toolkit, or model-specific
development dependencies. Model crashes and GPU runtime failures should not
destabilize the main app.

Consequence: the main app remains small and cloud-capable by default.

Revisit if: a native local runtime becomes small and stable enough to include in
the main app without increasing install or support burden.

## Keep Secrets Out Of Normal Config And Logs

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
