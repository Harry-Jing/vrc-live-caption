import { describe, expect, test } from "vitest";
import runtimeControlFixture from "../../contracts/runtime-control-snapshot-v3.json?raw";
import { decodeRuntimeControlSnapshotV3 } from "./runtimeControlContract";

const completePayload = {
  contractVersion: 3,
  revision: 9,
  runtime: {
    status: "running",
    message: "Runtime is running",
    timestampMs: 900,
  },
  desired: {
    revision: 4,
    config: {
      schemaVersion: 3,
      audio: { inputDeviceId: null },
      stt: {
        provider: "openai",
        languages: ["zh", "en"],
        model: "gpt-live-transcribe",
      },
      osc: { host: "127.0.0.1", port: 9000, enabled: true },
      publication: { mode: "live" },
      ui: { showPartial: true },
    },
    runtimePlan: {
      recognition: {
        path: "openAiGptLiveTranscribe",
        inputShape: "continuousAudioFrames",
        boundaryOwner: "application",
        unitBehavior: "unitBased",
        lanes: [
          {
            lane: "source",
            updates: "ongoingAndCompleted",
            revisions: "revisableFullSnapshot",
          },
        ],
      },
      publication: {
        state: "ready",
        mode: "live",
        policy: { policy: "liveUnit", observationWindowMs: 1000 },
        selectedLanes: ["source"],
      },
    },
    providerSecrets: [
      {
        provider: "openai",
        configured: true,
        storage: "systemCredentialStore",
        displaySuffix: "abcd",
        error: null,
      },
    ],
  },
  session: {
    generation: 3,
    phase: "running",
    startedFromConfigRevision: 4,
    selected: {
      audio: { inputDeviceId: null },
      stt: {
        provider: "openai",
        languages: ["zh", "en"],
        model: "gpt-live-transcribe",
      },
      osc: { host: "127.0.0.1", port: 9000, enabled: true },
      publication: { mode: "live" },
    },
    runtimePlan: {
      recognition: {
        path: "openAiGptLiveTranscribe",
        inputShape: "continuousAudioFrames",
        boundaryOwner: "application",
        unitBehavior: "unitBased",
        lanes: [
          {
            lane: "source",
            updates: "ongoingAndCompleted",
            revisions: "revisableFullSnapshot",
          },
        ],
      },
      publication: {
        state: "ready",
        mode: "live",
        policy: { policy: "liveUnit", observationWindowMs: 1000 },
        selectedLanes: ["source"],
      },
    },
    credential: null,
    chatbox: {
      state: "ready",
      host: "127.0.0.1",
      port: 9000,
    },
    uploadsMicrophoneAudio: true,
  },
  pendingChanges: [],
};

