# Use Tauri, Vue, and Rust

Date: 2026-05

The app is a Tauri 2 desktop shell with a Vue 3 + TypeScript + Vite frontend
and a Rust runtime. We need reliable Windows distribution, native audio
capture, OSC output, and future local-inference workers, without requiring
users to install any developer tooling.

Revisit if Tauri or Rust blocks a core audio, packaging, or runtime
requirement.
