# Identify audio devices by stable id

Config stores backend-issued device ids, never display names — duplicate names
and reconnects are common, especially on Windows. A saved but disconnected
device stays selectable in the UI instead of silently falling back to another
microphone.

This decision depends on those ids remaining stable across reconnects and
routine operating-system updates.
