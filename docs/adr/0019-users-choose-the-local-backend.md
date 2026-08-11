# Users choose the local backend preference

Local inference has one explicit preference: CPU or prefer NVIDIA GPU (CUDA),
defaulting to CPU. Utilization alone cannot predict VRChat frame time, VRAM
contention, or model behavior, so the app does not make an automatic performance
choice.

Before Start, unavailable hardware, an incompatible path, or failed CUDA
initialization may resolve a prefer-CUDA request to CPU only while preserving
the preference and showing the reason. The app always shows the effective
backend when it differs. The backend never changes during a runtime generation;
a worker failure ends that generation.
