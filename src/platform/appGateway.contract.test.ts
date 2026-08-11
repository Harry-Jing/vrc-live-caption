import { describe, expect, test } from "vitest";
import {
  createPreviewAppGateway,
  previewCaptionPipelinePlan,
} from "./preview/appGateway";
import type { AppGateway } from "../runtime/gateway";
import {
  RUNTIME_CONTROL_EVENT,
  RUNTIME_EVENTS,
  TAURI_COMMANDS,
} from "../runtime/wire/tauriIpc";
import { createTauriAppGateway, type TauriIpcBridge } from "./tauri/appGateway";
import {
  APP_CONFIG_SCHEMA_VERSION,
  type AppConfig,
} from "../runtime/appConfig";
import type { AudioProbeRequest } from "../runtime/audio";
import type { CaptionAggregateSnapshot } from "../runtime/captionAggregate";
import type {
  RuntimeControlSnapshot,
  RuntimeGenerationSnapshot,
} from "../runtime/runtimeControl";
import type {
  DiagnosticCategory,
  RuntimeEvent,
  RuntimeStatusEvent,
} from "../runtime/runtimeEvents";
const fakeInitialConfig: AppConfig = {
  schemaVersion: APP_CONFIG_SCHEMA_VERSION,
  audio: { inputDeviceId: null },
  recognition: {
    path: "openai/gpt-transcribe",
    expectedLanguages: ["en"],
  },
  osc: {
    host: "127.0.0.1",
    port: 9000,
    enabled: true,
  },
  publication: { mode: "completed" },
  ui: { showOngoingPreview: true },
};

function createFakeTauriIpcBridge(): TauriIpcBridge {
  const listeners = new Map<
    string,
    Set<(event: Readonly<{ payload: unknown }>) => void>
  >();
  let nextEventNumber = 0;
  let nextTimestampMs = 10;
  let config = structuredClone(fakeInitialConfig);
  let configRevision = 1;
  let controlRevision = 1;
  let nextGeneration = 0;
  let generation: RuntimeGenerationSnapshot | null = null;
  let latestStatus: RuntimeStatusEvent = {
    status: "idle",
    message: "Runtime is idle",
    timestampMs: 1,
  };
  let captionAggregate: CaptionAggregateSnapshot = {
    contractVersion: 1,
    snapshotRevision: 0,
    activeStream: null,
    openSourceUnits: [],
    captions: [],
  };

  function nextEventId(prefix: string) {
    nextEventNumber += 1;
    return `${prefix}-tauri-${String(nextEventNumber)}`;
  }

  function timestamp() {
    nextTimestampMs += 1;
    return nextTimestampMs;
  }

  function emit(eventName: string, payload: unknown) {
    for (const listener of listeners.get(eventName) ?? []) {
      listener({ payload });
    }
  }

  function emitStatus(status: RuntimeStatusEvent["status"], message: string) {
    latestStatus = { status, message, timestampMs: timestamp() };
    emitControlSnapshot();
    emit(RUNTIME_EVENTS.status, latestStatus);
  }

  function controlSnapshot(): RuntimeControlSnapshot {
    return {
      contractVersion: 1,
      revision: controlRevision,
      runtimeStatus: { ...latestStatus },
      desired: {
        revision: configRevision,
        config: structuredClone(config),
        captionPipelinePlan: previewCaptionPipelinePlan(config),
        credentials: [],
      },
      generation: generation ? structuredClone(generation) : null,
      pendingGenerationChanges: [],
    };
  }

  function emitControlSnapshot() {
    controlRevision += 1;
    emit(RUNTIME_CONTROL_EVENT, controlSnapshot());
  }

  function emitCaptionAggregateUpdate(
    next: Omit<
      CaptionAggregateSnapshot,
      "contractVersion" | "snapshotRevision"
    >,
  ) {
    captionAggregate = {
      contractVersion: 1,
      snapshotRevision: captionAggregate.snapshotRevision + 1,
      ...next,
    };
    emit(
      RUNTIME_EVENTS.captionAggregateChanged,
      structuredClone(captionAggregate),
    );
  }

  function emitDiagnostic(
    category: DiagnosticCategory,
    code: string,
    message: string,
  ) {
    emit(RUNTIME_EVENTS.diagnostic, {
      id: nextEventId("diagnostic"),
      category,
      severity: "info",
      code,
      message,
      timestampMs: timestamp(),
    });
  }

  return {
    listen(eventName, listener) {
      const eventListeners = listeners.get(eventName) ?? new Set();
      eventListeners.add(listener);
      listeners.set(eventName, eventListeners);

      return Promise.resolve(() => {
        eventListeners.delete(listener);

        if (eventListeners.size === 0) {
          listeners.delete(eventName);
        }
      });
    },

    invoke<Result>(command: string, args?: Record<string, unknown>) {
      let result: unknown;

      if (command === TAURI_COMMANDS.startRuntime) {
        if (
          ["starting", "running", "reconnecting", "stopping"].includes(
            latestStatus.status,
          )
        ) {
          return Promise.reject(new Error("Runtime is already active."));
        }

        nextGeneration += 1;
        const selection = {
          audio: structuredClone(config.audio),
          recognition: structuredClone(config.recognition),
          osc: structuredClone(config.osc),
          publication: structuredClone(config.publication),
        };
        generation = {
          id: nextGeneration,
          phase: "starting",
          startedFromConfigRevision: configRevision,
          selection,
          captionPipelinePlan: previewCaptionPipelinePlan(config),
          credential: null,
          chatboxPublication: {
            state: selection.osc.enabled ? "ready" : "disabled",
            host: selection.osc.host,
            port: selection.osc.port,
          },
          uploadsMicrophoneAudio: true,
        };
        emitCaptionAggregateUpdate({
          activeStream: {
            generation: nextGeneration,
            streamId: `recognition-${String(nextGeneration)}-1`,
          },
          openSourceUnits: [],
          captions: captionAggregate.captions.filter(
            (caption) => caption.state === "completed",
          ),
        });
        emitStatus("starting", "Starting runtime");
        generation = { ...generation, phase: "running" };
        emitStatus("running", "Runtime is running");
        emit(RUNTIME_EVENTS.audioLevel, {
          generation: nextGeneration,
          revision: 1,
          rmsDbfs: -24,
          peakDbfs: -6,
          clipping: false,
          gateOpen: true,
          timestampMs: timestamp(),
        });
        result = controlSnapshot();
      } else if (command === TAURI_COMMANDS.stopRuntime) {
        if (
          latestStatus.status === "idle" ||
          latestStatus.status === "stopped"
        ) {
          generation = null;
          emitStatus("stopped", "Runtime is already stopped");
        } else {
          if (generation) {
            generation = { ...generation, phase: "stopping" };
          }
          emitStatus("stopping", "Stopping runtime");
          emitCaptionAggregateUpdate({
            activeStream: null,
            openSourceUnits: [],
            captions: captionAggregate.captions.filter(
              (caption) => caption.state === "completed",
            ),
          });
          generation = null;
          emitStatus("stopped", "Runtime stopped");
          emitDiagnostic("runtime", "runtime.stopped", "Runtime stopped");
        }
        result = controlSnapshot();
      } else if (command === TAURI_COMMANDS.sendOscTestMessage) {
        emitDiagnostic("osc", "osc.test_simulated", "OSC test simulated");
      } else if (command === TAURI_COMMANDS.getRuntimeControlSnapshot) {
        result = controlSnapshot();
      } else if (command === TAURI_COMMANDS.getCaptionAggregateSnapshot) {
        result = structuredClone(captionAggregate);
      } else if (command === TAURI_COMMANDS.probeAudioInput) {
        const request = args?.["request"] as AudioProbeRequest;
        result = {
          sampleRate: 48_000,
          durationMs: request.durationMs,
          rmsDbfs: -24,
          peakDbfs: -6,
          clipping: false,
          gateOpen: true,
        };
      } else if (command === TAURI_COMMANDS.saveAppConfig) {
        config = structuredClone(args?.["config"] as AppConfig);
        configRevision += 1;
        emitControlSnapshot();
        result = controlSnapshot();
      }

      return Promise.resolve(result as Result);
    },
  };
}

