# Keep local inference out of process

Date: 2026-05

Local STT and translation run behind Rust workers or sidecars, never inside
the main app process. Users should not need Python, PyTorch, or CUDA Toolkit
installs, and a model or GPU crash must not take down the app.

Consequences: a worker crash stops that recognition session and offers an
explicit retry or backend change. It never restarts silently on CPU and never
falls back to cloud on its own.
