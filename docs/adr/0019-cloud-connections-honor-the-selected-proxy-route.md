# Cloud connections honor the selected proxy route

Date: 2026-07

Cloud connections honor an explicitly selected environment proxy or the
operating system's current manual proxy route. A malformed, unsupported, or
failed selected route fails closed; the app never treats it as permission to
connect directly.

This supports common system-proxy setups while keeping the boundary honest:
PAC/WPAD execution, SOCKS, standalone machine-policy routing, and other
unimplemented mechanisms are reported as unsupported rather than approximated.
Platform-specific discovery and bypass semantics remain private transport
details.

Real Chinese-network validation is required before recommending a cloud route
to that community.
