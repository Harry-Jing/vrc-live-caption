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

## Support partial / stable / final Event Semantics

Decision: the architecture supports `partial`, `stable`, and `final`
transcript semantics.

Reason: MVP providers may only emit partial and final, but the product needs a
clean path for two-pass recognition, incoming caption, and future
interpretation.

Consequence: UI and output sinks should consume normalized transcript events
instead of provider raw messages.

Revisit if: provider behavior makes `stable` impossible to define consistently.

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
files or printed in logs.

Reason: STT and translation providers often require secrets, and diagnostics
must be safe to share.

Consequence: ordinary config can store non-sensitive settings only. Secret
storage and log redaction are product requirements.

Revisit if: a provider requires a credential flow that needs a separate secure
storage design.
