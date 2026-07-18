# Keep session history in memory only

Date: 2026-06

The app keeps a bounded in-memory history of recent completed captions and
diagnostics for the UI, and persists nothing. This covers preview and
diagnosis without storage, retention, or privacy design.

Reopen this when persistent history and export are built; they are on the
long-term feature list.
