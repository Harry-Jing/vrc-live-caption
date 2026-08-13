# Contract artifacts

This directory holds shared evidence for the Rust/TypeScript boundary. The
artifacts complement the Rust types, TypeScript decoders, and tests; payload
fixtures are representative compatibility scenarios, not complete JSON Schema.

- `*-vN.json` files are versioned payload scenarios consumed by both languages.
- `tauri-ipc.json` is the exact manifest of command and event identifiers in one
  application build.
- `wire-vocabulary.json` is the exact manifest of closed enum values and tagged
  union discriminators shared across the boundary.
- `chatbox-layout-cases-v1.json` is a portable synthetic corpus consumed by the
  Rust Chatbox tests. It contains exact payloads, relations, and independently
  computed Unicode facts, but no screenshots, runtime observations, layout
  predictions, or expected publisher output. Its embedded source hashes pin the
  reviewed 178-case corpus projection; normal builds and tests require no
  external research tools or VRChat installation.

`chatbox-layout-cases-v1.json` is generator-owned canonical JSON. Do not edit or
reformat it by hand. A corpus maintainer updates it with the standalone,
dependency-locked `chatbox-corpus export-live-caption <output>` command, reviews
the embedded source and manifest hashes, and then runs the Rust contract and
Chatbox behavior tests in this repository. The generator is an authoring tool,
not a build or CI dependency.

## Compatibility baseline

The first merge of this development line into `main` establishes the supported
V1 baseline for every persisted or independently consumed format present in
that merge, even without a packaged release. App Config, Runtime Control, and
Caption Aggregate are part of that baseline. Formats that existed only before
it are unsupported and receive no migration; a format introduced later begins
its own compatibility commitment when it first reaches `main`.

After the mainline baseline, each format owns an independent monotonic version.
Advance it for an incompatible serialized-shape or semantic change; never
renumber, reuse, or reset a supported value. Internal refactors, code renames,
test-fixture edits, and prose changes do not advance a format version.

Persisted settings use `schemaVersion`. UI-facing payloads use their own
`contractVersion`. Diagnostic reports use `reportVersion`, and a future worker
protocol will need its own version if it becomes independently deployed.

Scenario fixture versions are deliberately distinct from runtime revisions,
application SemVer, and dependency versions so tests cannot couple them.
