# Architecture

## Scope

This document defines runtime boundaries, ownership, normalized semantics, and
data flow. Code and tests own mechanics inside a boundary; `contracts/` owns
exact cross-language fixtures and manifests; the [roadmap](./roadmap.md) owns
implementation status.

The **Current architecture** sections describe the implemented system shape.
The **Planned extension seams** are accepted boundaries for roadmap work, not a
claim that those features exist.

## System view

```text
Vue application
  <-> typed application gateway
  <-> Tauri desktop composition
      -> runtime generation
         -> audio capture
         -> Recognition Module -> Recognition Driver
         -> Caption Aggregate
         -> publication plan
         -> App view + Chatbox publishers

Planned:
  Caption Aggregate -> Translation Module
  Recognition Module -> local worker Driver
  separate incoming-audio pipeline
```

## Cross-cutting invariants

- The frontend never receives raw audio or provider wire events.
- Capture produces provider-independent audio; recognition paths own their
  protocol, boundary, buffering, and model-specific behavior.
- Recognition and translation produce normalized full snapshots, not raw deltas
  ([ADR 0010](./adr/0010-drivers-emit-full-snapshots-not-deltas.md)).
- A provider or worker never publishes directly to Chatbox. Output sinks consume
  application-owned caption state.
- Publication eligibility and sink pacing are separate decisions.
- Capture and recognition never wait on translation or Chatbox output.
- Failures are application-classified and cannot silently change a selected
  path, publication mode, content selection, or local backend.
- Stop is the final output boundary for one runtime generation
  ([ADR 0011](./adr/0011-stop-is-a-hard-cutoff.md)).

## Current architecture

### Frontend and desktop boundary

The Vue application reaches runtime control, settings, audio, credentials, and
OSC through a typed application gateway. Tauri IPC and the deterministic browser
Preview are adapters behind that boundary; feature state does not depend on a
transport directly.

Rust is authoritative for runtime and caption state. The frontend receives
revisioned snapshots, rejects older revisions, and can pull the current snapshot
after reload or missed best-effort events
([ADR 0013](./adr/0013-event-delivery-is-best-effort.md)). UI-only capabilities,
such as confirmation and clipboard output, stay in narrow host ports rather than
expanding the runtime gateway.

Exact command names, event identifiers, closed wire vocabulary, and shared
scenario payloads are maintained in [contracts/](../contracts/). Presentation
labels and states are derived in the frontend; caption payloads do not contain
UI placeholders.

### Runtime control and lifecycle

Saved configuration is desired state for the next Start. A Start captures an
immutable recognition selection, credential identity, publication request, and
audio/OSC settings into a new runtime generation. Saving later does not mutate
that generation ([ADR 0012](./adr/0012-saved-settings-are-not-the-runtime-generation.md)).

Start validates the requested combination before opening capture. Incompatible
choices remain visible with supported alternatives; the app does not rewrite
them. Rust exposes desired state, the resolved pipeline plan, credential status,
runtime status, the active generation selection, and pending next-generation
changes as one revisioned control snapshot.

A safe unit-level failure may leave the generation running. A terminal failure
moves it to an explicit error state. Stop bypasses ordinary queued work, closes
capture and publication admission, rejects late results, and releases resources.

### Audio boundary

The desktop runtime owns microphone selection and capture. Capture supplies
continuous, provider-independent mono audio to the active Recognition Module
through a bounded, non-blocking boundary. Backpressure fails visibly rather than
blocking the capture callback or silently losing accepted speech.

The same frames produce short-window level, gate, and clipping telemetry for the
UI; raw PCM never crosses IPC. A local microphone probe is mutually exclusive
with runtime capture and opens no provider, OSC, credential, or persistence path.

### Recognition boundary

One runtime generation owns one active Recognition Module. Runtime owns capture,
the generation output fence, and publication; the Module owns path-specific
execution:

```text
runtime
  -> continuous audio + lifecycle control
Recognition Module
  -> Recognition Driver
     -> speech boundaries and caption-unit lifecycle
     -> replaceable recognition attempts
     -> provider protocol or worker I/O
     -> normalized ordered signals
  <- ready, reconnecting, caption, unit, and error signals
```

This ownership keeps provider commits, identifiers, JSON, worker messages,
model-native frames, and inference windows out of Runtime
([ADR 0016](./adr/0016-recognition-modules-own-path-execution.md)). A concrete
Driver may be cloud or local without pretending that its internal attempts,
budgets, or speech boundaries are identical.

