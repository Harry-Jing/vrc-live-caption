# Identify audio devices by stable id

Date: 2026-06

Config stores CPAL device ids, never display names — duplicate names and
reconnects are common, especially on Windows. A saved but disconnected device
stays selectable in the UI instead of silently falling back to another
microphone.

Revisit if CPAL ids prove unstable across driver or OS updates.
