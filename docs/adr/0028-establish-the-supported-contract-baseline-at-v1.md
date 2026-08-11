# Establish the supported contract baseline at V1

Date: 2026-08

Status: accepted

Before the first supported mainline baseline, the development branch used
several incompatible config and UI-facing contract versions without shipping
them to users. The final pre-main cleanup resets persisted App Config, Runtime
Control, and Caption Aggregate contracts to V1 once; archived development
formats remain unsupported. Current implementation types use semantic names
without version suffixes, while source-only IPC and vocabulary manifests stay
unversioned because they do not cross a runtime compatibility seam.

After this baseline, every released durable or independently consumed format
advances monotonically and is never renumbered or reused. Persisted settings,
diagnostic reports, and future separately executable worker protocols keep
independent versions and migration policies. Runtime generations, revisions,
document numbers, roadmap phases, application SemVer, and dependency versions
are separate identities and do not participate in this reset.
