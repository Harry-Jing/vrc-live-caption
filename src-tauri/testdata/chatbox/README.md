# Chatbox regression data

This directory contains versioned inputs for the Rust Chatbox regression tests.
They are test data, not persisted settings, IPC payloads, or public runtime
contracts.

- `layout-cases-v1.json` is the generator-owned canonical projection of the
  reviewed 178-case synthetic corpus. It contains exact payloads, relations,
  and independently computed Unicode facts, but no screenshots, VRChat client
  observations, model predictions, or expected publisher output.
- `vrchat-client-observations-2026.3.1-1885-81193b80fa-v1.json` is a
  build-scoped selection of 52 high-confidence VRChat client observations
  joined to the corpus by Case ID and payload SHA-256. Forty-nine observations
  are layout-prediction comparisons; three document why explicit product-side
  preparation is required.

Do not edit or reformat the corpus by hand. Update it with the standalone,
dependency-locked `chatbox-corpus export-live-caption <output>` command, review
the embedded source and manifest hashes, and then run the Rust regression and
Chatbox behavior tests. The generator is an authoring tool, not a build or CI
dependency.

No production path or repository script discovers these files at runtime. Rust
tests embed them by explicit path, while the standalone exporter writes to the
caller-selected output path.

After a VRChat update, add a new build-named VRChat client-observation file and
keep prior files intact. A new result must not silently rewrite evidence from
an older build. Raw screenshots, logs, extracted game resources, and analysis
tools stay in the separate local research workspace and are not test
dependencies.
