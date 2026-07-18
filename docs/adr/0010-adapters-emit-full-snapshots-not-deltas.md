# Adapters emit full snapshots, not deltas

Date: 2026-07

Provider adapters reconcile raw deltas, appends, and replacements into
full-text caption snapshots with a monotonic revision and one state: ongoing
or completed. Snapshots identify their lane (source or translation), their
session/stream, and their caption unit when the path has real units.
Downstream consumers never replay provider protocols.

Consequences: there is no third "stable" state between ongoing and completed,
and no partial/stable/final wire ladder. Provider-specific stable-prefix
tricks stay inside the adapter.
