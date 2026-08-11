# Users choose the local backend; no auto-selection

Date: 2026-07

Local inference has one explicit preference: CPU or prefer NVIDIA GPU (CUDA),
defaulting to CPU. Utilization alone cannot predict VRChat frame time, VRAM
contention, or model behavior, so the app does not make an automatic performance
choice. It always shows the effective backend and the reason when that differs
from the saved preference, and a crash never switches backend or uploads audio
to cloud on its own.

Before Start, unavailable hardware, an incompatible path, or failed CUDA
initialization may resolve a prefer-CUDA request to CPU only while preserving
the preference and showing the reason. The backend never changes during a
runtime generation; a worker failure ends that generation.
