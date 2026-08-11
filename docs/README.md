# Project documentation

This index identifies the audience and authority of each project document. A
fact should have one primary home; other documents should link to it instead of
maintaining another copy.

## Where to start

| If you want to... | Start here |
|---|---|
| Understand or try the project | [Project README](../README.md) |
| Contribute a change | [CONTRIBUTING.md](../CONTRIBUTING.md) |
| Understand product intent and user guarantees | [product.md](./product.md) |
| See what is implemented and what comes next | [roadmap.md](./roadmap.md) |
| Work on runtime boundaries or data flow | [architecture.md](./architecture.md), then the relevant [ADR](./adr/) |
| Use project terminology consistently | [CONTEXT.md](../CONTEXT.md) |
| Change Chatbox layout, pacing, clipping, or OSC behavior | [VRChat Chatbox reference](./research/vrchat-chatbox-reference.md) |

## Sources of truth

| Source | Audience and authority |
|---|---|
| [README.md](../README.md) | Public introduction: maturity, current user-visible capability, privacy, and the shortest source-run path. |
| [CONTRIBUTING.md](../CONTRIBUTING.md) | Human contribution workflow: issue-first coordination, setup, checks, pull requests, and security hygiene. |
| [CONTEXT.md](../CONTEXT.md) | Project glossary. It defines shared domain terms, not lifecycle specifications or implementation status. |
| [product.md](./product.md) | Durable product intent, user choices, guarantees, and non-goals. |
| [roadmap.md](./roadmap.md) | The only implementation-status and sequencing record. |
| [architecture.md](./architecture.md) | Runtime boundaries, ownership, data flow, and cross-boundary invariants. |
| [adr/](./adr/) | Accepted decisions whose trade-offs and reasoning need to survive the implementation. |
| [research/](./research/) | External facts, measurements, experiments, candidate evaluations, and evidence-backed integration constraints. |
| [contracts/](../contracts/) | Shared fixtures and manifests plus the V1 cutoff and versioning rules; the cutoff is the merge to `main`, not the first packaged release. |
| Code, tests, and configuration | Exact current mechanics, commands, versions, module names, protocol handling, and executable behavior. |

## Research library

- [Cloud translation evaluation](./research/cloud-translation-evaluation.md)
  records OpenAI evidence and the provider, language, and native validation
  still required by ADR 0021.
- [VRChat Chatbox reference](./research/vrchat-chatbox-reference.md) records
  the canonical evidence and derived constraints used by the Chatbox
  implementation, including its measured pacing and layout model.
- [Local recognition evaluation](./research/local-recognition-evaluation.md)
  compares model, runtime, backend, packaging, and benchmark candidates for the
  future local path.
- [Tauri build integration](./research/tauri-build-integration.md) records the
  project's frontend, Rust, Tauri hook, and packaging boundaries, including the
  reasons behind the current build commands.

Add a review date to findings that can become stale; promote accepted decisions
to ADRs and implementation progress to the roadmap.
