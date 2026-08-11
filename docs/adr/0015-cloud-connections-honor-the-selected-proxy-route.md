# Cloud connections honor the selected proxy route

Cloud connections honor an explicitly selected environment proxy or the
operating system's current manual proxy route. A malformed, unsupported, or
failed selected route fails closed; the app never treats it as permission to
connect directly.

Only verified proxy mechanisms are accepted; unsupported routes fail explicitly
rather than being approximated. Platform-specific discovery and bypass
semantics remain private transport details.
