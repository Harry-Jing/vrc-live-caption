# Reconnect transient recognition failures within one runtime generation

Date: 2026-08

Status: accepted and implemented

## Context

The product is intended to stay active while a player is in VR. Ending the
runtime after every temporary DNS, TCP, WebSocket, rate-limit, or provider
availability failure would require an inaccessible manual Start. Replaying
audio across a lost Realtime connection is not safe either: the app cannot
prove which commits the provider accepted, so replay can duplicate speech or
attach a result to the wrong caption unit.

## Decision

A user Start creates one **runtime generation** with immutable selected
settings and one hard Stop boundary. A retryable failure may replace the
OpenAI provider connection inside that generation. Connection attempts receive
monotonic internal epochs, and the old worker is stopped and joined before a
new connection can publish. Stop interrupts connection setup and backoff and
continues to reject every later App or Chatbox output from that generation.

Retryability comes only from structured application classifications. Temporary
network failures, provider rate limits, and provider unavailability retry with
jittered exponential backoff from 500 ms to a 30-second cap. A connection must
remain ready for at least 30 seconds before it resets the accumulated backoff,
so a flapping endpoint cannot force a permanent 500 ms retry loop.
Authentication, permission, invalid-request, usage-limit, proxy-policy,
TLS-configuration, and unknown failures remain terminal. Provider-authored
messages and metadata are discarded at the Driver boundary; they never choose
retry policy or enter diagnostics. When an HTTP 429 handshake includes a
recognized structured quota code, it is terminal rather than being mistaken
for a transient rate limit.

At a reconnect boundary the microphone and current recognition attempt are closed,
unconfirmed caption units end visibly, and buffered or ambiguous audio is
discarded. Capture resumes only after a fresh recognition attempt is ready. Audio
is never replayed, the model and language hints never change, and no fallback
provider or transport is selected. The UI exposes `reconnecting` rather than
pretending to be Running or ending the generation.

## Consequences

- A brief outage can recover without the player removing their headset.
- Speech around the outage may be lost, but it cannot be duplicated or
  mis-correlated.
- Repeated transient failures may keep retrying until Stop; terminal failures
  still move the runtime to Error with a stable application diagnostic.
- Tests must keep connection-attempt cancellation separate from generation
  cancellation and must prove that Stop interrupts backoff.
