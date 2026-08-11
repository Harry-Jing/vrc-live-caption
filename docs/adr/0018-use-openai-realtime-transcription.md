# Use OpenAI Realtime transcription

Date: 2026-08

The closed OpenAI recognition catalog contains exactly two path/model entries:
`gpt-transcribe` supports Completed publication, and `gpt-live-transcribe`
supports Completed or Live.

Both use Realtime transcription WebSockets behind the Recognition Module. The
application owns the closed catalog and capability records; it rejects unknown
or removed identifiers rather than migrating them or accepting arbitrary model
strings.

There is no REST/WAV recognition fallback, production Mock path, or silent model
switch. Provider events, identifiers, deltas, language hints, and transport
details are normalized inside the concrete Driver. Future service providers and
local runtimes join through their own Drivers rather than emulating the OpenAI
protocol.

Expected-language hints are input only and never masquerade as detected
language. The app exposes a detected language only when the provider reports it
for the completed recognition result.
