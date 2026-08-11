# Localize the UI in the frontend

The application runtime emits stable machine-readable codes plus English
fallback text; the frontend owns user-facing localization. Runtime and wire
semantics therefore stay locale-independent, and changing locale does not alter
runtime state or diagnostic codes.
