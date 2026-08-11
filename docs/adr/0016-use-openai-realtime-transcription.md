# Use OpenAI Realtime transcription

OpenAI cloud recognition uses Realtime transcription WebSockets instead of
retaining the segmented REST/WAV path. Realtime supports completed items and
honest ongoing revisions behind one Driver boundary without making Runtime own
WAV upload or provider-specific segmentation.

The application owns a closed capability catalog and rejects unknown or removed
identifiers. There is no REST/WAV fallback or silent provider/model switch;
future providers and local runtimes use their own Drivers rather than emulate
the OpenAI protocol.
