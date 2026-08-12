# Cloud Translation Evaluation

Official OpenAI facts were reviewed on 2026-08-11. No authenticated provider
request, translation-quality study, latency measurement, or native
Windows/VRChat evaluation was performed for this note.

## Verified provider facts

- The Responses API accepts text through `POST /responses`; the official URL is
  `https://api.openai.com/v1/responses`
  ([create a response](https://developers.openai.com/api/reference/resources/responses/methods/create)).
- `gpt-5.6-luna` supports text input/output, the Responses API, streaming, and
  `reasoning.effort: none`
  ([model page](https://developers.openai.com/api/docs/models/gpt-5.6-luna),
  [model guidance](https://developers.openai.com/api/docs/guides/latest-model)).
- Responses are non-streaming by default; `stream: true` uses server-sent
  events. Raw HTTP parsing must handle typed output items rather than depend on
  an SDK-only `output_text` helper
  ([streaming](https://developers.openai.com/api/docs/guides/streaming-responses),
  [response schema](https://developers.openai.com/api/reference/resources/responses/methods/create)).
- OpenAI treats API keys as secrets and warns against exposing them in client
  code. An OS credential store is this desktop project's risk trade-off, not an
  OpenAI-recommended server-side key-management pattern
  ([authentication](https://developers.openai.com/api/reference/overview#authentication)).
- `store: false` disables later Response retrieval; it is not Zero Data
  Retention. Default abuse-monitoring logs may retain prompts and responses for
  up to 30 days
  ([data controls](https://developers.openai.com/api/docs/guides/your-data#default-usage-policies-by-endpoint)).

These capability statements do not establish this project's translation
quality, latency, or Custom endpoint behavior. The accepted request profile and
trust choices live in
[ADR 0021](../adr/0021-use-openai-responses-for-completed-translation.md) and
[ADR 0015](../adr/0015-cloud-connections-honor-explicit-routes-and-endpoints.md).

## Evidence still required

- Run redacted Official and Custom smoke tests for both target languages,
  verifying the fixed request fields, typed-output parser, separate credentials,
  API-base-URL joining, redirect rejection, and fail-closed incompatibility.
- Have bilingual reviewers evaluate English-to-Simplified-Chinese and
  Simplified-Chinese-to-English samples containing names, numbers, punctuation,
  mixed-language text, and Chatbox-relevant Unicode.
- Measure queue, provider, and terminal latency against the provisional
  12-second admission deadline in
  [Issue #11](https://github.com/Harry-Jing/vrc-live-caption/issues/11), including
  saturation, bounded sizes, retry, cancellation, failure, and Stop.
- Verify that the UI discloses the selected recipient of Source text and that
  diagnostics contain no credentials, caption text, provider bodies, or full
  Custom URLs.
- Run the native Windows/VRChat matrix for both target directions,
  Translation-only and Bilingual output, pagination, sustained backlog,
  capture/recognition continuity, and remote-observer readability.

All gates remain pending. Re-check the model entry, request schema,
authentication guidance, and data controls before implementation and again
whenever those provider surfaces or the accepted request profile change.
