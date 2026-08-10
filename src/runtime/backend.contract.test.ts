import { describe, expect, test } from "vitest";
import type { RuntimeBackend } from "./backend";
import { createPreviewBackend, previewRuntimePlan } from "./previewBackend";
import { createTauriBackend, type TauriBackendBridge } from "./tauriBackend";
import { RUNTIME_CONTROL_EVENT, RUNTIME_EVENTS } from "./wire/tauriIpc";
import {
  APP_CONFIG_SCHEMA_VERSION,
  type AppConfig,
  type AudioProbeRequest,
  type CaptionSessionSnapshotV1,
  type DiagnosticCategory,
  type RuntimeControlSnapshot,
  type RuntimeEvent,
  type RuntimeSession,
  type RuntimeStatusEvent,
} from "./types";

const fakeInitialConfig: AppConfig = {
  schemaVersion: APP_CONFIG_SCHEMA_VERSION,
  audio: { inputDeviceId: null },
  stt: {
    provider: "openai",
    languages: ["en"],
    model: "gpt-transcribe",
  },
  osc: {
    host: "127.0.0.1",
    port: 9000,
    enabled: true,
  },
  publication: { mode: "completed" },
  ui: { showPartial: true },
};

function createFakeTauriBridge(): TauriBackendBridge {
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
  let session: RuntimeSession | null = null;
  let latestStatus: RuntimeStatusEvent = {
    status: "idle",
    message: "Runtime is idle",
    timestampMs: 1,
  };
  let captionSession: CaptionSessionSnapshotV1 = {
    contractVersion: 1,
    snapshotRevision: 0,
    active: null,
    activeUnits: [],
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
      contractVersion: 3,
      revision: controlRevision,
      runtime: { ...latestStatus },
      desired: {
        revision: configRevision,
        config: structuredClone(config),
        runtimePlan: previewRuntimePlan(config),
        providerSecrets: [],
      },
      session: session ? structuredClone(session) : null,
      pendingChanges: [],
    };
  }

  function emitControlSnapshot() {
    controlRevision += 1;
    emit(RUNTIME_CONTROL_EVENT, controlSnapshot());
  }

  function emitCaptionSessionUpdate(
    next: Omit<
      CaptionSessionSnapshotV1,
      "contractVersion" | "snapshotRevision"
    >,
  ) {
    captionSession = {
      contractVersion: 1,
      snapshotRevision: captionSession.snapshotRevision + 1,
      ...next,
    };
    emit(RUNTIME_EVENTS.captionSessionChanged, structuredClone(captionSession));
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

      if (command === "start_runtime") {
        if (
          ["starting", "running", "reconnecting", "stopping"].includes(
            latestStatus.status,
          )
        ) {
          return Promise.reject(new Error("Runtime is already active."));
        }

        nextGeneration += 1;
        const selected = {
          audio: structuredClone(config.audio),
          stt: structuredClone(config.stt),
          osc: structuredClone(config.osc),
          publication: structuredClone(config.publication),
        };
        session = {
          generation: nextGeneration,
          phase: "starting",
          startedFromConfigRevision: configRevision,
          selected,
          runtimePlan: previewRuntimePlan(config),
          credential: null,
          chatbox: {
            state: selected.osc.enabled ? "ready" : "disabled",
            host: selected.osc.host,
            port: selected.osc.port,
          },
          uploadsMicrophoneAudio: true,
        };
        emitCaptionSessionUpdate({
          active: {
            generation: nextGeneration,
            streamId: `recognition-${String(nextGeneration)}-1`,
          },
          activeUnits: [],
          captions: captionSession.captions.filter(
            (caption) => caption.state === "completed",
          ),
        });
        emitStatus("starting", "Starting runtime");
        session = { ...session, phase: "running" };
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
      } else if (command === "stop_runtime") {
        if (
          latestStatus.status === "idle" ||
          latestStatus.status === "stopped"
        ) {
          session = null;
          emitStatus("stopped", "Runtime is already stopped");
        } else {
          if (session) {
            session = { ...session, phase: "stopping" };
          }
          emitStatus("stopping", "Stopping runtime");
          emitCaptionSessionUpdate({
            active: null,
            activeUnits: [],
            captions: captionSession.captions.filter(
              (caption) => caption.state === "completed",
            ),
          });
          session = null;
          emitStatus("stopped", "Runtime stopped");
          emitDiagnostic("runtime", "runtime.stopped", "Runtime stopped");
        }
        result = controlSnapshot();
      } else if (command === "send_osc_test_message") {
        emitDiagnostic("osc", "osc.test_simulated", "OSC test simulated");
      } else if (command === "get_runtime_control_snapshot") {
        result = controlSnapshot();
      } else if (command === "get_caption_session_snapshot") {
        result = structuredClone(captionSession);
      } else if (command === "probe_audio_input") {
        const request = args?.["request"] as AudioProbeRequest;
        result = {
          sampleRate: 48_000,
          durationMs: request.durationMs,
          rmsDbfs: -24,
          peakDbfs: -6,
          clipping: false,
          gateOpen: true,
        };
      } else if (command === "save_app_config") {
        config = structuredClone(args?.["config"] as AppConfig);
        configRevision += 1;
        emitControlSnapshot();
        result = controlSnapshot();
      }

      return Promise.resolve(result as Result);
    },
  };
}

