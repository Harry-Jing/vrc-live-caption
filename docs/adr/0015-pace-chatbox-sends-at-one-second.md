# Pace Chatbox sends at one second

Date: 2026-07

All Chatbox text-send attempts stay at least 1000 ms apart, measured from the
previous actual attempt and including failed attempts. Real-client
numbered-message tests skipped messages at 200–900 ms cadences but delivered
120 consecutive messages at 1000 ms — consistent with a leaky bucket of about
five messages replenishing one per second. The publisher does not spend the
initial burst.

Consequences: Live output coalesces to one latest-wins viewport, and
Completed output uses an ordered bounded page queue — queued Live revisions
would replay stale guesses, while dropped Completed pages would lose real
speech. Full evidence:
[vrchat-chatbox-reference.md](../research/vrchat-chatbox-reference.md).

Revisit only after re-running the numbered-message test on a newer VRChat
client.
