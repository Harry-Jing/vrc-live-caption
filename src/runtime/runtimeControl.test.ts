import { expect, test } from "vitest";
import {
  projectRuntimeControlSnapshot,
  runtimeStatusNeedsControlReconciliation,
  selectNewerRuntimeControlSnapshot,
} from "./runtimeControl";
import {
  APP_CONFIG_SCHEMA_VERSION,
  type AppConfig,
  type RuntimeControlSnapshot,
  type RuntimePlan,
} from "./types";

const desiredConfig: AppConfig = {
  schemaVersion: APP_CONFIG_SCHEMA_VERSION,
  audio: { inputDeviceId: "next-device" },
  stt: {
    provider: "openai",
    languages: ["zh", "en"],
    model: "gpt-live-transcribe",
  },
  osc: { enabled: false, host: "192.0.2.20", port: 9010 },
  publication: { mode: "live" },
  ui: { showPartial: false },
};
const desiredRuntimePlan: RuntimePlan = {
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
    policy: { policy: "liveUnit", observationWindowMs: 1_000 },
    selectedLanes: ["source"],
  },
};
const sessionRuntimePlan: RuntimePlan = {
  recognition: {
    path: "openAiGptTranscribe",
    inputShape: "continuousAudioFrames",
    boundaryOwner: "application",
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
    state: "ready",
    mode: "completed",
    policy: { policy: "completed" },
    selectedLanes: ["source"],
  },
};

test("projects desired settings separately from the immutable active session", () => {
  const snapshot: RuntimeControlSnapshot = {
    contractVersion: 3,
    revision: 4,
    runtime: {
      status: "running",
      message: "Listening",
      timestampMs: 40,
    },
    desired: {
      revision: 2,
      config: desiredConfig,
      runtimePlan: desiredRuntimePlan,
      providerSecrets: [
        {
          provider: "openai",
          configured: false,
          storage: null,
          displaySuffix: null,
          error: null,
        },
      ],
    },
    session: {
      generation: 3,
      phase: "running",
      startedFromConfigRevision: 1,
      selected: {
        audio: { inputDeviceId: "active-device" },
        stt: {
          provider: "openai",
          languages: ["en"],
          model: "gpt-transcribe",
        },
        osc: { enabled: true, host: "127.0.0.1", port: 9000 },
        publication: { mode: "completed" },
      },
      runtimePlan: sessionRuntimePlan,
      credential: {
        provider: "openai",
        storage: "systemCredentialStore",
        displaySuffix: "cdef",
        revision: 1,
      },
      chatbox: {
        state: "ready",
        host: "127.0.0.1",
        port: 9000,
      },
      uploadsMicrophoneAudio: true,
    },
    pendingChanges: [
      "microphone",
      "recognition",
      "chatboxOutput",
      "publication",
    ],
  };

  const accepted = selectNewerRuntimeControlSnapshot(null, snapshot);
  const projection = projectRuntimeControlSnapshot(accepted);

  expect(projection.config).toEqual(desiredConfig);
  expect(projection.currentSetupConfig).toEqual({
    ...desiredConfig,
    audio: { inputDeviceId: "active-device" },
    stt: {
      provider: "openai",
      languages: ["en"],
      model: "gpt-transcribe",
    },
    osc: { enabled: true, host: "127.0.0.1", port: 9000 },
    publication: { mode: "completed" },
  });
  expect(projection.currentSession).toBe(snapshot.session);
  expect(projection.desiredRuntimePlan).toBe(snapshot.desired.runtimePlan);
  expect(projection.sessionRuntimePlan).toBe(snapshot.session?.runtimePlan);
  expect(projection.pendingSessionChanges).toEqual([
    "microphone",
    "recognition",
    "chatboxOutput",
    "publication",
  ]);
  expect(projection.sessionUploadsMicrophoneAudio).toBe(true);
  expect(projection.secretStatuses.openai?.configured).toBe(false);
});

test("ignores duplicate and older authoritative control snapshots", () => {
  const current = {
    contractVersion: 3,
    revision: 8,
    runtime: { status: "running", timestampMs: 80 },
    desired: {
      revision: 3,
      config: desiredConfig,
      runtimePlan: desiredRuntimePlan,
      providerSecrets: [],
    },
    session: null,
    pendingChanges: [],
  } satisfies RuntimeControlSnapshot;
  const stale = {
    ...current,
    revision: 7,
    runtime: { status: "stopped", timestampMs: 70 },
  } satisfies RuntimeControlSnapshot;

  expect(selectNewerRuntimeControlSnapshot(current, current)).toBe(current);
  expect(selectNewerRuntimeControlSnapshot(current, stale)).toBe(current);
});

test("accepts a newer snapshot even when its display timestamp is lower", () => {
  const current = {
    contractVersion: 3,
    revision: 3,
    runtime: { status: "idle", timestampMs: 30 },
    desired: {
      revision: 1,
      config: desiredConfig,
      runtimePlan: desiredRuntimePlan,
      providerSecrets: [],
    },
    session: null,
    pendingChanges: [],
  } satisfies RuntimeControlSnapshot;
  const newer = {
    ...current,
    revision: 4,
    runtime: { status: "error", timestampMs: 20 },
  } satisfies RuntimeControlSnapshot;

  const accepted = selectNewerRuntimeControlSnapshot(current, newer);
  const projection = projectRuntimeControlSnapshot(accepted);

  expect(accepted).toBe(newer);
  expect(accepted.runtime).toEqual({ status: "error", timestampMs: 20 });
  expect(projection.currentSession).toBeNull();
  expect(projection.desiredRuntimePlan).toBe(newer.desired.runtimePlan);
  expect(projection.sessionRuntimePlan).toBeNull();
  expect(projection.currentSetupConfig).toBe(desiredConfig);
  expect(projection.sessionUploadsMicrophoneAudio).toBe(false);
});

test("requests a control pull when a legacy status outpaces a missed control event", () => {
  const startingSnapshot = {
    contractVersion: 3,
    revision: 4,
    runtime: {
      status: "starting",
      message: "Starting runtime",
      timestampMs: 40,
    },
    desired: {
      revision: 1,
      config: desiredConfig,
      runtimePlan: desiredRuntimePlan,
      providerSecrets: [],
    },
    session: null,
    pendingChanges: [],
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
        runtime: observedRunning,
      },
      observedRunning,
    ),
  ).toBe(false);
});

test("requests a control pull for an accepted same-timestamp status mismatch", () => {
  const snapshot = {
    contractVersion: 3,
    revision: 4,
    runtime: { status: "starting", timestampMs: 40 },
    desired: {
      revision: 1,
      config: desiredConfig,
      runtimePlan: desiredRuntimePlan,
      providerSecrets: [],
    },
    session: null,
    pendingChanges: [],
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
