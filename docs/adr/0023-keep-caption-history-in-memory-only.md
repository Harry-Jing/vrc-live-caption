# Keep caption history in memory only

Date: 2026-06

The app keeps bounded recent caption and diagnostic state in memory and creates
no automatic history or report files. A user may explicitly copy a redacted
diagnostic report built from an allowlist, but caption text, credentials,
credential state, diagnostic free text, configuration, device identity, network
targets, and paths stay out of it.

Persistent history or export requires a new retention and privacy design.
