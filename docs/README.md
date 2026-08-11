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
| [research/](./research/) | External facts, measurements, experiments, candidate evaluations, and evidence-backed integration constraints. Product intent belongs in `product.md`, accepted decisions in ADRs, and implementation status in the roadmap. |
| [contracts/](../contracts/) | Serialized fixtures and manifests that pin Rust/TypeScript compatibility boundaries. |
| Code, tests, and configuration | Exact current mechanics, commands, versions, module names, protocol handling, and executable behavior. |

When a detail is easy to verify in code or configuration and does not explain a
project-wide boundary or rationale, keep it there. Prose should explain the
product, boundaries, decisions, and evidence that readers cannot recover
quickly from an implementation file.

## Compatibility baseline

Compatibility cutoff and versioning rules live in
[`contracts/README.md`](../contracts/README.md). Review them before changing
persisted configuration or a cross-language contract; this project does not use
the first packaged release as an implicit substitute for those rules.

## Research library

- [VRChat Chatbox reference](./research/vrchat-chatbox-reference.md) records
  the canonical evidence and derived constraints used by the Chatbox
  implementation, including its measured pacing and layout model.
- [Local recognition evaluation](./research/local-recognition-evaluation.md)
  compares model, runtime, backend, packaging, and benchmark candidates for the
  future local path.
- [Tauri build integration](./research/tauri-build-integration.md) records the
  project's frontend, Rust, Tauri hook, and packaging boundaries, including the
  reasons behind the current build commands.

Research findings should name their evidence and review date when the facts can
age. Once research produces an accepted direction, record the decision in an
ADR and the implementation status in the roadmap instead of turning the
research note into both.
