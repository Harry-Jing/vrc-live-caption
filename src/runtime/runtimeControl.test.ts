import { expect, expectTypeOf, test } from "vitest";
import {
  projectRuntimeControlSnapshot,
  runtimeStatusNeedsControlReconciliation,
  selectNewerRuntimeControlSnapshot,
} from "./runtimeControl";
import { type AppConfig, APP_CONFIG_SCHEMA_VERSION } from "./appConfig";
import type { CaptionPipelinePlan } from "./captionPipeline";
import type {
  RuntimeControlSnapshot,
  RuntimeGenerationSelection,
} from "./runtimeControl";

const desiredConfig: AppConfig = {
  schemaVersion: APP_CONFIG_SCHEMA_VERSION,
  audio: { inputDeviceId: "next-device" },
  recognition: {
    path: "openai/gpt-live-transcribe",
    expectedLanguages: ["zh", "en"],
  },
  osc: { enabled: false, host: "192.0.2.20", port: 9010 },
  publication: { mode: "live" },
  ui: { showOngoingPreview: false },
};
const desiredCaptionPipelinePlan: CaptionPipelinePlan = {
  recognition: {
    path: "openai/gpt-live-transcribe",
    inputShape: "continuousAudioFrames",
    captionBoundaryOwner: "application",
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
    state: "compatible",
    mode: "live",
    timing: { timing: "liveUnit", observationWindowMs: 1_000 },
    selectedLanes: ["source"],
  },
};
const generationCaptionPipelinePlan: CaptionPipelinePlan = {
  recognition: {
    path: "openai/gpt-transcribe",
    inputShape: "continuousAudioFrames",
    captionBoundaryOwner: "application",
    unitBehavior: "unitBased",
    lanes: [
      {
        lane: "source",
        updates: "completedOnly",
        revisions: "appendOnly",
      },
    ],
  },
  publication: {
    state: "compatible",
    mode: "completed",
    timing: { timing: "completed" },
    selectedLanes: ["source"],
  },
};

test("classifies every current app config field into generation selection or runtime-only metadata", () => {
  expectTypeOf<RuntimeGenerationSelection>().toEqualTypeOf<
    Omit<AppConfig, "schemaVersion" | "ui">
  >();

  const selection = {
    audio: desiredConfig.audio,
    recognition: desiredConfig.recognition,
    osc: desiredConfig.osc,
    publication: desiredConfig.publication,
  } satisfies RuntimeGenerationSelection;

  expect(Object.keys(selection).sort()).toEqual([
    "audio",
    "osc",
    "publication",
    "recognition",
  ]);
  expect(selection).not.toHaveProperty("schemaVersion");
  expect(selection).not.toHaveProperty("ui");
});

test("projects desired settings separately from the immutable active generation", () => {
  const snapshot: RuntimeControlSnapshot = {
    contractVersion: 1,
    revision: 4,
    runtimeStatus: {
      status: "running",
      message: "Listening",
      timestampMs: 40,
    },
    desired: {
      revision: 2,
      config: desiredConfig,
      captionPipelinePlan: desiredCaptionPipelinePlan,
      credentials: [
        {
          state: "unconfigured",
          id: "openai",
        },
      ],
    },
    generation: {
      id: 3,
      phase: "running",
      startedFromConfigRevision: 1,
      selection: {
        audio: { inputDeviceId: "active-device" },
        recognition: {
          path: "openai/gpt-transcribe",
          expectedLanguages: ["en"],
        },
        osc: { enabled: true, host: "127.0.0.1", port: 9000 },
        publication: { mode: "completed" },
      },
      captionPipelinePlan: generationCaptionPipelinePlan,
      credential: {
        id: "openai",
        storage: "systemCredentialStore",
        displaySuffix: "cdef",
        revision: 1,
      },
      chatboxPublication: {
        state: "ready",
        host: "127.0.0.1",
        port: 9000,
      },
      uploadsMicrophoneAudio: true,
    },
    pendingGenerationChanges: [
      "microphone",
      "recognition",
      "chatboxOutput",
      "publication",
    ],
  };

  const accepted = selectNewerRuntimeControlSnapshot(null, snapshot);
  const projection = projectRuntimeControlSnapshot(accepted);

  expect(projection.desiredConfig).toEqual(desiredConfig);
  expect(projection.currentGenerationSelection).toEqual({
    audio: { inputDeviceId: "active-device" },
    recognition: {
      path: "openai/gpt-transcribe",
      expectedLanguages: ["en"],
    },
    osc: { enabled: true, host: "127.0.0.1", port: 9000 },
    publication: { mode: "completed" },
  });
  expect(projection.currentGeneration).toBe(snapshot.generation);
  expect(projection.desiredCaptionPipelinePlan).toBe(
    snapshot.desired.captionPipelinePlan,
  );
  expect(projection.currentGenerationCaptionPipelinePlan).toBe(
    snapshot.generation?.captionPipelinePlan,
  );
  expect(projection.pendingGenerationChanges).toEqual([
    "microphone",
    "recognition",
    "chatboxOutput",
    "publication",
  ]);
  expect(projection.currentGenerationUploadsMicrophoneAudio).toBe(true);
  expect(projection.credentialStatuses.openai?.state).toBe("unconfigured");
});

