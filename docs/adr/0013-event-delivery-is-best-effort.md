# Event delivery is best-effort

Date: 2026-06

Runtime-to-UI events are at-most-once, and the runtime never depends on an
emit reaching the webview — an emit only fails while the webview is being
torn down, and the capture pipeline must not die because the view is gone.
The UI derives its state from the newest revisioned snapshot and can always
pull the full snapshot to resynchronize after a reload or missed event.
