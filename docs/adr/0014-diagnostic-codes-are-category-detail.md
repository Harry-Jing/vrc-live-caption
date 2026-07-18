# Diagnostic codes are `<category>.<detail>`

Date: 2026-06

Every diagnostic event carries a stable machine-readable code whose prefix
equals its serialized category, with an exhaustive error-to-category mapping
in code. Codes are the contract for filtering, tests, and future UI
localization; `message` and `detail` are English fallback text only.