const gatewayCases: readonly Readonly<{
  name: string;
  create: () => AppGateway;
}>[] = [
  { name: "PreviewAppGateway", create: createPreviewAppGateway },
  {
    name: "TauriAppGateway",
    create: () => createTauriAppGateway(createFakeTauriIpcBridge()),
  },
];

describe.each(gatewayCases)("$name contract", ({ create }) => {
  test("publishes a realtime audio level for the active generation", async () => {
    const gateway = create();
    const levels: RuntimeEvent[] = [];
    const unsubscribe = await gateway.subscribeRuntimeEvents((event) => {
      if (event.type === "audioLevel") {
        levels.push(event);
      }
    });

    try {
      const started = await gateway.startRuntime();
      const generation = started.generation?.id;

      expect(levels).toEqual([
        {
          type: "audioLevel",
          payload: {
            generation,
            revision: 1,
            rmsDbfs: -24,
            peakDbfs: -6,
            clipping: false,
            gateOpen: true,
            timestampMs: expect.any(Number),
          },
        },
      ]);
    } finally {
      unsubscribe();
    }
  });

  test("publishes control before each legacy lifecycle status", async () => {
    const gateway = create();
    const observed: string[] = [];
    const unsubscribeControl = await gateway.subscribeRuntimeControlSnapshots(
      () => {
        observed.push("control");
      },
    );
    const unsubscribeEvents = await gateway.subscribeRuntimeEvents((event) => {
      if (event.type === "status") {
        observed.push("status");
      }
    });

    try {
      await gateway.startRuntime();
    } finally {
      unsubscribeEvents();
      unsubscribeControl();
    }

    expect(observed).toEqual(["control", "status", "control", "status"]);
  });

  test("rejects duplicate Start while preserving the active runtime", async () => {
    const gateway = create();
    const events: RuntimeEvent[] = [];
    const unsubscribe = await gateway.subscribeRuntimeEvents((event) => {
      events.push(event);
    });

    try {
      await gateway.startRuntime();
      const eventCount = events.length;

      await expect(gateway.startRuntime()).rejects.toThrow(/already active/u);
      expect(events).toHaveLength(eventCount);
      expect(
        (await gateway.getRuntimeControlSnapshot()).runtimeStatus.status,
      ).toBe("running");
    } finally {
      unsubscribe();
    }
  });

  test("supports independent subscriptions and unsubscribe", async () => {
    const gateway = create();
    const first: RuntimeEvent[] = [];
    const second: RuntimeEvent[] = [];
    const unsubscribeFirst = await gateway.subscribeRuntimeEvents((event) => {
      first.push(event);
    });
    const unsubscribeSecond = await gateway.subscribeRuntimeEvents((event) => {
      second.push(event);
    });

    await gateway.sendOscTestMessage();
    unsubscribeFirst();
    await gateway.sendOscTestMessage();
    unsubscribeSecond();

    expect(first).toHaveLength(1);
    expect(second).toHaveLength(2);
    expect(second.map((event) => event.type)).toEqual([
      "diagnostic",
      "diagnostic",
    ]);

    const shared: RuntimeEvent[] = [];
    const sharedListener = (event: RuntimeEvent) => {
      shared.push(event);
    };
    const unsubscribeSharedFirst =
      await gateway.subscribeRuntimeEvents(sharedListener);
    const unsubscribeSharedSecond =
      await gateway.subscribeRuntimeEvents(sharedListener);

    await gateway.sendOscTestMessage();
    unsubscribeSharedFirst();
    await gateway.sendOscTestMessage();
    unsubscribeSharedSecond();

    expect(shared).toHaveLength(3);
  });

  test("round-trips settings without changing unrelated fields", async () => {
    const gateway = create();
    const initial = (await gateway.getRuntimeControlSnapshot()).desired.config;
    const changed: AppConfig = {
      ...initial,
      audio: { inputDeviceId: "chosen-device" },
      osc: { ...initial.osc, enabled: false },
      ui: { showOngoingPreview: false },
    };

    expect((await gateway.saveAppConfig(changed)).desired.config).toEqual(
      changed,
    );
    expect((await gateway.getRuntimeControlSnapshot()).desired.config).toEqual(
      changed,
    );
  });

  test("returns a deterministic offline microphone probe", async () => {
    const gateway = create();

    await expect(
      gateway.probeAudioInput({ inputDeviceId: null, durationMs: 1_500 }),
    ).resolves.toEqual({
      sampleRate: 48_000,
      durationMs: 1_500,
      rmsDbfs: -24,
      peakDbfs: -6,
      clipping: false,
      gateOpen: true,
    });
  });
});

