import { describe, expect, test } from "vitest";
import type { RuntimeBackend } from "./backend";
import { createPreviewBackend } from "./previewBackend";
import {
  createRuntimeState,
  reduceRuntimeState,
  selectRuntimeView,
} from "./runtimeState";
import { createTauriBackend, type TauriBackendBridge } from "./tauriBackend";
import {
  APP_CONFIG_SCHEMA_VERSION,
  RUNTIME_EVENTS,
  type AppConfig,
  type DiagnosticCategory,
  type RuntimeEvent,
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
  let latestStatus: RuntimeStatusEvent = {
    status: "idle",
    message: "Runtime is idle",
    timestampMs: 1,
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
    emit(RUNTIME_EVENTS.status, latestStatus);
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

        emitStatus("starting", "Starting runtime");
        emitStatus("running", "Runtime is running");
      } else if (command === "stop_runtime") {
        if (
          latestStatus.status === "idle" ||
          latestStatus.status === "stopped"
        ) {
          emitStatus("stopped", "Runtime is already stopped");
        } else {
          emitStatus("stopping", "Stopping runtime");
          emitStatus("stopped", "Runtime stopped");
          emitDiagnostic("runtime", "runtime.stopped", "Runtime stopped");
        }
      } else if (command === "emit_mock_transcript") {
        const utteranceId = eventId("utterance");
        const timestampMs = timestamp();
        const base = {
          utteranceId,
          language: "en",
          provider: "openai",
          timestampMs,
        };

        emit(RUNTIME_EVENTS.utteranceStarted, {
          id: eventId("utterance-start"),
          utteranceId,
          timestampMs,
        });
        emit(RUNTIME_EVENTS.transcriptPartial, {
          ...base,
          id: eventId("transcript"),
          kind: "partial",
          text: "Testing live caption preview...",
          revision: 1,
        });
        emit(RUNTIME_EVENTS.transcriptFinal, {
          ...base,
          id: eventId("transcript"),
          kind: "final",
          text: expectedFinalText,
          revision: 2,
        });
        emitDiagnostic(
          "stt",
          "stt.mock_transcript_emitted",
          "Mock transcript emitted",
        );
      } else if (command === "send_osc_test_message") {
        emitDiagnostic("osc", "osc.test_simulated", "OSC test simulated");
      } else if (command === "get_runtime_status") {
        result = { ...latestStatus };
      } else if (command === "get_app_config") {
        result = structuredClone(config);
      } else if (command === "save_app_config") {
        config = structuredClone(args?.["config"] as AppConfig);
        result = structuredClone(config);
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

  for (const event of events) {
    state = reduceRuntimeState(state, { type: "backendEvent", event });
  }

  return selectRuntimeView(state, { showPartial: true });
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
  test("normalizes the same completed lifecycle and Stop behavior", async () => {
    const backend = create();
    const events: RuntimeEvent[] = [];
    const unsubscribe = await backend.listen((event) => {
      events.push(event);
    });

    try {
      await backend.runCommand("start_runtime");
      expect((await backend.getRuntimeStatus()).status).toBe("running");

      await backend.runCommand("emit_mock_transcript");
      await backend.runCommand("stop_runtime");
      expect((await backend.getRuntimeStatus()).status).toBe("stopped");
    } finally {
      unsubscribe();
    }

    expect(events.map((event) => event.type)).toEqual([
      "status",
      "status",
      "utteranceStarted",
      "transcriptPartial",
      "transcriptFinal",
      "diagnostic",
      "status",
      "status",
      "diagnostic",
    ]);

    const partial = events.find((event) => event.type === "transcriptPartial");
    const final = events.find((event) => event.type === "transcriptFinal");

    expect(partial?.payload).toMatchObject({
      kind: "partial",
      language: "en",
      provider: "openai",
      revision: 1,
      text: "Testing live caption preview...",
    });
    expect(final?.payload).toMatchObject({
      kind: "final",
      language: "en",
      provider: "openai",
      revision: 2,
      text: expectedFinalText,
    });
    expect(final?.payload.utteranceId).toBe(partial?.payload.utteranceId);

    const view = projectEvents(events);

    expect(view.runtimeStatus.status).toBe("stopped");
    expect(view.captionMode).toBe("final");
    expect(view.visibleTranscript?.text).toBe(expectedFinalText);
    expect(view.finalTranscripts).toHaveLength(1);
    expect(view.diagnostics.at(0)?.code).toBe("runtime.stopped");
    expect(view.diagnostics.at(1)?.code).toBe("stt.mock_transcript_emitted");
  });

  test("rejects duplicate Start while preserving the active runtime", async () => {
    const backend = create();
    const events: RuntimeEvent[] = [];
    const unsubscribe = await backend.listen((event) => {
      events.push(event);
    });

    try {
      await backend.runCommand("start_runtime");
      const eventCount = events.length;

      await expect(backend.runCommand("start_runtime")).rejects.toThrow(
        /already active/u,
      );
      expect(events).toHaveLength(eventCount);
      expect((await backend.getRuntimeStatus()).status).toBe("running");
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
    const initial = await backend.getConfig();
    const changed: AppConfig = {
      ...initial,
      audio: { inputDeviceId: "chosen-device" },
      osc: { ...initial.osc, enabled: false },
      ui: { showPartial: false },
    };

    expect(await backend.saveConfig(changed)).toEqual(changed);
    expect(await backend.getConfig()).toEqual(changed);
  });
});

test("TauriBackend cleans up successful channel registrations when one fails", async () => {
  const registeredChannels: string[] = [];
  const activeChannels = new Set<string>();
  const bridge: TauriBackendBridge = {
    listen(eventName) {
      registeredChannels.push(eventName);

      if (eventName === RUNTIME_EVENTS.transcriptFinal) {
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
