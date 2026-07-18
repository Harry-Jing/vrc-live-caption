# Signal speech activity with the typing indicator

Date: 2026-06 (updated 2026-07)

While speech or publication activity is active, the app sends the VRChat
typing indicator on; it turns off on resolution, failure, and Stop. It fills
the gap between speaking and the first published text, using VRChat's native
presence signal.

Consequences: real-client testing showed VRChat hides an unrefreshed
indicator after about five seconds, so the publisher reasserts typing-on
every four seconds while activity continues. Typing packets do not consume
text-send pacing opportunities.

Revisit if in-game validation shows the indicator confuses other players.
