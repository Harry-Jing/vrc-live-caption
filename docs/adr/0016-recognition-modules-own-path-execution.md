# Recognition Modules own path execution

Date: 2026-08

Runtime owns microphone capture, the runtime-generation output fence, and
publication. It submits continuous provider-independent audio to one active
Recognition Module. The Module owns the selected path's speech boundaries,
caption-unit lifecycle, replaceable attempts, reconnect behavior, protocol or
worker I/O, and normalization.

This boundary prevents OpenAI commits, item identifiers, WebSocket mechanics,
local worker messages, model windows, and backend state from leaking into the
runtime coordinator. Cloud and local Drivers can share normalized inputs and
signals without pretending that their internal lifecycles or budgets match.

Audio and normalized-signal admission stay bounded and non-blocking. Exhaustion
fails visibly; queued work never crosses a retired attempt or hard Stop boundary.

Increasing a command queue, silently dropping frames, replaying ambiguous
audio, and introducing a generic transport abstraction were rejected because
they preserve the wrong ownership or hide corrupted speech.