test("ignores duplicate and older authoritative control snapshots", () => {
  const current = {
    contractVersion: 1,
    revision: 8,
    runtimeStatus: { status: "running", timestampMs: 80 },
    desired: {
      revision: 3,
      config: desiredConfig,
      captionPipelinePlan: desiredCaptionPipelinePlan,
      credentials: [],
    },
    generation: null,
    pendingGenerationChanges: [],
  } satisfies RuntimeControlSnapshot;
  const stale = {
    ...current,
    revision: 7,
    runtimeStatus: { status: "stopped", timestampMs: 70 },
  } satisfies RuntimeControlSnapshot;

  expect(selectNewerRuntimeControlSnapshot(current, current)).toBe(current);
  expect(selectNewerRuntimeControlSnapshot(current, stale)).toBe(current);
});

test("accepts a newer snapshot even when its display timestamp is lower", () => {
  const current = {
    contractVersion: 1,
    revision: 3,
    runtimeStatus: { status: "idle", timestampMs: 30 },
    desired: {
      revision: 1,
      config: desiredConfig,
      captionPipelinePlan: desiredCaptionPipelinePlan,
      credentials: [],
    },
    generation: null,
    pendingGenerationChanges: [],
  } satisfies RuntimeControlSnapshot;
  const newer = {
    ...current,
    revision: 4,
    runtimeStatus: { status: "error", timestampMs: 20 },
  } satisfies RuntimeControlSnapshot;

  const accepted = selectNewerRuntimeControlSnapshot(current, newer);
  const projection = projectRuntimeControlSnapshot(accepted);

  expect(accepted).toBe(newer);
  expect(accepted.runtimeStatus).toEqual({
    status: "error",
    timestampMs: 20,
  });
  expect(projection.currentGeneration).toBeNull();
  expect(projection.desiredCaptionPipelinePlan).toBe(
    newer.desired.captionPipelinePlan,
  );
  expect(projection.currentGenerationCaptionPipelinePlan).toBeNull();
  expect(projection.currentGenerationSelection).toBeNull();
  expect(projection.currentGenerationUploadsMicrophoneAudio).toBe(false);
});

test("requests a control pull when a legacy status outpaces a missed control event", () => {
  const startingSnapshot = {
    contractVersion: 1,
    revision: 4,
    runtimeStatus: {
      status: "starting",
      message: "Starting runtime",
      timestampMs: 40,
    },
    desired: {
      revision: 1,
      config: desiredConfig,
      captionPipelinePlan: desiredCaptionPipelinePlan,
      credentials: [],
    },
    generation: null,
    pendingGenerationChanges: [],
  } satisfies RuntimeControlSnapshot;
  const observedRunning = {
    status: "running",
    message: "Runtime is running",
    timestampMs: 50,
  } as const;

  expect(
    runtimeStatusNeedsControlReconciliation(startingSnapshot, observedRunning),
  ).toBe(true);
  expect(
    runtimeStatusNeedsControlReconciliation(
      {
        ...startingSnapshot,
        revision: 5,
        runtimeStatus: observedRunning,
      },
      observedRunning,
    ),
  ).toBe(false);
});

test("requests a control pull for an accepted same-timestamp status mismatch", () => {
  const snapshot = {
    contractVersion: 1,
    revision: 4,
    runtimeStatus: { status: "starting", timestampMs: 40 },
    desired: {
      revision: 1,
      config: desiredConfig,
      captionPipelinePlan: desiredCaptionPipelinePlan,
      credentials: [],
    },
    generation: null,
    pendingGenerationChanges: [],
  } satisfies RuntimeControlSnapshot;

  expect(
    runtimeStatusNeedsControlReconciliation(snapshot, {
      status: "running",
      timestampMs: 40,
    }),
  ).toBe(true);
  expect(
    runtimeStatusNeedsControlReconciliation(snapshot, {
      status: "idle",
      timestampMs: 30,
    }),
  ).toBe(false);
});
