import { describe, expect, test } from "vitest";
import runtimeControlFixture from "../../contracts/runtime-control-snapshot-v2.json?raw";
import { decodeRuntimeControlSnapshotV2 } from "./runtimeControlContract";

const completePayload = {
  contractVersion: 2,
  revision: 9,
  runtime: {
    status: "running",
    message: "Runtime is running",
    timestampMs: 900,
  },
  desired: {
    revision: 4,
    config: {
      schemaVersion: 2,
      audio: { inputDeviceId: null },
      stt: {
        provider: "mock",
        language: "en",
        model: "mock-ongoing-completed",
      },
      osc: { host: "127.0.0.1", port: 9000, enabled: true },
      publication: { mode: "live" },
      ui: { showPartial: true },
    },
    runtimePlan: {
      recognition: {
        path: "mockOngoingCompleted",
        inputShape: "continuousAudioFrames",
        boundaryOwner: "provider",
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
        provider: "mock",
        language: "en",
        model: "mock-ongoing-completed",
      },
      osc: { host: "127.0.0.1", port: 9000, enabled: true },
      publication: { mode: "live" },
    },
    runtimePlan: {
      recognition: {
        path: "mockOngoingCompleted",
        inputShape: "continuousAudioFrames",
        boundaryOwner: "provider",
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
    uploadsMicrophoneAudio: false,
  },
  pendingChanges: [],
};

describe("runtime control contract", () => {
  test("decodes the shared Rust-serialized V2 fixture", () => {
    const fixture = JSON.parse(runtimeControlFixture) as unknown;

    expect(decodeRuntimeControlSnapshotV2(fixture)).toEqual(fixture);
  });

  test("decodes a complete authoritative V2 snapshot", () => {
    expect(decodeRuntimeControlSnapshotV2(completePayload)).toEqual(
      completePayload,
    );
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

    expect(decodeRuntimeControlSnapshotV2(payload)).toMatchObject({
      desired: {
        config: { publication: { mode: "live" } },
        runtimePlan: { publication: incompatiblePublication },
      },
      session: null,
    });
  });

  test("decodes completed and unitless Live policies", () => {
    const cases = [
      {
        mode: "completed",
        policy: { policy: "completed" },
      },
      {
        mode: "live",
        policy: { policy: "liveUnitless", firstNonEmptyDelayMs: 1000 },
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
        decodeRuntimeControlSnapshotV2(payload).desired.runtimePlan.publication,
      ).toEqual(payload.desired.runtimePlan.publication);
    }
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

    expect(() => decodeRuntimeControlSnapshotV2(payload)).toThrow(
      "Invalid runtime control payload at $.session.runtimePlan.publication.state: installed sessions require a ready publication plan.",
    );
  });

  test.each([
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
    expect(() => decodeRuntimeControlSnapshotV2(payload)).toThrow(path);
  });
});
