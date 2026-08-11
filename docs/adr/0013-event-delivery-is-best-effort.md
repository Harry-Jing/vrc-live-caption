# Event delivery is best-effort

Runtime-to-UI events are at-most-once, and the runtime never depends on an
emit reaching the webview. The UI derives its state from the newest revisioned
snapshot and can pull the full snapshot to resynchronize after a reload or
missed event.