test("TauriAppGateway cleans up successful channel registrations when one fails", async () => {
  const registeredChannels: string[] = [];
  const activeChannels = new Set<string>();
  const bridge: TauriIpcBridge = {
    listen(eventName) {
      registeredChannels.push(eventName);

      if (eventName === RUNTIME_EVENTS.captionAggregateChanged) {
        return Promise.reject(new Error("listener registration failed"));
      }

      activeChannels.add(eventName);
      return Promise.resolve(() => {
        activeChannels.delete(eventName);
      });
    },
    invoke<Result>() {
      return Promise.resolve(undefined as Result);
    },
  };
  const gateway = createTauriAppGateway(bridge);

  await expect(gateway.subscribeRuntimeEvents(() => undefined)).rejects.toThrow(
    "listener registration failed",
  );

  expect(registeredChannels).toEqual([
    RUNTIME_EVENTS.status,
    RUNTIME_EVENTS.audioLevel,
    RUNTIME_EVENTS.captionAggregateChanged,
    RUNTIME_EVENTS.diagnostic,
  ]);
  expect(activeChannels.size).toBe(0);
});

test.each([
  [RUNTIME_EVENTS.status, { status: "paused", timestampMs: 1 }],
  [
    RUNTIME_EVENTS.diagnostic,
    {
      id: "diagnostic-invalid",
      category: "osc",
      severity: "fatal",
      code: "osc.invalid",
      message: "Invalid diagnostic",
      timestampMs: 1,
    },
  ],
] as const)(
  "TauriAppGateway rejects malformed %s pushes before delivery",
  async (eventName, payload) => {
    let deliver: ((event: Readonly<{ payload: unknown }>) => void) | undefined;
    const gateway = createTauriAppGateway({
      listen(registeredEventName, listener) {
        if (registeredEventName === eventName) {
          deliver = listener;
        }

        return Promise.resolve(() => undefined);
      },
      invoke<Result>() {
        return Promise.resolve(undefined as Result);
      },
    });
    const received: RuntimeEvent[] = [];
    const unsubscribe = await gateway.subscribeRuntimeEvents((event) => {
      received.push(event);
    });

    expect(() => deliver?.({ payload })).toThrow(/Invalid runtime event/u);
    expect(received).toEqual([]);
    unsubscribe();
  },
);
