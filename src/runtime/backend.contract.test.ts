import { describe, expect, test } from "vitest";
import type { RuntimeBackend } from "./backend";
import { createPreviewBackend } from "./previewBackend";
import {
  createCaptionSessionState,
  reduceCaptionSessionState,
  selectCaptionSessionView,
} from "./captionSession";
import {
  createRuntimeState,
  reduceRuntimeState,
  selectRuntimeView,
} from "./runtimeState";
import { createTauriBackend, type TauriBackendBridge } from "./tauriBackend";
import {
  APP_CONFIG_SCHEMA_VERSION,
  RUNTIME_EVENTS,
  RUNTIME_CONTROL_EVENT,
  type AppConfig,
  type CaptionSessionSnapshotV1,
  type CaptionSnapshotV1,
  type DiagnosticCategory,
  type RuntimeControlSnapshot,
  type RuntimeEvent,
  type RuntimeSession,
  type RuntimeStatusEvent,
} from "./types";

const expectedFinalText = "Testing live caption preview from the mock runtime.";

const fakeInitialConfig: AppConfig = {
  schemaVersion: APP_CONFIG_SCHEMA_VERSION,
  audio: { inputDeviceId: null },
  stt: {
    provider: "openai",
    language: "en",
    model: "gpt-4o-mini-transcribe",
  },
  osc: {
    host: "127.0.0.1",
    port: 9000,
    enabled: true,
  },
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

  function eventId(prefix: string) {
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
    publishControl();
    emit(RUNTIME_EVENTS.status, latestStatus);
  }

  function controlSnapshot(): RuntimeControlSnapshot {
    return {
      contractVersion: 1,
      revision: controlRevision,
      runtime: { ...latestStatus },
      desired: {
        revision: configRevision,
        config: structuredClone(config),
        providerSecrets: [],
      },
      session: session ? structuredClone(session) : null,
      pendingChanges: [],
    };
  }

  function publishControl() {
    controlRevision += 1;
    emit(RUNTIME_CONTROL_EVENT, controlSnapshot());
  }

  function publishCaptionSession(
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
      id: eventId("diagnostic"),
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
        if (["starting", "running", "stopping"].includes(latestStatus.status)) {
          return Promise.reject(new Error("Runtime is already active."));
        }

        nextGeneration += 1;
        const selected = {
          audio: structuredClone(config.audio),
          stt: structuredClone(config.stt),
          osc: structuredClone(config.osc),
        };
        session = {
          generation: nextGeneration,
          phase: "starting",
          startedFromConfigRevision: configRevision,
          selected,
          credential: null,
          chatbox: {
            state: selected.osc.enabled ? "ready" : "disabled",
            host: selected.osc.host,
            port: selected.osc.port,
          },
          uploadsMicrophoneAudio: selected.stt.provider === "openai",
        };
        publishCaptionSession({
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
          publishCaptionSession({
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
      } else if (command === "emit_mock_transcript") {
        if (
          latestStatus.status !== "running" ||
          session?.selected.stt.provider !== "mock"
        ) {
          return Promise.reject(
            new Error(
              "Mock Transcript requires an active Mock runtime session.",
            ),
          );
        }
        const utteranceId = eventId("utterance");
        const timestampMs = timestamp();
        const active = captionSession.active;

        if (active === null) {
          return Promise.reject(new Error("Recognition stream is missing."));
        }
        const base = {
          generation: active.generation,
          streamId: active.streamId,
          unitId: utteranceId,
          lane: "source" as const,
          language: session.selected.stt.language,
          provider: session.selected.stt.provider,
          model: session.selected.stt.model,
          unitStartedAtMs: timestampMs,
          timestampMs,
        };

        emit(RUNTIME_EVENTS.utteranceStarted, {
          id: eventId("utterance-start"),
          generation: active.generation,
          streamId: active.streamId,
          utteranceId,
          timestampMs,
        });
        publishCaptionSession({
          active,
          activeUnits: [{ unitId: utteranceId, startedAtMs: timestampMs }],
          captions: captionSession.captions.filter(
            (caption) => caption.state === "completed",
          ),
        });
        const ongoing: CaptionSnapshotV1 = {
          ...base,
          revision: 1,
          text: "Testing live caption preview...",
          state: "ongoing",
        };
        publishCaptionSession({
          active,
          activeUnits: [{ unitId: utteranceId, startedAtMs: timestampMs }],
          captions: [ongoing, ...captionSession.captions],
        });
        const completed: CaptionSnapshotV1 = {
          ...base,
          revision: 2,
          text: expectedFinalText,
          state: "completed",
        };
        publishCaptionSession({
          active,
          activeUnits: [],
          captions: [
            completed,
            ...captionSession.captions.filter(
              (caption) => caption.state === "completed",
            ),
          ].slice(0, 5),
        });
        emitDiagnostic(
          "stt",
          "stt.mock_transcript_emitted",
          "Mock transcript emitted",
        );
      } else if (command === "send_osc_test_message") {
        emitDiagnostic("osc", "osc.test_simulated", "OSC test simulated");
      } else if (command === "get_runtime_control_snapshot") {
        result = controlSnapshot();
      } else if (command === "get_caption_session_snapshot") {
        result = structuredClone(captionSession);
      } else if (command === "save_app_config") {
        config = structuredClone(args?.["config"] as AppConfig);
        configRevision += 1;
        publishControl();
        result = controlSnapshot();
      }

      return Promise.resolve(result as Result);
    },
  };
}

function projectEvents(events: readonly RuntimeEvent[]) {
  let state = createRuntimeState({
    status: "idle",
    message: "Synthetic initial state",
    timestampMs: 0,
  });
  let captionState = createCaptionSessionState();

  for (const event of events) {
    if (event.type === "captionSessionChanged") {
      captionState = reduceCaptionSessionState(captionState, {
        type: "snapshotReceived",
        snapshot: event.payload,
      });
    } else {
      state = reduceRuntimeState(state, { type: "backendEvent", event });
    }
  }

  return {
    runtime: selectRuntimeView(state),
    caption: selectCaptionSessionView(captionState, true),
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

  test("normalizes the same completed lifecycle and Stop behavior", async () => {
    const backend = create();
    const events: RuntimeEvent[] = [];
    const unsubscribe = await backend.listen((event) => {
      events.push(event);
    });

    try {
      const initialConfig = (await backend.getControlSnapshot()).desired.config;
      await backend.saveConfig({
        ...initialConfig,
        stt: { ...initialConfig.stt, provider: "mock" },
      });
      await backend.startRuntime();
      expect((await backend.getControlSnapshot()).runtime.status).toBe(
        "running",
      );

      await backend.runCommand("emit_mock_transcript");
      await backend.stopRuntime();
      expect((await backend.getControlSnapshot()).runtime.status).toBe(
        "stopped",
      );
    } finally {
      unsubscribe();
    }

    expect(events.map((event) => event.type)).toEqual([
      "captionSessionChanged",
      "status",
      "status",
      "utteranceStarted",
      "captionSessionChanged",
      "captionSessionChanged",
      "captionSessionChanged",
      "diagnostic",
      "status",
      "captionSessionChanged",
      "status",
      "diagnostic",
    ]);

    const captionAggregates = events.filter(
      (event) => event.type === "captionSessionChanged",
    );
    const ongoing = captionAggregates.find((event) =>
      event.payload.captions.some((caption) => caption.state === "ongoing"),
    );
    const completed = captionAggregates.find((event) =>
      event.payload.captions.some(
        (caption) => caption.text === expectedFinalText,
      ),
    );

    expect(ongoing?.payload.captions[0]).toMatchObject({
      state: "ongoing",
      language: "en",
      provider: "mock",
      revision: 1,
      text: "Testing live caption preview...",
    });
    expect(completed?.payload.captions[0]).toMatchObject({
      state: "completed",
      language: "en",
      provider: "mock",
      revision: 2,
      text: expectedFinalText,
    });

    const view = projectEvents(events);

    expect(view.runtime.runtimeStatus.status).toBe("stopped");
    expect(view.caption.captionMode).toBe("final");
    expect(view.caption.visibleCaption?.text).toBe(expectedFinalText);
    expect(view.caption.completedCaptions).toHaveLength(1);
    expect(view.runtime.diagnostics.at(0)?.code).toBe("runtime.stopped");
    expect(view.runtime.diagnostics.at(1)?.code).toBe(
      "stt.mock_transcript_emitted",
    );
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

    await backend.runCommand("send_osc_test_message");
    unsubscribeFirst();
    await backend.runCommand("send_osc_test_message");
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

    await backend.runCommand("send_osc_test_message");
    unsubscribeSharedFirst();
    await backend.runCommand("send_osc_test_message");
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

  expect(registeredChannels).toHaveLength(Object.keys(RUNTIME_EVENTS).length);
  expect(activeChannels.size).toBe(0);
});