The current cloud Module exposes the closed OpenAI recognition catalog selected
in [ADR 0018](./adr/0018-use-openai-realtime-transcription.md). Both paths use
Realtime transcription behind the same Module boundary; no REST/WAV or product
Mock fallback exists.

Expected-language hints are Driver inputs, not detected-language results. A
detected language is exposed only when a provider completion reports one.

A retryable recognition failure may replace an attempt inside the same runtime
generation. The retired attempt cannot publish again, ambiguous audio is not
replayed, capture resumes only for a fresh ready attempt, and the UI remains in a
visible reconnecting state. Retry policy uses structured application
classifications; provider-authored messages and metadata stay inside the Driver
and neither choose policy nor enter application diagnostics
([ADR 0019](./adr/0019-reconnect-within-one-runtime-generation.md)).

### Caption Aggregate

The Caption Aggregate is the application-owned normalized caption state. It
contains the active caption stream, open Source units, and bounded recent
snapshots. Unit identity follows application correlation rather than provider
item identifiers or wall-clock order.

The current Aggregate contract represents both lanes even though no Translation
Module is implemented. Each lane has its own monotonic revision chain, and
completion is terminal for that lane, not the whole caption unit. A Translation
snapshot identifies the exact completed Source snapshot it consumed, preventing
timing or display order from becoming a correlation contract
([ADR 0022](./adr/0022-link-translations-to-exact-source-snapshots.md)).

The Aggregate may retain bounded completed captions from older runtime
generations for the app view. Stop removes ongoing work and rejects late output
without turning retained completed captions into an active session.

### Pipeline planning

Capabilities belong to a complete path, not a model name. A path declares the
input shape, boundary ownership, update and revision behavior, produced lanes,
and supported publication timing.

Current planning resolves source-only publication. Translation content selection
must extend the same planner for path capabilities, publication mode, and output
constraints rather than introduce a separate one. An incompatible request
remains explicit instead of causing a silent path or mode change
([ADR 0007](./adr/0007-publication-timing-is-completed-or-live.md)).

### Chatbox publication

Completed and Live publication are separate workers behind one runtime-facing
facade. They share process-wide send pacing but apply different queue semantics:

- Completed preserves ordered caption units and lossless pagination within its
  bounded admission policy.
- Live recomputes a latest-wins viewport and may skip obsolete revisions instead
  of replaying them as history.

Publisher instances are generation-scoped. Stop discards their queued caption
text rather than draining it. Typing indication is lifecycle control outside the
text-send pacer.

OSC, pacing, layout, wrapping, clipping, and validation constraints are defined
by the [VRChat Chatbox reference](./research/vrchat-chatbox-reference.md).

### Network and secrets

Service credentials are resolved at desktop composition and bound to the
prepared recognition path without exposing plaintext to Runtime Control or the
frontend. Config and diagnostics carry only redacted credential status.

Cloud connections follow the user's selected system or explicit environment
proxy route. Unsupported or malformed selected proxy configurations fail closed;
the app does not silently bypass them with a direct connection
([ADR 0017](./adr/0017-cloud-connections-honor-the-selected-proxy-route.md)).
Network targets use bounded, cancellable resolution so Start and Stop cannot
wait indefinitely on an operating-system lookup.

## Planned extension seams

### Translation

The first Translation Module consumes completed Source snapshots and emits
correlated Translation snapshots. Work is bounded and cancellable; a stale,
timed-out, or late result cannot overwrite another source revision or unit.

Transcript-driven and direct-audio translation remain different path shapes.
Repeatedly translating every unstable source revision is not the default Live
strategy because it amplifies requests, races, cost, and visible rewrites.

### Local recognition

Local inference runs in a Rust worker outside the desktop process
([ADR 0020](./adr/0020-keep-local-inference-out-of-process.md)). Its Driver sits
behind the same Recognition Module boundary while owning model loading,
resampling, inference cadence, IPC, backend state, and worker health.

A crash ends the runtime generation and requires an explicit user choice; it
does not restart silently on CPU or switch to cloud. Candidate runtimes, models,
backends, packaging choices, and benchmarks are in the
[local recognition evaluation](./research/local-recognition-evaluation.md).

### Incoming captions

System or VRChat audio may later enter a separate incoming pipeline with its own
capture, caption lanes, and publication policy. It does not share outgoing
microphone assumptions.
