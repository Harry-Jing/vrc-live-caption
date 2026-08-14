# Chatbox regression data

This directory contains versioned inputs for the Rust Chatbox regression tests.
They are test data, not persisted settings, IPC payloads, or public runtime
contracts.

The build scope and interpretation of these expectations come from the
[VRChat Chatbox reference](../../../docs/research/vrchat-chatbox-reference.md).
These fixtures keep only the data required for executable product regressions.

- `layout-cases-v1.json` is the product-owned 178-case synthetic corpus. It
  contains exact payloads, relations, and Unicode facts, but no screenshots,
  VRChat client observations, model predictions, or expected publisher output.
- `vrchat-client-observations-2026.3.1-1885-81193b80fa-v1.json` is a
  product-owned, build-scoped set of 52 VRChat client observations joined to
  the corpus by Case ID and payload SHA-256. Forty-nine observations are
  layout-prediction comparisons; three document why explicit product-side
  preparation is required. Optional observation fields are absent when they
  were not directly determined; absence does not mean `false`.

Treat stable Case IDs as test API: keep an ID when refining metadata, and add a
new ID when the payload or semantic purpose changes. `payload_sha256` is the
content identity used to join a client observation to its exact synthetic
payload. The tests recompute payload identity and core Unicode counts and
boundaries, and reject unknown fixture fields. Fixture changes must therefore
be reviewed together with the Chatbox regression tests.

No production path or repository script discovers these files at runtime. Rust
tests embed them by explicit path.

After a VRChat update, add a new build-named VRChat client-observation file and
keep prior files intact. Register the new file explicitly in
`regression_tests/support.rs`, add or update the corresponding observation
test, and run the full Chatbox regression suite. A new result must not silently
rewrite evidence from an older build. Raw screenshots, logs, extracted game
resources, and analysis tools are not repository test dependencies.
