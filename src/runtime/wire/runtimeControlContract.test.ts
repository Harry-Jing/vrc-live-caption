import { describe, expect, test } from "vitest";
import runtimeControlFixture from "../../../contracts/runtime-control-snapshot-v1.json?raw";
import {
  decodeRuntimeControlSnapshot,
  RuntimeControlContractError,
} from "./runtimeControlContract";

const fixtureJson = JSON.parse(runtimeControlFixture) as unknown;
const completePayload = decodeRuntimeControlSnapshot(fixtureJson);

describe("decodeRuntimeControlSnapshot", () => {
  test("decodes the shared Rust-serialized fixture", () => {
    expect(decodeRuntimeControlSnapshot(fixtureJson)).toEqual(fixtureJson);
  });

  test("rejects the old runtime control contract version", () => {
    expect(() =>
      decodeRuntimeControlSnapshot({
        ...(fixtureJson as object),
        contractVersion: 4,
      }),
    ).toThrow(
      "Invalid runtime control payload at $.contractVersion: expected 1.",
    );
  });

  test("rejects the old app config schema version", () => {
    expect(() =>
      decodeRuntimeControlSnapshot({
        ...completePayload,
        desired: {
          ...completePayload.desired,
          config: { ...completePayload.desired.config, schemaVersion: 4 },
        },
      }),
    ).toThrow(
      "Invalid runtime control payload at $.desired.config.schemaVersion: expected 1.",
    );
  });

  test.each([
    {
      name: "empty expected-language list",
      expectedLanguages: [] as string[],
      path: "$.desired.config.recognition.expectedLanguages",
    },
    {
      name: "blank expected-language hint",
      expectedLanguages: ["en", "   "],
      path: "$.desired.config.recognition.expectedLanguages[1]",
    },
    {
      name: "case-insensitive duplicate expected-language hint",
      expectedLanguages: ["en", " EN "],
      path: "$.desired.config.recognition.expectedLanguages",
    },
  ])("rejects $name", ({ expectedLanguages, path }) => {
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        config: {
          ...completePayload.desired.config,
          recognition: {
            ...completePayload.desired.config.recognition,
            expectedLanguages,
          },
        },
      },
    };

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(path);
  });

  test("decodes an active generation while its recognition service reconnects", () => {
    const payload = {
      ...completePayload,
      runtimeStatus: { status: "reconnecting", timestampMs: 901 },
      generation: completePayload.generation
        ? { ...completePayload.generation, phase: "reconnecting" }
        : null,
    };

    expect(decodeRuntimeControlSnapshot(payload)).toMatchObject({
      runtimeStatus: { status: "reconnecting" },
      generation: { phase: "reconnecting" },
    });
  });

  test("decodes an unconfigured service credential without configured-only fields", () => {
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        credentials: [{ state: "unconfigured", id: "openai" }],
      },
    };

    expect(decodeRuntimeControlSnapshot(payload).desired.credentials).toEqual([
      { state: "unconfigured", id: "openai" },
    ]);
  });

  test("decodes an unavailable service credential with a structured failure", () => {
    const failure = {
      code: "config.secret_failed",
      message: "System credential store is unavailable.",
    };
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        credentials: [{ state: "unavailable", id: "openai", failure }],
      },
    };

    expect(decodeRuntimeControlSnapshot(payload).desired.credentials).toEqual([
      { state: "unavailable", id: "openai", failure },
    ]);
  });

  test("rejects credential fields that do not belong to the tagged state", () => {
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        credentials: [
          {
            state: "unavailable",
            id: "openai",
            failure: {
              code: "config.secret_failed",
              message: "System credential store is unavailable.",
            },
            configured: false,
          },
        ],
      },
    };

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(
      "$.desired.credentials[0].configured",
    );
  });

  test("preserves an incompatible desired publication request", () => {
    const incompatiblePublication = {
      state: "incompatible",
      requestedMode: "live",
      selectedLanes: ["source"],
      reason: { reason: "modeUnsupported", lanes: ["source"] },
      supportedModes: ["completed"],
    };
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        captionPipelinePlan: {
          ...completePayload.desired.captionPipelinePlan,
          publication: incompatiblePublication,
        },
      },
      generation: null,
    };

    expect(decodeRuntimeControlSnapshot(payload)).toMatchObject({
      desired: {
        config: { publication: { mode: "live" } },
        captionPipelinePlan: { publication: incompatiblePublication },
      },
      generation: null,
    });
  });

  test("decodes completed and unit-based Live timings", () => {
    const cases = [
      { mode: "completed", timing: { timing: "completed" } },
      {
        mode: "live",
        timing: { timing: "liveUnit", observationWindowMs: 1000 },
      },
    ] as const;

    for (const { mode, timing } of cases) {
      const payload = {
        ...completePayload,
        desired: {
          ...completePayload.desired,
          config: {
            ...completePayload.desired.config,
            publication: { mode },
          },
          captionPipelinePlan: {
            ...completePayload.desired.captionPipelinePlan,
            publication: {
              state: "compatible",
              mode,
              timing,
              selectedLanes: ["source"],
            },
          },
        },
        generation: null,
      };

      expect(
        decodeRuntimeControlSnapshot(payload).desired.captionPipelinePlan
          .publication,
      ).toEqual(payload.desired.captionPipelinePlan.publication);
    }
  });

  test.each([
    ["inputShape", "completedAudioUnits"],
    ["captionBoundaryOwner", "provider"],
    ["captionBoundaryOwner", "none"],
    ["unitBehavior", "unitless"],
  ] as const)("rejects removed recognition %s value %s", (field, value) => {
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        captionPipelinePlan: {
          ...completePayload.desired.captionPipelinePlan,
          recognition: {
            ...completePayload.desired.captionPipelinePlan.recognition,
            [field]: value,
          },
        },
      },
    };

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(
      `$.desired.captionPipelinePlan.recognition.${field}`,
    );
  });

  test("rejects removed unitless Live timing", () => {
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        captionPipelinePlan: {
          ...completePayload.desired.captionPipelinePlan,
          publication: {
            ...completePayload.desired.captionPipelinePlan.publication,
            timing: { timing: "liveUnitless", firstNonEmptyDelayMs: 1000 },
          },
        },
      },
    };

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(
      "$.desired.captionPipelinePlan.publication.timing.timing",
    );
  });

  test("rejects an incompatible plan on an installed generation", () => {
    const generation = completePayload.generation;
    if (generation === null) {
      throw new Error("Fixture must contain a runtime generation.");
    }
    const payload = {
      ...completePayload,
      generation: {
        ...generation,
        captionPipelinePlan: {
          ...generation.captionPipelinePlan,
          publication: {
            state: "incompatible",
            requestedMode: "live",
            selectedLanes: ["source"],
            reason: { reason: "modeUnsupported", lanes: ["source"] },
            supportedModes: ["completed"],
          },
        },
      },
    };

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(
      "Invalid runtime control payload at $.generation.captionPipelinePlan.publication.state: installed generations require a compatible publication plan.",
    );
  });

  test.each([
    [
      "a pre-baseline contract version",
      { ...completePayload, contractVersion: 3 },
      "$.contractVersion",
    ],
    [
      "the removed stt config shape",
      {
        ...completePayload,
        desired: {
          ...completePayload.desired,
          config: {
            ...completePayload.desired.config,
            recognition: undefined,
            stt: {
              provider: "openai",
              languages: ["en"],
              model: "gpt-transcribe",
            },
          },
        },
      },
      "$.desired.config.stt",
    ],
    [
      "the removed partial-preview field",
      {
        ...completePayload,
        desired: {
          ...completePayload.desired,
          config: {
            ...completePayload.desired.config,
            ui: { showPartial: true },
          },
        },
      },
      "$.desired.config.ui.showPartial",
    ],
    [
      "an arbitrary recognition path",
      {
        ...completePayload,
        desired: {
          ...completePayload.desired,
          config: {
            ...completePayload.desired.config,
            recognition: {
              ...completePayload.desired.config.recognition,
              path: "openai/removed-model",
            },
          },
        },
      },
      "$.desired.config.recognition.path",
    ],
    [
      "unknown nested fields",
      {
        ...completePayload,
        desired: { ...completePayload.desired, unexpected: true },
      },
      "$.desired.unexpected",
    ],
    [
      "a non-positive Live delay",
      {
        ...completePayload,
        desired: {
          ...completePayload.desired,
          captionPipelinePlan: {
            ...completePayload.desired.captionPipelinePlan,
            publication: {
              ...completePayload.desired.captionPipelinePlan.publication,
              timing: { timing: "liveUnit", observationWindowMs: 0 },
            },
          },
        },
      },
      "$.desired.captionPipelinePlan.publication.timing.observationWindowMs",
    ],
    [
      "a plan that disagrees with the configured mode",
      {
        ...completePayload,
        desired: {
          ...completePayload.desired,
          config: {
            ...completePayload.desired.config,
            publication: { mode: "completed" },
          },
        },
      },
      "$.desired.captionPipelinePlan.publication.mode",
    ],
  ])("rejects %s", (_name, payload, path) => {
    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(path);
  });

  test("preserves the runtime control contract error type", () => {
    expect(() => decodeRuntimeControlSnapshot(null)).toThrow(
      RuntimeControlContractError,
    );
  });
});
