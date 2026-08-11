# Local Inference Notes

Research snapshot for the planned single-pass local-recognition phases.
Re-check model, runtime, size, license, and backend support before
implementation or distribution; model capability and the capability exposed
by a particular runtime are not always the same.

## Practical Meaning Of Rust-Native

For this project, local inference is Rust-native when:

- the main app and inference worker are Rust processes;
- the worker is called through a Rust crate or narrow native interface;
- users do not install Python, PyTorch, Conda, or development toolchains;
- Windows runtime libraries and model files are packaged as managed components.

The underlying inference implementation may contain C/C++ and ONNX Runtime.
Requiring every kernel to be written in Rust would substantially reduce model
choice without improving the user-visible product boundary.

[`sherpa-onnx`](https://github.com/k2-fsa/sherpa-onnx) is the first runtime to
evaluate. It has an official
[`sherpa-onnx` Rust crate](https://docs.rs/sherpa-onnx/latest/sherpa_onnx/),
Windows x64 support, online and offline ASR, VAD, and official
[Rust examples](https://github.com/k2-fsa/sherpa-onnx/tree/master/rust-api-examples)
and [Tauri examples](https://github.com/k2-fsa/sherpa-onnx/tree/master/tauri-examples).

Local inference remains out of process even with a Rust API. Large native
dependencies, model crashes, GPU runtime failures, and model replacement must
not destabilize the Tauri process.

## Single-Pass Shapes

The first local implementation does not use two-pass. One recognition attempt
loads one recognition model whose concrete Driver has one of two useful
shapes.

### Bounded recognition

```text
continuous microphone capture
  -> VAD / natural boundary / hard maximum
  -> completed audio span
  -> local recognizer
  -> completed caption snapshot
```

This supports Completed publication. It does not honestly provide Live text
inside the still-open span.

### Streaming recognition

```text
audio frames
  -> online recognizer
  -> ongoing full-text snapshots
  -> endpoint
  -> completed snapshot
  -> reset stream
```

This supports both Completed and Live. Completed simply ignores ongoing
snapshots until the endpoint closes the caption unit; it does not require a
second model.

Every path uses natural-boundary-first segmentation with an application hard
maximum so uninterrupted speech cannot grow without bound. Exact target and
hard-limit durations require real speech benchmarks.

## First Local Model Candidates

Sizes below are approximate model-file sizes from current sherpa-onnx packages,
not total RAM, VRAM, download, or packaged runtime size.

| Candidate | Current sherpa-onnx behavior | Why evaluate it | Important limitation | Initial role |
|---|---|---|---|---|
| [SenseVoiceSmall int8](https://k2-fsa.github.io/sherpa/onnx/sense-voice/pretrained.html) | Bounded recognition after VAD; completed text | Roughly 228 MB, fast non-autoregressive CPU path, strong Chinese and mixed-language candidate | No true ongoing partials on this path; language/emotion/event tokens must be normalized and filtered | First worker/Completed implementation |
| [Streaming Paraformer bilingual int8](https://k2-fsa.github.io/sherpa/onnx/pretrained_models/online-paraformer/paraformer-models.html) | Native online ongoing snapshots plus endpoint completion | Roughly 226 MB; Chinese/English and code-switching candidate with low latency | The public package does not support timestamps; quality relative to newer models requires measurement | First Live candidate |
| [Streaming Zipformer](https://k2-fsa.github.io/sherpa/onnx/pretrained_models/online-transducer/zipformer-transducer-models.html) | Native online ongoing snapshots plus endpoint completion | Small variants can be very light; useful comparison for low-resource machines | The smallest model may trade substantial accuracy for size and speed | Low-resource Live candidate |

The implementation order is not a permanent product default or quality
ranking. SenseVoice comes first because it exercises the simplest bounded
worker path. Paraformer and Zipformer are tested independently for real Live
behavior. The product must not automatically swap models when a user changes
publication mode; it explains incompatibility and lets the user preserve either
the model or the desired experience.

### Possible later packaging profiles

If benchmarks and the distribution decision justify several downloadable model
packs, possible user-facing profiles are:

- a small low-resource pack;
- a low-latency Live pack;
- a larger accuracy-focused pack.

These are packaging hypotheses, not defaults or guarantees. Model names,
download sizes, supported modes, and measured hardware costs must remain visible
so a profile label never silently chooses a different recognizer.

## Later Accuracy And Language Candidates

After the first three paths work, benchmark accuracy-focused candidates such as
FireRedASR2 CTC/AED, Fun-ASR-Nano, Qwen3-ASR, offline Paraformer, Whisper, and
Omnilingual ASR. Their model claims, runtime streaming behavior, quantization,
license, and resource cost must be evaluated separately.

In particular, a model repository advertising streaming does not prove that the
selected sherpa-onnx Rust integration exposes streaming. Record capability for
the full model/runtime/backend combination.

## Normalized Recognition Driver Contract

Do not build one implementation full of model-name branches. The worker exposes
a small recognition-attempt seam and each behaviorally distinct model family
owns a concrete driver.

Every driver emits full snapshots containing at least:

- runtime-generation and caption-unit identity;
- source lane;
- monotonic revision;
- full current text;
- ongoing or completed state;
- detected language, timestamps, and other metadata only when reliable;

Recognition path, runtime, and effective-backend identity belong in Runtime
Control and diagnostics. They are not duplicated as unchecked free-form
`provider` / `model` strings in every UI-facing caption snapshot.

Raw deltas, SenseVoice tags, endpoint calls, VAD buffers, and recognizer reset
rules remain inside the driver. Same-family sizes may share a driver when their
lifecycle behavior is identical.

Provider-specific stable-prefix information may guide driver internals, but it
does not create a public `stable` caption state. Two-pass authority is not part
of the first worker contract.

## CPU And Windows GPU Backends

### CPU

CPU is the first compatibility implementation because the official Rust crate
defaults to Windows x64 CPU libraries and is simplest to package and diagnose.
This is engineering order, not a claim that CPU is best while VRChat is running.

### NVIDIA CUDA

sherpa-onnx publishes Windows x64 CUDA builds, including current CUDA 12.x /
cuDNN 9 packages. The Rust crate does not automatically choose them: CUDA uses
shared libraries, matching runtime DLLs, and an explicitly selected provider.
See the official [Windows CUDA build and package notes](https://k2-fsa.github.io/sherpa/onnx/install/windows/build-cuda.html)
and [ONNX Runtime CUDA requirements](https://onnxruntime.ai/docs/execution-providers/CUDA-ExecutionProvider.html).

`provider = "cuda"` does not guarantee that every operation remains on GPU or
that an int8 model is faster than CPU. Unsupported nodes may execute on CPU and
data transfer can dominate a small model. Benchmark each supported
model/quantization/runtime/backend combination while VRChat is running.

### Other Windows GPU Backends

This research snapshot has not verified a supported sherpa-onnx Rust
distribution path for DirectML. DirectML or another non-CUDA backend may be
investigated later for AMD/Intel/NVIDIA Windows GPUs, but it must not be listed
as supported until an official or maintainable runtime path, model coverage,
packaging, and real-machine behavior are confirmed. TensorRT adds substantial
version and API complexity and is not in the current product plan.

### User preference and effective backend

The product stores one global preference:

- CPU;
- prefer NVIDIA GPU (CUDA).

There is no automatic performance selector now. Missing preference uses CPU.
The worker plan also records the effective backend:

- if the chosen model/runtime does not support CUDA, use CPU and show why;
- keep the global CUDA preference while showing the current model's GPU option
  unavailable; do not rewrite the saved preference merely because one model
  lacks support;
- on hardware without a compatible NVIDIA GPU, disable the CUDA choice; if a
  transferred configuration requests it, resolve CPU and show the reason;
- if CUDA initialization fails before the runtime generation starts, CPU may be used only
  with a clear visible warning;
- if a running worker crashes, end the runtime generation and let the user explicitly
  retry the same backend or choose CPU;
- never switch backend during an active runtime generation;
- never turn a local failure into cloud upload without explicit user action.

Aggregate CPU/GPU percentages are insufficient for recommendations. Measure at
least first useful text, speech-end completion, real-time factor, CPU/RAM,
GPU/VRAM, VRChat CPU/GPU frame time, dropped/reprojected frames where available,
temperature/throttling, and long-running stability.

## Component And Model Distribution

Distribution shape is not decided. Evaluate these options before implementing
the first local model:

1. bundle one CPU runtime and model with the installer;
2. keep the base installer small and download the first model on demand;
3. build a managed component catalog for several runtime/model packs.

Compare installer and update size, offline installation, repair/removal,
license notices, CDN cost, signing, partial-download recovery, and the support
burden of matching CPU/CUDA runtimes to models. Do not commit the project to a
full component manager merely because it is the most extensible option.

Whichever option is selected must verify model/runtime integrity, record
version and license, run a smoke test before selection, and unload the old
recognizer before loading a new one. If a managed catalog is selected, prefer
separate runtime and model packs, temporary downloads, verification before
unpacking, explicit compatibility constraints, and a catalog signature that is
independent from the main App updater.

Possible managed-component flow, conditional on that decision:

```text
user selects a model component
  -> show size, languages, behavior, license, and supported backends
  -> download and verify
  -> install runtime/model packs
  -> smoke-test each advertised backend
  -> mark usable
  -> user chooses model and global backend preference
```

## Worker Supervision

```text
runtime coordinator
  -> start active Recognition Module
  -> submit continuous owned mono audio
local recognition driver
  -> start worker
  -> health check
  -> load one model on one effective backend
  -> create a bounded or streaming recognition attempt
  -> exchange bounded audio/snapshot IPC
  -> emit normalized caption and lifecycle signals
  -> stop, unload, and report health
```

- runtime uses the same active Recognition Module boundary as the cloud path; worker
  commands, resampling windows, model-native frames, and backend state stay
  inside the local driver
  ([ADR 0026](../adr/0026-recognition-modules-own-attempt-execution.md));
- audio admission is bounded by represented duration plus a frame safety
  ceiling; IPC queues are independently bounded for the selected runtime and
  model;
- capture never waits for inference;
- queued audio carries the active attempt epoch; after a worker crash or
  retirement it is discarded and can never enter a replacement attempt;
- if streaming inference falls far behind, fail or report an explicit audio
  gap according to the selected path rather than replaying very old captions;
- model load errors, missing DLLs, GPU memory exhaustion, worker crashes, and
  unsupported combinations have separate diagnostic codes;
- user-facing errors have concise summaries and expandable/copyable technical
  detail;
- a worker crash never destabilizes the main app and never triggers silent
  restart or backend/cloud switching.

## Local Translation

Local translation is a separate later component and may load a translation
model alongside the one active recognition model. That resource cost can be similar to
running another AI model even though it is not two-pass recognition.

The earlier research brief named the following starting points. They are not
verified selections; re-check current model versions, language quality,
licenses, runtime support, and redistribution terms before implementation:

- local LLM translation through `llama.cpp` and GGUF model packs, with CPU as a
  baseline and Vulkan or CUDA only where the chosen runtime/model combination
  is verified;
- TranslateGemma- and Qwen-family translation-capable models as LLM candidates;
- CTranslate2 with OPUS-MT or M2M100 as lower-latency, more deterministic NMT
  candidates;
- NLLB-class models only after a strict license and redistribution review.

LLM output needs constraints and post-processing so it does not explain,
rewrite, or add material. Classic NMT may be a useful light mode even if its
conversational CJK quality is lower. These are hypotheses for a benchmark, not
product tiers.

Do not assume all translation models are small. Evaluate:

- model/runtime/backend size and resource use;
- whether input is completed source, revising source, or audio;
- whether target output is one completion, token streaming after a completed
  source, or simultaneous translation during speech;
- deterministic translation versus explanation/rewrite risk;
- language coverage and distribution license.

The first translation implementation uses completed source units. Provider-
native ongoing or simultaneous target revisions are evaluated only in the later
conditional Live-translation phase. Repeated translation of every unstable ASR
revision is deferred. The public treatment of token streaming from a fixed
source is provisional: it may feed ongoing target snapshots, but the UI must
say that translation starts after the pause and user testing must determine
whether that experience should be called Live.

## Two-Pass

Two-pass is not part of the first local phase. It would load a low-latency
recognizer and a separate correction recognizer over correlated audio, with
distinct authority and failure rules. Consider it only after:

- single-pass Completed and Live are stable;
- model and component management is complete;
- translation and Chatbox behavior are validated;
- benchmarks demonstrate a material accuracy improvement worth the added RAM,
  CPU/GPU contention, latency, and complexity.

If it is ever added, it is explicit opt-in and never an automatic default.
