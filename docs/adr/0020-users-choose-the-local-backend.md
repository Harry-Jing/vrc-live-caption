# Users choose the local backend; no auto-selection

Date: 2026-07

Local inference has one global preference: CPU or prefer NVIDIA GPU (CUDA),
defaulting to CPU. There is no automatic performance selector — utilization
numbers do not reveal frame-time or VRAM contention while VRChat runs, so
model/backend recommendations wait for real benchmarks. The app always shows
the effective backend when it differs from the preference, with the reason,
and a crash never switches backend automatically.

CPU is implemented first because it is easiest to package for every Windows
x64 machine; that is engineering order, not a quality ranking.
