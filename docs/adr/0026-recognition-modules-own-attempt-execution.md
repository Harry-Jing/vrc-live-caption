# Recognition Modules own attempt execution

Date: 2026-08

Status: accepted and implemented for the OpenAI path

## Context

The first Realtime implementation let runtime divide microphone callbacks into
speech units and enqueue provider commands while a separate worker alternated
one command with a synchronous WebSocket read. A nominal two-millisecond read
timeout blocks for roughly 10–20 milliseconds on Windows Winsock, so ordinary
capture outpaced the callback-count queue and stopped with `stt.backpressure`.
Batching commands reduces that symptom but leaves runtime coupled to OpenAI
commit semantics and gives a future local model the wrong Interface.

## Decision

Runtime owns microphone capture, the runtime-generation output fence, and
publication. It starts one active **Recognition Module** and submits continuous,
owned mono audio frames. The Module owns path-specific speech boundaries,
caption-unit lifecycle, recognition attempts, reconnect/backoff, protocol or
worker I/O, and normalization. Its external Interface is start, bounded
non-blocking audio submission, ordered normalized signals, reconnect capture
acknowledgement, and out-of-band hard Stop. Provider commands, JSON, commits,
item IDs, local IPC messages, and model-native frame shapes remain internal.

Audio admission is bounded by represented audio duration plus a frame-count
safety ceiling, not by capture callback count. Every admitted frame carries an
attempt epoch and an RAII budget permit. Reconnect closes admission, advances
the epoch, discards queued audio, asks runtime to drop capture, and waits for an
exact matching acknowledgement before any fresh attempt can accept audio. A
frame racing with retirement is rejected or discarded by epoch and can never
cross into the new attempt. Stop closes admission out of band, wakes connect,
ack, backoff, protocol, or worker waits, clears pending signals, and joins the
owner.

Normalized signals keep total order. Revisions of the same ongoing caption may
coalesce latest-wins, while lifecycle control has reserved bounded capacity;
completed captions and unit boundaries are never silently reclassified as an
ongoing update. The OpenAI Network Owner drives an established TLS/WebSocket
connection without blocking capture: reads, writes, control frames, partial
records, and pending output make independent bounded progress, and Stop can
shut down the socket directly.

A later local STT path implements the same active Interface with an
out-of-process driver. That driver may choose different audio-duration and IPC
budgets and owns model loading, resampling/windowing, inference cadence, and
worker health. Runtime does not gain local-model branches or provider lifecycle
commands. Translation remains a downstream Module over normalized source
snapshots in Phase 5 and is not folded into recognition.

## Considered Options

- Increasing the old queue or draining commands in batches was rejected: both
  encode a platform-dependent timing workaround and retain a shallow,
  OpenAI-shaped runtime seam.
- An unbounded queue or silent frame dropping was rejected: either permits
  unbounded latency/memory or corrupts captions without a trustworthy gap.
- Replaying queued audio after reconnect was rejected because the application
  cannot prove what the retired provider accepted.
- A generic WebSocket or generic local-worker transport was rejected. Each
  concrete Module owns its protocol; only continuous audio and normalized
  signals are shared.

## Consequences

- Windows callback bursts and socket scheduling no longer define recognition
  throughput through a fake read timeout.
- OpenAI and future local recognition share one deep application Interface
  without pretending their internal attempts, boundaries, or budgets match.
- Backpressure remains explicit and terminal when bounded audio or durable
  signal capacity is genuinely exhausted.
- Runtime coordinator tests cover Ready, reconnect capture retirement, Stop,
  and terminal failure; adapter tests cover unitization, attempt isolation,
  protocol progress, and provider normalization.
