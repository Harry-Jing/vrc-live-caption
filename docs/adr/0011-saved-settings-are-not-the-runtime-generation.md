# Saved settings are not the runtime generation

Saved configuration is desired state for the next Start. Each Start captures an
immutable selection; saving while captioning neither mutates nor restarts it.
Runtime exposes desired and active state together instead of asking the
frontend to infer what is running from form values.

Hot reconfiguration requires a separate decision.