const backendCases: readonly Readonly<{
  name: string;
  create: () => RuntimeBackend;
}>[] = [
  { name: "PreviewBackend", create: createPreviewBackend },
  {
    name: "TauriBackend",
    create: () => createTauriBackend(createFakeTauriBridge()),
  },
];

describe.each(backendCases)("$name contract", ({ create }) => {
  test("publishes a realtime audio level for the active generation", async () => {
    const backend = create();
    const levels: RuntimeEvent[] = [];
    const unsubscribe = await backend.listen((event) => {
      if (event.type === "audioLevel") {
        levels.push(event);
      }
    });

    try {
      const started = await backend.startRuntime();
      const generation = started.session?.generation;

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
    const backend = create();
    const observed: string[] = [];
    const unsubscribeControl = await backend.listenControl(() => {
      observed.push("control");
    });
    const unsubscribeEvents = await backend.listen((event) => {
      if (event.type === "status") {
        observed.push("status");
      }
    });

    try {
      await backend.startRuntime();
    } finally {
      unsubscribeEvents();
      unsubscribeControl();
    }

    expect(observed).toEqual(["control", "status", "control", "status"]);
  });

  test("rejects duplicate Start while preserving the active runtime", async () => {
    const backend = create();
    const events: RuntimeEvent[] = [];
    const unsubscribe = await backend.listen((event) => {
      events.push(event);
    });

    try {
      await backend.startRuntime();
      const eventCount = events.length;

      await expect(backend.startRuntime()).rejects.toThrow(/already active/u);
      expect(events).toHaveLength(eventCount);
      expect((await backend.getControlSnapshot()).runtime.status).toBe(
        "running",
      );
    } finally {
      unsubscribe();
    }
  });

  test("supports independent subscriptions and unsubscribe", async () => {
    const backend = create();
    const first: RuntimeEvent[] = [];
    const second: RuntimeEvent[] = [];
    const unsubscribeFirst = await backend.listen((event) => {
      first.push(event);
    });
    const unsubscribeSecond = await backend.listen((event) => {
      second.push(event);
    });

    await backend.sendOscTestMessage();
    unsubscribeFirst();
    await backend.sendOscTestMessage();
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
    const unsubscribeSharedFirst = await backend.listen(sharedListener);
    const unsubscribeSharedSecond = await backend.listen(sharedListener);

    await backend.sendOscTestMessage();
    unsubscribeSharedFirst();
    await backend.sendOscTestMessage();
    unsubscribeSharedSecond();

    expect(shared).toHaveLength(3);
  });

  test("round-trips settings without changing unrelated fields", async () => {
    const backend = create();
    const initial = (await backend.getControlSnapshot()).desired.config;
    const changed: AppConfig = {
      ...initial,
      audio: { inputDeviceId: "chosen-device" },
      osc: { ...initial.osc, enabled: false },
      ui: { showPartial: false },
    };

    expect((await backend.saveConfig(changed)).desired.config).toEqual(changed);
    expect((await backend.getControlSnapshot()).desired.config).toEqual(
      changed,
    );
  });

  test("returns a deterministic offline microphone probe", async () => {
    const backend = create();

    await expect(
      backend.probeAudioInput({ inputDeviceId: null, durationMs: 1_500 }),
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

test("TauriBackend cleans up successful channel registrations when one fails", async () => {
  const registeredChannels: string[] = [];
  const activeChannels = new Set<string>();
  const bridge: TauriBackendBridge = {
    listen(eventName) {
      registeredChannels.push(eventName);

      if (eventName === RUNTIME_EVENTS.captionSessionChanged) {
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
  const backend = createTauriBackend(bridge);

  await expect(backend.listen(() => undefined)).rejects.toThrow(
    "listener registration failed",
  );

  expect(registeredChannels).toEqual([
    RUNTIME_EVENTS.status,
    RUNTIME_EVENTS.audioLevel,
    RUNTIME_EVENTS.captionSessionChanged,
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
  "TauriBackend rejects malformed %s pushes before delivery",
  async (eventName, payload) => {
    let deliver: ((event: Readonly<{ payload: unknown }>) => void) | undefined;
    const backend = createTauriBackend({
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
    const unsubscribe = await backend.listen((event) => {
      received.push(event);
    });

    expect(() => deliver?.({ payload })).toThrow(/Invalid runtime event/u);
    expect(received).toEqual([]);
    unsubscribe();
  },
);
