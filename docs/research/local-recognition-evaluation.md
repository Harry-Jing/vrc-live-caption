# Local Recognition Evaluation

External model and runtime facts were last reviewed on 2026-07-15. Re-check every
artifact, version, size, license, backend, and redistribution claim before
implementation or distribution.

## Scope

This note compares local speech-recognition shapes, runtimes, models, backends,
distribution options, and benchmark criteria. It does not define the runtime
contract or product policy:

- the process boundary is [ADR 0018](../adr/0018-keep-local-inference-out-of-process.md);
- backend choice is [ADR 0019](../adr/0019-users-choose-the-local-backend.md);
- Recognition Module ownership is
  [ADR 0014](../adr/0014-recognition-modules-own-path-execution.md);
- implementation order and status are in the [roadmap](../roadmap.md).

For this project, a local path is Rust-native when the app and worker are Rust
processes, users install no Python/PyTorch/Conda toolchain, and native runtime
libraries and model files are managed application components. The underlying
inference engine may use C, C++, or ONNX Runtime.

## Recognition shapes

### Bounded recognition

```text
continuous audio
  -> speech boundary
  -> completed audio span
  -> local recognizer
  -> completed caption snapshot
```

This shape supports Completed publication. It does not provide honest Live text
inside an open span.

### Streaming recognition

```text
audio frames
  -> online recognizer
  -> ongoing snapshots
  -> endpoint
  -> completed snapshot
```

This shape can support Completed and Live. Capability belongs to the full
model/runtime/backend combination, not to a model name alone.

## Runtime to evaluate first

[`sherpa-onnx`](https://github.com/k2-fsa/sherpa-onnx) is the first runtime
candidate. It has an official
[`sherpa-onnx` Rust crate](https://docs.rs/sherpa-onnx/latest/sherpa_onnx/),
Windows x64 support, online and offline ASR, VAD, and official
[Rust examples](https://github.com/k2-fsa/sherpa-onnx/tree/master/rust-api-examples)
and [Tauri examples](https://github.com/k2-fsa/sherpa-onnx/tree/master/tauri-examples).

That makes it a practical first integration candidate, not a permanent runtime
selection. The crate version, native artifact matrix, APIs, and licenses must be
pinned during implementation.

## First model candidates

The following sizes are approximate model-file sizes from the packages reviewed
in July 2026. They exclude RAM, VRAM, runtime libraries, packaging overhead, and
download metadata.

| Candidate | Shape | Why evaluate it | Known limitation | Possible role |
|---|---|---|---|---|
| [SenseVoiceSmall int8](https://k2-fsa.github.io/sherpa/onnx/sense-voice/pretrained.html) | bounded | roughly 228 MB; fast CPU-oriented Chinese and mixed-language candidate | no true ongoing partials on this path; tags require normalization | first Completed worker path |
| [Streaming Paraformer bilingual int8](https://k2-fsa.github.io/sherpa/onnx/pretrained_models/online-paraformer/paraformer-models.html) | streaming | roughly 226 MB; Chinese/English and code-switching candidate | reviewed package did not support timestamps; quality needs measurement | first Live candidate |
| [Streaming Zipformer](https://k2-fsa.github.io/sherpa/onnx/pretrained_models/online-transducer/zipformer-transducer-models.html) | streaming | small variants may suit constrained machines | smaller models may trade substantial accuracy for size and speed | low-resource Live comparison |

The evaluation order is not a product ranking. SenseVoiceSmall comes first only
because a bounded worker is the smallest path that tests packaging, isolation,
and Completed output. Streaming candidates must be measured independently.

A repository claim of streaming does not prove that the selected Rust runtime
exposes streaming.

### Additional candidate watchlist

Candidates to re-evaluate include FireRedASR2 CTC/AED, Fun-ASR-Nano, Qwen3-ASR,
offline Paraformer, Whisper, and Omnilingual ASR. This is a discovery list, not a
ranking or compatibility claim; verify the exact model/runtime/backend
combination, exposed streaming behavior, quantization, license and redistribution
terms, and resource cost before selection.

## Windows compute backends

### CPU

CPU is the first compatibility target because the reviewed Rust package defaults
to Windows x64 CPU libraries and is the smallest packaging surface. This is
implementation order, not a performance recommendation while VRChat runs.

### NVIDIA CUDA

The reviewed sherpa-onnx distribution published Windows x64 CUDA builds using
CUDA 12.x and cuDNN 9. CUDA requires matching shared libraries and explicit
provider selection; the Rust crate does not make the application packaging
decision automatically. Re-check the official
[Windows CUDA build notes](https://k2-fsa.github.io/sherpa/onnx/install/windows/build-cuda.html)
and [ONNX Runtime requirements](https://onnxruntime.ai/docs/execution-providers/CUDA-ExecutionProvider.html)
for the versions selected by the project.

`provider = "cuda"` does not guarantee that every operation stays on the GPU or
that an int8 model beats CPU. Unsupported nodes and transfer overhead require a
benchmark for every supported model/runtime/backend combination.

### Other GPU backends

The July 2026 review did not verify a maintainable sherpa-onnx Rust distribution
for DirectML. Do not advertise DirectML or another non-CUDA backend until runtime
artifacts, model coverage, packaging, and real Windows behavior are verified.

## Distribution decision

Choose the first distribution shape before implementation:

1. bundle one CPU runtime and model with the installer;
2. keep the base installer small and download the first component on demand;
3. build a managed catalog for multiple runtime/model packs.

Compare installer and update size, offline installation, repair and removal,
license notices, hosting cost, signing, interrupted-download recovery, and the
support cost of matching models to CPU/CUDA runtimes. Do not build a component
manager only because it is the most extensible option.

Any selected shape must verify integrity, record artifact and license versions,
smoke-test advertised backends, and make an installed component repairable and
removable. A multi-pack catalog additionally needs explicit compatibility rules
and signed metadata independent of the app updater.

## Benchmark plan

Run each candidate with VRChat active on representative Windows hardware. Record:

- English, Chinese, and mixed-speech accuracy;
- first useful text and speech-end completion latency;
- real-time factor and backlog behavior;
- CPU, RAM, GPU, and VRAM use;
- VRChat CPU/GPU frame time and dropped or reprojected frames where available;
- thermal throttling and long-running stability;
- model/runtime/backend download and installed size;
- worker startup, model-load, Stop, crash, and recovery behavior.

Aggregate utilization alone is not a recommendation: frame time, VRAM pressure,
latency, and stability matter while VRChat is running.

## Decision gates

Before the first CPU implementation:

- pin the sherpa-onnx crate and native artifact versions;
- identify exact model artifacts and verify their licenses and redistribution;
- choose the first distribution shape;
- define the Windows benchmark machines and acceptance thresholds.

Before CUDA, local Live, or any default change:

- validate at least one local Live path on native Windows with VRChat;
- validate the packaged CUDA dependency chain on clean machines;
- publish recommendations only from recorded benchmark results;
- revisit the local-default decision against measured quality and resource use.

Local translation is a separate research topic. Create a fresh evaluation when
that roadmap work begins rather than carrying unverified translation-model notes
inside the recognition decision.
