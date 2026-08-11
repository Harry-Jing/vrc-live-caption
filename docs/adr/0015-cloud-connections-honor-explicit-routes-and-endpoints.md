# Cloud connections honor explicit routes and endpoints

Cloud connections honor an explicitly selected environment proxy or the
operating system's current manual proxy route. A malformed, unsupported, or
failed selected route fails closed; the app never treats it as permission to
connect directly.

Only verified proxy mechanisms are accepted; unsupported routes fail explicitly
rather than being approximated. Platform-specific discovery and bypass
semantics remain private transport details.

A custom OpenAI-compatible base URL is a path-scoped endpoint trust choice, not
a proxy route or fallback. It must be selected and disclosed before Start
because its operator receives that path's credential and content: microphone
audio for Recognition or Source text for text-driven Translation.

Phase 5 Custom Translation requires HTTPS, including on loopback, and a
separate OS-stored credential; it never receives the Official credential or
Recognition audio. Redirects and silent direct-route fallback are forbidden,
and request fields cannot guarantee the Custom operator's retention policy.
The concrete Responses profile is in
[ADR 0021](./0021-use-openai-responses-for-completed-translation.md).
