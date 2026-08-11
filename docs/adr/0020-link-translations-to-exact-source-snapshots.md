# Link translations to exact source snapshots

Completion is terminal for one caption lane's revision chain, not for the entire
caption unit. A completed Source lane may therefore receive later Translation
snapshots without reopening recognition.

Every Translation snapshot references the exact generation, stream, unit, and
completed Source revision it consumed. The Caption Aggregate accepts it only
while that source remains authoritative. Unit identity alone, timestamps, the
latest visible text, and display position were rejected because none proves
which revisable source a translator consumed.

Admitting translation work pins that exact completed Source snapshot until the
work reaches a terminal outcome or its runtime generation Stops. Ordinary
history trimming cannot remove the unit while a reservation remains.

The reservation interface lands with the first real Translation Module, so its
ownership and cancellation semantics are defined by a real consumer rather than
a speculative API.

This lets source and translation progress independently while preventing late
or retried work from attaching to newer or unrelated speech.
