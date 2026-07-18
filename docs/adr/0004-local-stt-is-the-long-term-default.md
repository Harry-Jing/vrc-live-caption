# Local inference is the long-term default

Date: 2026-06 (extended to translation 2026-07)

Cloud STT is the current default, but the long-term default is local STT.
Getting an OpenAI key requires an account, international payment, and often a
proxy — a hard barrier for a large part of the target community, especially
Chinese players. The default switches to local only after a local engine is
validated on real Windows machines running VRChat; cloud then remains
available as a quality option.

The same direction applies to translation: the first translation
implementation is cloud text translation, and the long-term goal is a local
translator once the primary speech path is stable. Local translation loads a
second model, so it also switches only after a real benchmark shows
acceptable quality and resource use beside VRChat.

Revisit if no local engine reaches acceptable accuracy and resource usage on
machines that also run VRChat.
