# Saved settings are not the runtime generation

Date: 2026-07

Saved configuration is desired state for the next Start. Each Start captures an
immutable selection; saving while captioning neither mutates nor restarts it.
Rust therefore owns one control snapshot containing desired state, active
generation state, credential status, and pending changes, instead of asking the
frontend to infer what is running from form values.

Revisit only for an explicitly designed hot-reconfiguration capability.
