# Cloud requests follow the system proxy; a relay API option is planned

Date: 2026-07

Cloud requests follow the operating system's manual proxy configuration, read
when the runtime creates its HTTP client. This serves players in China who
reach OpenAI through a Clash-style proxy: the common "system proxy" toggle is
covered, TUN mode needs nothing from the app, and connection failures get a
dedicated network-unreachable diagnostic. PAC, WinHTTP, and machine-policy
proxies are out of scope.

The maintainer is in the US and cannot validate these setups personally, so
real Chinese-network testing is required before recommending the app to that
community.

Also planned: a custom OpenAI-compatible base URL setting ("relay API" /
中转), which lets users in China use the cloud path with no proxy at all.
Many prefer this over any proxy setup. Sending a key to a third-party relay
is the user's own choice.
