# Default to OpenAI for cloud recognition

Date: 2026-06

Status: superseded by
[ADR 0024](./0024-use-openai-realtime-transcription.md).

The first cloud-recognition path uses the OpenAI transcriptions API with
`gpt-4o-mini-transcribe`, uploading each bounded speech segment as one
request. It validates the cloud path with simple credential handling and no
streaming protocol work. This Driver emits completed captions only; that is a
fact about this path, not a rule for other recognition paths.

Revisit if per-segment latency or cost fails real usage.
