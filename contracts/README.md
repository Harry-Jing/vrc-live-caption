# Contract fixtures

This directory pins the current Rust and TypeScript boundary shapes.

- Versioned payload fixtures include their own `contractVersion`. The filename
  names that payload version so a future supported migration can retain both
  fixtures deliberately.
- `tauri-ipc.json` and `wire-vocabulary.json` are unversioned source manifests.
  They describe the exact commands, events, enum values, and tagged-union
  discriminators compiled into one application build; they are not runtime
  negotiation protocols.
- Persisted `config.json` carries its separate `schemaVersion`. Diagnostic
  reports carry a separate `reportVersion` because copied reports may outlive
  the application build that produced them.

Contract and schema versions start at 1. After a format is released, its
number only increases for an incompatible serialized-shape or semantic change;
internal refactors and wording changes do not advance it. Formats found only
in the archived pre-main development history are not supported migrations.

Values such as `generation`, `revision`, `snapshotRevision`, caption revision,
and credential revision are runtime correlation or ordering counters, not
format versions. Scenario fixtures deliberately use distinct non-trivial
values so tests cannot accidentally couple independent counters.
