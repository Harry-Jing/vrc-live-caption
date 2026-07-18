# The bounded cloud path caps units at 30 seconds

Date: 2026-07

The segmented OpenAI path closes a normal utterance after the existing
1.2-second silence boundary and uses 30 seconds as the hard maximum for
uninterrupted speech. Real-client testing showed the earlier 12-second
maximum split a roughly 20-second thought into two units. The maximum is an
internal adapter parameter, not a user setting or a rule for other providers.

Revisit if latency approaches the request timeout or longer units hurt
recognition quality.
