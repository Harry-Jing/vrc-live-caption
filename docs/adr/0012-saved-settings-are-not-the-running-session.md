# Saved settings are not the runtime generation

Date: 2026-07

Saved configuration is desired state for the next Start; each Start captures
an immutable selection. Saving during an active runtime generation neither
changes nor restarts it. Rust owns one revisioned control snapshot — desired
config, runtime status, generation selection, redacted service-credential
status, and derived pending changes — and the frontend renders that snapshot
instead of guessing from its own form state.

This exists because a save used to make the UI look as if the active generation
had changed when it had not. Stop bypasses config and credential I/O so it
can never be queued behind a slow Start.

Revisit if a future path gets a deliberately designed hot-reconfigure
capability.