describe("runtime control contract", () => {
  test("decodes the shared Rust-serialized V3 fixture", () => {
    const fixture = JSON.parse(runtimeControlFixture) as unknown;

    expect(decodeRuntimeControlSnapshotV3(fixture)).toEqual(fixture);
  });

  test("decodes a complete authoritative V3 snapshot", () => {
    expect(decodeRuntimeControlSnapshotV3(completePayload)).toEqual(
      completePayload,
    );
  });

  test("decodes an active session while its provider connection is reconnecting", () => {
    const payload = structuredClone(completePayload);
    payload.runtime.status = "reconnecting";
    payload.runtime.message = "Reconnecting speech recognition";
    payload.session.phase = "reconnecting";

    expect(decodeRuntimeControlSnapshotV3(payload)).toMatchObject({
      runtime: { status: "reconnecting" },
      session: { phase: "reconnecting" },
    });
  });

  test("decodes an incompatible desired plan without rewriting its mode", () => {
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
        runtimePlan: {
          ...completePayload.desired.runtimePlan,
          publication: incompatiblePublication,
        },
      },
      session: null,
    };

    expect(decodeRuntimeControlSnapshotV3(payload)).toMatchObject({
      desired: {
        config: { publication: { mode: "live" } },
        runtimePlan: { publication: incompatiblePublication },
      },
      session: null,
    });
  });

  test("decodes completed and unit-based Live policies", () => {
    const cases = [
      {
        mode: "completed",
        policy: { policy: "completed" },
      },
      {
        mode: "live",
        policy: { policy: "liveUnit", observationWindowMs: 1000 },
      },
    ];

    for (const { mode, policy } of cases) {
      const payload = {
        ...completePayload,
        desired: {
          ...completePayload.desired,
          config: {
            ...completePayload.desired.config,
            publication: { mode },
          },
          runtimePlan: {
            ...completePayload.desired.runtimePlan,
            publication: {
              state: "ready",
              mode,
              policy,
              selectedLanes: ["source"],
            },
          },
        },
        session: null,
      };

      expect(
        decodeRuntimeControlSnapshotV3(payload).desired.runtimePlan.publication,
      ).toEqual(payload.desired.runtimePlan.publication);
    }
  });

  test.each([
    ["inputShape", "completedAudioUnits"],
    ["boundaryOwner", "provider"],
    ["boundaryOwner", "none"],
    ["unitBehavior", "unitless"],
  ] as const)("rejects removed recognition %s value %s", (field, value) => {
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        runtimePlan: {
          ...completePayload.desired.runtimePlan,
          recognition: {
            ...completePayload.desired.runtimePlan.recognition,
            [field]: value,
          },
        },
      },
    };

    expect(() => decodeRuntimeControlSnapshotV3(payload)).toThrow(
      `$.desired.runtimePlan.recognition.${field}`,
    );
  });

  test("rejects removed ongoing-only lane behavior", () => {
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        runtimePlan: {
          ...completePayload.desired.runtimePlan,
          recognition: {
            ...completePayload.desired.runtimePlan.recognition,
            lanes: [
              {
                ...completePayload.desired.runtimePlan.recognition.lanes[0],
                updates: "ongoingOnly",
              },
            ],
          },
        },
      },
    };

    expect(() => decodeRuntimeControlSnapshotV3(payload)).toThrow(
      "$.desired.runtimePlan.recognition.lanes[0].updates",
    );
  });

  test("rejects removed unitless Live policy", () => {
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        runtimePlan: {
          ...completePayload.desired.runtimePlan,
          publication: {
            ...completePayload.desired.runtimePlan.publication,
            policy: { policy: "liveUnitless", firstNonEmptyDelayMs: 1000 },
          },
        },
      },
    };

    expect(() => decodeRuntimeControlSnapshotV3(payload)).toThrow(
      "$.desired.runtimePlan.publication.policy.policy",
    );
  });

  test("rejects an incompatible plan on an installed session", () => {
    const payload = {
      ...completePayload,
      session: {
        ...completePayload.session,
        runtimePlan: {
          ...completePayload.session.runtimePlan,
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

    expect(() => decodeRuntimeControlSnapshotV3(payload)).toThrow(
      "Invalid runtime control payload at $.session.runtimePlan.publication.state: installed sessions require a ready publication plan.",
    );
  });

  test.each([
    [
      "a legacy contract version",
      { ...completePayload, contractVersion: 2 },
      "$.contractVersion",
    ],
    [
      "a legacy OpenAI model",
      {
        ...completePayload,
        desired: {
          ...completePayload.desired,
          config: {
            ...completePayload.desired.config,
            stt: {
              ...completePayload.desired.config.stt,
              model: "gpt-4o-mini-transcribe",
            },
          },
        },
      },
      "$.desired.config.stt.model",
    ],
    [
      "the removed Mock provider",
      {
        ...completePayload,
        desired: {
          ...completePayload.desired,
          config: {
            ...completePayload.desired.config,
            stt: {
              ...completePayload.desired.config.stt,
              provider: "mock",
            },
          },
        },
      },
      "$.desired.config.stt.provider",
    ],
    [
      "the legacy singular language field",
      {
        ...completePayload,
        desired: {
          ...completePayload.desired,
          config: {
            ...completePayload.desired.config,
            stt: {
              provider: "openai",
              language: "en",
              model: "gpt-transcribe",
            },
          },
        },
      },
      "$.desired.config.stt.language",
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
      "an invalid recognition enum",
      {
        ...completePayload,
        desired: {
          ...completePayload.desired,
          runtimePlan: {
            ...completePayload.desired.runtimePlan,
            recognition: {
              ...completePayload.desired.runtimePlan.recognition,
              unitBehavior: "timedGuess",
            },
          },
        },
      },
      "$.desired.runtimePlan.recognition.unitBehavior",
    ],
    [
      "a non-positive Live delay",
      {
        ...completePayload,
        desired: {
          ...completePayload.desired,
          runtimePlan: {
            ...completePayload.desired.runtimePlan,
            publication: {
              ...completePayload.desired.runtimePlan.publication,
              policy: { policy: "liveUnit", observationWindowMs: 0 },
            },
          },
        },
      },
      "$.desired.runtimePlan.publication.policy.observationWindowMs",
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
      "$.desired.runtimePlan.publication.mode",
    ],
  ])("rejects %s", (_name, payload, path) => {
    expect(() => decodeRuntimeControlSnapshotV3(payload)).toThrow(path);
  });
});
