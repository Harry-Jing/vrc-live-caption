# Publication timing is Completed or Live

Date: 2026-07

Users choose between two timing modes for Chatbox output: **Completed**
(publish a caption only when its unit is complete) and **Live** (also publish
ongoing revisions).

The project's earliest design was a two-pass pipeline — a fast recognizer for
instant text plus a correction recognizer for quality. It was dropped when it
became clear that speech models come in many shapes (final-only, streaming,
continuous) and that running two models costs more than most machines can
spare while VRChat runs. The replacement principle: the user picks the
experience, the application resolves a Caption Pipeline Plan that checks
whether the selected path can honestly deliver it, and an incompatible choice
is explained with explicit alternatives — keep the path and pick a supported
mode, or keep the mode and pick a compatible path. The app never silently
switches path or mode, and never invents a completion the path did not
produce.

Consequences: the bounded OpenAI path supports Completed; a streaming path
with real unit completion supports both; a continuous path with no per-unit
completion supports Live only. Two-pass remains a possible far-future
experiment, not a setting.

Revisit if in-game testing shows Live replacement is unreadable or VRChat
changes Chatbox replacement semantics.
