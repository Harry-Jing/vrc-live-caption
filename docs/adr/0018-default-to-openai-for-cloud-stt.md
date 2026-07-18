# Default to OpenAI for cloud STT

Date: 2026-06

The first STT provider is the OpenAI transcriptions API with
`gpt-4o-mini-transcribe`, uploading each bounded speech segment as one
request. It validates the cloud path with simple credential handling and no
streaming protocol work. This adapter emits completed captions only; that is
a fact about this path, not a rule for other providers.

Revisit if per-segment latency or cost fails real usage.
