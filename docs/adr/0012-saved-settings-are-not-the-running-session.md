# Saved settings are not the running session

Date: 2026-07

Saved configuration is desired state for the next Start; each Start captures
an immutable copy. Saving during an active session neither changes nor
restarts it. Rust owns one revisioned control snapshot — desired config,
runtime status, active-session selection, redacted secret status, and derived
pending changes — and the frontend renders that snapshot instead of guessing
from its own form state.

This exists because a save used to make the UI look as if the running session
had changed when it had not. Stop bypasses config and credential I/O so it
can never be queued behind a slow Start.

Revisit if a future provider gets a deliberately designed hot-reconfigure
capability.
