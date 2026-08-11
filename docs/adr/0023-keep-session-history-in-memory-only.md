# Keep caption history in memory only

Date: 2026-06

The app keeps a bounded in-memory history of recent completed captions and
diagnostics for the UI, and persists nothing. This covers preview and
diagnosis without storage, retention, or privacy design.

A user-initiated copy of a redacted diagnostic report is allowed because the
App still creates no persistent history or report file. That report excludes
all free-text event fields and uses an explicit metadata allowlist: app version,
normalized platform family, runtime status and timestamp, plus diagnostic
category, severity, stable code, and timestamp. Caption text, configuration,
device identifiers, network targets, paths, and service-credential status are
not serialized.

Reopen this when automatic persistence or file export is built; persistent
history remains on the long-term feature list.
