# Product

## Positioning

VRC Live Caption is a local desktop tool for real-time speech understanding,
caption preview, translation, and output routing. The first usable product path
is focused on VRChat, but the product should not treat VRChat Chatbox as the
center of the whole system.

The product is designed for long always-on sessions: users start it once and
keep it running while they play, rather than starting and stopping it around
every conversation.

The target community is global, with English and Chinese as the first
priorities. The long-term default speech path is local STT; cloud STT is the
MVP default and later a quality option (see [decisions.md](./decisions.md)).

## MVP Scope

The MVP is Outgoing Caption: the user speaks into a microphone, the App shows a
caption preview, and final text is sent to VRChat Chatbox.

MVP-A:

- microphone input
- cloud STT
- App caption preview
- final-only Chatbox output
- basic settings
- basic diagnostics

MVP-B:

- final-only translation
- target-language or bilingual Chatbox rendering
- translation timeout and fallback behavior
- settings polish for translation

## User Scenarios

### Outgoing Caption

The user speaks, sees a preview in the App, and sends stable text to VRChat
Chatbox so other players can read it.

MVP behavior:

- App preview may show partial and final text.
- Chatbox receives final text only.
- Chatbox output is paced, length-limited, and shaped for VRChat constraints.

### Outgoing Translation

The user speaks one language and sends translated text to Chatbox after the
source transcript is final.

MVP-B behavior:

- Translation uses final transcripts by default.
- Translation must not block audio capture or STT.
- If translation fails, the App should keep the source transcript visible and
  report the failure clearly.

### Incoming Caption

The App may later capture system or VRChat audio and show captions for other
speakers inside the App.

This is not part of the MVP.

### Local Inference

The App may later support local STT or local translation without requiring the
user to install Python, PyTorch, or CUDA Toolkit.

This is not part of the MVP.

## Requirements

MUST:

- The MVP must support Outgoing Caption from microphone input.
- The MVP must send only final text to VRChat Chatbox.
- Chatbox output must be paced and length-limited.
- API keys and secrets must not be stored in normal config files or logs.
- The App must clearly disclose when microphone audio is uploaded to a cloud
  provider.
- Provider raw events must be normalized before they reach UI-facing runtime
  consumers.

SHOULD:

- The App should show partial transcript preview when the selected provider
  supports it.
- Translation should use final transcript text by default.
- Diagnostics should separate audio, STT, translation, OSC, config, network, and
  local worker failure areas.
- The default path should work without local model downloads.
- The App UI should be localizable; English and Chinese are the first targets.

MAY:

- Later versions may support Incoming Caption.
- Later versions will add local STT (the planned long-term default) and may
  support local translation.
- Later versions may support history, export, interpretation, TTS, or virtual
  microphone output.

## Non-Goals

The MVP does not include:

- Chatbox partial streaming
- system audio capture
- speaker diarization
- local model download and management
- local STT
- local translation
- TTS
- virtual microphone output
- plugin system
- mobile support
- full caption history and search

Local STT and its model management are planned post-MVP work rather than
open-ended ideas (see [decisions.md](./decisions.md)); the rest are
unscheduled.

## Open Questions

Resolved questions move to [decisions.md](./decisions.md).

- Should MVP-B support target-only Chatbox output before bilingual output?
  Recommendation: start with target-only output, then add bilingual rendering if
  the renderer and UX stay simple.

- Should users be able to manually approve final text before Chatbox output?
  Recommendation: keep automatic final output as the default MVP path, and treat
  manual approval as an optional mode if it does not slow the core path.

- What is the end-to-end latency target from end of speech to Chatbox text?
  Decide after the local STT engine direction is set, because a streaming local
  engine changes the latency profile.

- Does the local-first default direction extend to translation in MVP-B, or
  does translation stay cloud-first?

- How do VR users, who cannot see the desktop App while wearing the headset,
  start, stop, and monitor captioning? Candidates include OVR overlays, global
  hotkeys, auto-start with VRChat, and status feedback through the Chatbox
  itself.
