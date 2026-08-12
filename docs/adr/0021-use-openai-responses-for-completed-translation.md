# Use OpenAI Responses for completed translation

Phase 5 uses one temporary `OpenAiResponsesCompletedText` path. Each admitted
authoritative Completed Source resolves as either one terminal Translation or a
visible failure; successful work uses one non-streaming Responses request. The
profile fixes `gpt-5.6-luna`,
`reasoning.effort: none`, `store: false`, and explicit `en` or `zh-Hans`
targets, with no model, protocol, provider, or local fallback. A failed
[evaluation](../research/cloud-translation-evaluation.md) requires a new
decision rather than a runtime switch.

Official uses the existing OpenAI credential. Custom uses a separate OS-stored
credential and an explicitly selected HTTPS API base URL implementing the same
profile; the app appends one `responses` segment and never guesses `/v1`.
Selecting it does not reroute Recognition audio. Endpoint trust follows
[ADR 0015](./0015-cloud-connections-honor-explicit-routes-and-endpoints.md);
generation immutability, exact Source linkage, Stop, and presentation remain
governed by [ADR 0011](./0011-saved-settings-are-not-the-runtime-generation.md),
[ADR 0020](./0020-link-translations-to-exact-source-snapshots.md),
[ADR 0010](./0010-stop-is-a-hard-cutoff.md), and
[ADR 0007](./0007-bilingual-output-is-one-asynchronous-view.md).
