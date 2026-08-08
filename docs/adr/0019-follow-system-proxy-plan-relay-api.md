# Cloud requests follow the system proxy; a relay API option is planned

Date: 2026-07

Cloud requests follow the operating system's manual proxy configuration, read
when the runtime opens its cloud transport. This serves players in China who
reach OpenAI through a Clash-style proxy: the common "system proxy" toggle is
covered, TUN mode needs nothing from the app, and connection failures get a
dedicated network-unreachable diagnostic. Executing PAC scripts, resolving
WPAD, reading standalone WinHTTP configuration, and routing through
machine-policy proxies are out of scope. Out of scope never authorizes a silent
direct connection: when the current-user settings visibly select PAC or WPAD,
the app rejects the unsupported selection before opening an OpenAI connection.

On Windows, the active connection's current-user WinINet/Internet Options
settings are read through the documented WinHTTP IE-configuration bridge. That
bridge exposes manual proxy, bypass, PAC, and auto-detection selections for the
active LAN or VPN; it does not add standalone WinHTTP or machine-policy proxy
routing to the supported surface.

On macOS, manual proxy settings are resolved for the actual OpenAI target with
CFNetwork. This preserves Apple's `ExceptionsList` wildcard rules and
`ExcludeSimpleHostnames` behavior instead of translating them into curl-style
`NO_PROXY` semantics. The proxy dictionary is type-checked before the narrow
CFNetwork boundary; malformed values fail closed. PAC and WPAD are rejected
before target resolution, and the first target-specific CFNetwork route is
authoritative. Environment `NO_PROXY` is consulted only when an explicit
environment `HTTPS_PROXY` or `ALL_PROXY` selected that environment route; it
does not override operating-system routing by itself.

The maintainer is in the US and cannot validate these setups personally, so
real Chinese-network testing is required before recommending the app to that
community.

Also planned: a custom OpenAI-compatible base URL setting ("relay API" /
中转), which lets users in China use the cloud path with no proxy at all.
Many prefer this over any proxy setup. Sending a key to a third-party relay
is the user's own choice.
