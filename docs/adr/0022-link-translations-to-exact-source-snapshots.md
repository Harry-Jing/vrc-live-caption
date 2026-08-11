# Link translations to exact source snapshots

Date: 2026-08

Completion is terminal for one caption lane's revision chain, not for the entire
caption unit. A completed Source lane may therefore receive later Translation
snapshots without reopening recognition.

Every Translation snapshot references the exact generation, stream, unit, and
completed Source revision it consumed. The Caption Aggregate accepts it only
while that source remains authoritative. Unit identity alone, timestamps, the
latest visible text, and display position were rejected because none proves
which revisable source a translator consumed.

Admitting translation work pins that exact completed Source snapshot in the
Aggregate. The reservation is released when the work completes, fails, times
out, is cancelled, or its runtime generation Stops. Ordinary history trimming
cannot remove a unit while any reservation remains.

This lets source and translation progress independently while preventing late
or retried work from attaching to newer or unrelated speech.
