# Cloud connections honor explicit routes and endpoints

Cloud connections honor an explicitly selected environment proxy or the
operating system's current manual proxy route. A malformed, unsupported, or
failed selected route fails closed; the app never treats it as permission to
connect directly.

Only verified proxy mechanisms are accepted; unsupported routes fail explicitly
rather than being approximated. Platform-specific discovery and bypass
semantics remain private transport details.

A custom OpenAI-compatible base URL is a separate endpoint trust choice, not a
proxy route or automatic fallback. It must be explicitly selected and disclosed
before Start because its operator receives the configured service credential and
the user content sent through that endpoint, including microphone audio for
recognition and Source text for text-driven translation.
