# Use OpenAI Realtime transcription

OpenAI cloud recognition uses Realtime transcription WebSockets instead of
retaining the segmented REST/WAV path. Realtime supports completed items and
honest ongoing revisions behind one Driver boundary without making Runtime own
WAV upload or provider-specific segmentation.

The OpenAI Driver applies a 1.2-second silence boundary and a 30-second hard
maximum to uninterrupted speech. Real-client testing showed that the earlier
12-second maximum split an approximately 20-second thought, so this remains a
path-internal parameter rather than a user setting or product-wide rule.

The application owns a closed capability catalog and rejects unknown or removed
identifiers. There is no REST/WAV fallback or silent provider/model switch;
future providers and local runtimes use their own Drivers rather than emulate
the OpenAI protocol.
