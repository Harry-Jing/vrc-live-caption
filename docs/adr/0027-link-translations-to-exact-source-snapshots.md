# Link translations to exact source snapshots

Date: 2026-08

Status: accepted

## Context

The pre-baseline caption-session aggregate treated the first completed lane as
terminal for the entire caption unit. That matched the source-only Phase 4
path, but it would reject a translation arriving after its source completed.
The word "session" also hid that the aggregate retains completed history
across runtime generations.

Translation cannot safely attach to whichever source text is newest when the
result arrives. Source revisions can be replaced before completion, units may
complete out of order, recognition can reconnect within one runtime
generation, and old completed units remain visible in the aggregate.

## Decision

The Caption Aggregate is the authoritative normalized caption state. One active
caption stream belongs to a runtime generation and may survive replacement
recognition attempts. Open Source units describe source recognition activity;
they do not imply that correlated Translation work has settled.

Completion is terminal for one lane's revision chain, not for the entire
correlated caption unit. A completed Source lane closes source recognition for
that unit while still allowing a Translation lane to progress. Replay guards
and monotonic revision checks are therefore keyed by caption unit and lane.

Every Translation snapshot carries a `sourceRef` containing the exact runtime
generation, caption stream, caption unit, and completed Source revision that it
consumed. The aggregate accepts the translation only while that exact source
snapshot remains authoritative and retained. Source snapshots never carry a
`sourceRef`.

Retention is work-scoped, not count- or time-based. When Phase 5 admits bounded
translation work, it also reserves that exact completed Source snapshot in the
aggregate. The reservation is released when the work completes, fails, times
out, is cancelled, or its runtime generation stops. Normal history trimming
can remove only units that have no active reservation. The reservation API
lands with the first real Translation Module so its ownership and cancellation
semantics are defined by a real consumer rather than a speculative interface.

Stop remains a hard generation cutoff. Closing a runtime generation discards
ongoing source and translation snapshots and rejects late results, while
retaining bounded completed history for the UI.

## Considered Options

- Marking the whole caption unit terminal on Source completion was rejected
  because normal asynchronous translation would always arrive too late.
- Linking a translation by unit id alone was rejected because it cannot prove
  which revisable Source snapshot the translator consumed.
- Attaching to the latest visible Source text or using timestamps was rejected
  because event timing and display order are not correlation contracts.
- Keeping the pre-baseline `CaptionSessionSnapshot` name was rejected because
  the state intentionally spans multiple runtime generations.

## Consequences

- Phase 5 can add a Translation Module without changing caption identity or
  guessing alignment.
- Source and Translation revisions remain independently monotonic and may
  complete at different times.
- The aggregate must retain lane-aware replay metadata and completed Source
  snapshots for as long as admitted translation work holds a reservation.
- Publication still consumes application-normalized state; provider item ids,
  deltas, and timing heuristics remain inside concrete drivers.
