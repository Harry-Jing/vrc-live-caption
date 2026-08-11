# Publication timing is Completed or Live

Users choose **Completed**, which publishes only terminal lane snapshots, or
**Live**, which may also publish ongoing revisions. Timing is independent from
provider, model, and content selection.

A two-pass pipeline was rejected as the default design: recognition paths expose
different final, streaming, and continuous shapes, while running a low-latency
recognizer plus a correction recognizer beside VRChat is too costly for many
machines. Two-pass remains a separately benchmarked future path, not a
publication timing mode.

The application resolves the request against the complete path's capabilities.
An incompatibility preserves the user's choices and offers explicit alternatives
rather than switching a path or mode or inventing completion. Live remains
contingent on real-observer testing showing that replacement is readable.
