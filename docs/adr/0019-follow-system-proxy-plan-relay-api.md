# Cloud requests follow the system proxy; a relay API option is planned

Date: 2026-07

Cloud requests follow the operating system's manual proxy configuration, read
when the runtime creates its HTTP client. This serves players in China who
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

The maintainer is in the US and cannot validate these setups personally, so
real Chinese-network testing is required before recommending the app to that
community.

Also planned: a custom OpenAI-compatible base URL setting ("relay API" /
中转), which lets users in China use the cloud path with no proxy at all.
Many prefer this over any proxy setup. Sending a key to a third-party relay
is the user's own choice.
