# Stop is a hard cutoff

Stop is a trust action: it releases the microphone, discards buffered and
queued speech, and rejects every late caption or translation result from the
stopped generation, for both the App and the Chatbox. The only allowed output
after Stop is one typing-indicator-off cleanup message.

Consequences: speech captured just before Stop is lost by design and reported
as a diagnostic. An uncancellable in-flight request may finish during cleanup,
but its result is ignored.
