# Windows is Tier 1; macOS and Linux are Tier 2

Date: 2026-07

Windows x86_64 is the first-release platform and the only one with complete
real-machine validation, because it is the project's only full VRChat test
environment (development happens on macOS). macOS arm64 and Linux x86_64 stay
green in CI — compilation, tests, and native package builds — to catch
portability regressions early.

Consequences: a Tier 2 build or test failure blocks merging, but Tier 2
bundles are test artifacts, not release commitments, and platform-specific
Tier 2 runtime issues may be deferred.

Revisit if repeatable real-machine validation becomes available for a Tier 2
platform.
