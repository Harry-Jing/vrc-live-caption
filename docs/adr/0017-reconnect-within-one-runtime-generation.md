# Reconnect transient recognition failures within one runtime generation

A user Start creates one runtime generation with an immutable path selection and
one hard Stop boundary. A structured transient failure may replace the current
recognition attempt inside that generation and remains visibly reconnecting;
authentication, permission, configuration, protocol, and unknown failures are
terminal.

Repeated transient failures may continue retrying until Stop; a terminal
classification ends the generation visibly.

The retired attempt is fenced before a replacement can publish. Unconfirmed and
ambiguous audio is discarded rather than replayed, because the app cannot prove
what a lost provider connection accepted. Stop interrupts connection and
backoff work and continues to reject every late result.

This trades some speech around an outage for recovery without duplication,
mis-correlation, or a hidden provider/model change.
