# Local Inference Notes

Source material for the local STT phase (roadmap Phase 6) and for the open
question about local translation. Extracted in June 2026 from the retired
Chinese rewrite brief. Candidate lists are starting points, not verified
choices: every engine and model here must be re-validated during engine
research, including license review and resource usage measured on a Windows
machine that is also running VRChat.

## Local STT Candidates

Local STT splits into two roles that may use different engines.

### Low-latency online pass

Feeds App preview partials, future incoming caption, and future
interpretation. Optimize for speed and stability, not peak accuracy.

- sherpa-onnx streaming Zipformer
- sherpa-onnx online Paraformer
- other light models with stable partial output

### High-quality final pass

Feeds Chatbox output, history, and translation input.

- Fun-ASR-Nano ONNX: high-quality Chinese, English, and Japanese candidate,
  including Chinese dialects
- SenseVoice: Chinese and mixed Chinese-English candidate
- Paraformer offline: Chinese baseline candidate
- whisper.cpp: multilingual fallback and cross-GPU option

### Packaging tiers

Do not ship one all-in-one bundle. Plan separate packs:

- light pack: small download, CPU-friendly
- high-quality pack: larger models for accuracy-focused users
- low-latency pack: App-internal real-time partials

## Local Translation Candidates

The primary direction is LLM-based translation:

- llama.cpp as the native LLM runtime, with GGUF model packs
- CPU as the baseline backend; Vulkan or CUDA as optional acceleration
- candidate models to evaluate: TranslateGemma, Qwen
- output must be constrained by prompt and post-processing so the model does
  not explain, rewrite, or add extra content

Traditional NMT is a complement:

- CTranslate2 with OPUS-MT or M2M100 for low-latency deterministic translation
- NLLB-class models need a strict license review before any distribution
- classic NMT may underperform on CJK conversational speech but can serve as a
  light mode

## Component And Model Distribution

Local inference ships as managed components, never inside the main installer.

Principles:

- runtime packs and model packs are separate components
- every component carries a manifest: version, hash or signature, license,
  compatibility constraints, and a self-test
- the main app updates through the app updater; components update through a
  signed component catalog, and the two channels stay separate
- local inference failures fall back to cloud or CPU with a clear diagnostic

Install flow sketch:

```text
user picks a local STT or translation component
  -> app fetches the component catalog
  -> candidates filtered by OS / arch / GPU / RAM
  -> user confirms size, license, and purpose
  -> download to a temporary directory
  -> verify hash / signature / manifest
  -> unpack into app data
  -> run doctor / smoke test
  -> mark usable
  -> optional benchmark to pick the default backend
```

## Worker Supervision

```text
Rust runtime
  -> start worker
  -> health check
  -> load model
  -> send audio/text requests
  -> receive normalized results
  -> restart or fall back on failure
```

- a worker crash must not destabilize the main app
- GPU runtime errors fall back to CPU or cloud
- model load failures, GPU memory exhaustion, and missing licenses produce
  user-readable diagnostics
- worker logs flow into the App diagnostics surface
