import { describe, expect, test } from "vitest";
import type { RuntimeBackend } from "./backend";
import { createPreviewBackend, previewRuntimePlan } from "./previewBackend";
import { createTauriBackend, type TauriBackendBridge } from "./tauriBackend";
import {
  APP_CONFIG_SCHEMA_VERSION,
  RUNTIME_EVENTS,
  RUNTIME_CONTROL_EVENT,
  type AppConfig,
  type CaptionSessionSnapshotV1,
  type RuntimeControlSnapshot,
  type RuntimeEvent,
} from "./types";

const initialConfig: AppConfig = {
  schemaVersion: APP_CONFIG_SCHEMA_VERSION,
  audio: { inputDeviceId: null },
  stt: {
    provider: "openai",
    language: "en",
    model: "gpt-4o-mini-transcribe",
  },
  osc: { enabled: true, host: "127.0.0.1", port: 9000 },
  publication: { mode: "completed" },
  ui: { showPartial: true },
};

function createControlBridge(): TauriBackendBridge {
  const listeners = new Map<
    string,
    Set<(event: Readonly<{ payload: unknown }>) => void>
  >();
  let snapshot: RuntimeControlSnapshot = {
    contractVersion: 2,
    revision: 1,
    runtime: { status: "idle", timestampMs: 1 },
    desired: {
      revision: 1,
      config: initialConfig,
      runtimePlan: previewRuntimePlan(initialConfig),
      providerSecrets: [],
    },
    session: null,
    pendingChanges: [],
  };
  let secretRevision = 0;
  let sessionSecretRevision: number | null = null;
  let captionSession: CaptionSessionSnapshotV1 = {
    contractVersion: 1,
    snapshotRevision: 0,
    active: null,
    activeUnits: [],
    captions: [],
  };

  function emit(eventName: string, payload: unknown) {
    for (const listener of listeners.get(eventName) ?? []) {
      listener({ payload });
    }
  }

  function emitControl() {
    emit(RUNTIME_CONTROL_EVENT, structuredClone(snapshot));
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

  function pendingChanges(
    config: AppConfig,
  ): RuntimeControlSnapshot["pendingChanges"] {
    const selected = snapshot.session?.selected;

    if (!selected) {
      return [];
    }

    const pending: RuntimeControlSnapshot["pendingChanges"] = [];
    if (selected.audio.inputDeviceId !== config.audio.inputDeviceId) {
      pending.push("microphone");
    }
    if (
      selected.stt.provider !== config.stt.provider ||
      selected.stt.language !== config.stt.language ||
      selected.stt.model !== config.stt.model
    ) {
      pending.push("recognition");
    }
    if (
      selected.stt.provider === "openai" &&
      sessionSecretRevision !== secretRevision
    ) {
      pending.push("credential");
    }
    if (
      selected.osc.enabled !== config.osc.enabled ||
      selected.osc.host !== config.osc.host ||
      selected.osc.port !== config.osc.port
    ) {
      pending.push("chatboxOutput");
    }
    if (selected.publication.mode !== config.publication.mode) {
      pending.push("publication");
    }

    return pending;
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
      if (command === "start_runtime") {
        sessionSecretRevision = secretRevision;
        const selected = snapshot.desired.config;
        snapshot = {
          ...snapshot,
          revision: snapshot.revision + 1,
          runtime: { status: "running", timestampMs: 2 },
          session: {
            generation: 1,
            phase: "running",
            startedFromConfigRevision: snapshot.desired.revision,
            selected: {
              audio: structuredClone(selected.audio),
              stt: structuredClone(selected.stt),
              osc: structuredClone(selected.osc),
              publication: structuredClone(selected.publication),
            },
            runtimePlan: previewRuntimePlan(selected),
            credential: null,
            chatbox: {
              state: selected.osc.enabled ? "ready" : "disabled",
              host: selected.osc.host,
              port: selected.osc.port,
            },
            uploadsMicrophoneAudio: selected.stt.provider === "openai",
          },
        };
        publishCaptionSession({
          active: { generation: 1, streamId: "recognition-1-1" },
          activeUnits: [],
          captions: captionSession.captions,
        });
        emitControl();
      } else if (command === "stop_runtime") {
        sessionSecretRevision = null;
        snapshot = {
          ...snapshot,
          revision: snapshot.revision + 1,
          runtime: {
            status: "stopped",
            message: "Runtime stopped",
            timestampMs: snapshot.runtime.timestampMs + 1,
          },
          session: null,
          pendingChanges: [],
        };
        publishCaptionSession({
          active: null,
          activeUnits: [],
          captions: captionSession.captions.filter(
            (caption) => caption.state === "completed",
          ),
        });
        emitControl();
      } else if (command === "save_app_config") {
        const config = structuredClone(args?.["config"] as AppConfig);
        snapshot = {
          ...snapshot,
          revision: snapshot.revision + 1,
          desired: {
            ...snapshot.desired,
            revision: snapshot.desired.revision + 1,
            config,
            runtimePlan: previewRuntimePlan(config),
          },
          pendingChanges: pendingChanges(config),
        };
        emitControl();
      } else if (command === "save_provider_secret") {
        const secretArgument = args?.["secret"];
        const secret =
          typeof secretArgument === "string" ? secretArgument.trim() : "";
        secretRevision += 1;
        snapshot = {
          ...snapshot,
          revision: snapshot.revision + 1,
          desired: {
            ...snapshot.desired,
            providerSecrets: [
              {
                provider: "openai",
                configured: true,
                storage: "systemCredentialStore",
                displaySuffix: secret.slice(-4),
                error: null,
              },
            ],
          },
          pendingChanges: pendingChanges(snapshot.desired.config),
        };
        emitControl();
      } else if (command === "emit_mock_transcript") {
        const activeSession = snapshot.session;

        if (
          snapshot.runtime.status !== "running" ||
          activeSession?.selected.stt.provider !== "mock"
        ) {
          return Promise.reject(
            new Error(
              "Mock Transcript requires an active Mock runtime session.",
            ),
          );
        }

        const utteranceId = "fake-control-utterance";
        const active = captionSession.active;

        if (active === null) {
          return Promise.reject(new Error("Recognition stream is missing."));
        }
        const base = {
          generation: active.generation,
          streamId: active.streamId,
          unitId: utteranceId,
          lane: "source" as const,
          language: activeSession.selected.stt.language,
          provider: activeSession.selected.stt.provider,
          model: activeSession.selected.stt.model,
          unitStartedAtMs: 4,
          timestampMs: 4,
        };
        emit(RUNTIME_EVENTS.utteranceStarted, {
          id: "fake-control-start",
          generation: active.generation,
          streamId: active.streamId,
          utteranceId,
          timestampMs: 4,
        });
        publishCaptionSession({
          active,
          activeUnits: [{ unitId: utteranceId, startedAtMs: 4 }],
          captions: captionSession.captions,
        });
        publishCaptionSession({
          active,
          activeUnits: [{ unitId: utteranceId, startedAtMs: 4 }],
          captions: [
            {
              ...base,
              text: "Testing live caption preview...",
              revision: 1,
              state: "ongoing" as const,
            },
            ...captionSession.captions,
          ],
        });
        publishCaptionSession({
          active,
          activeUnits: [],
          captions: [
            {
              ...base,
              text: "Testing live caption preview from the mock runtime.",
              revision: 2,
              state: "completed" as const,
            },
            ...captionSession.captions.filter(
              (caption) => caption.state === "completed",
            ),
          ].slice(0, 5),
        });
      } else if (command === "get_caption_session_snapshot") {
        return Promise.resolve(structuredClone(captionSession) as Result);
      }

      return Promise.resolve(structuredClone(snapshot) as Result);
    },
  };
}

const cases: readonly Readonly<{
  name: string;
  create: () => RuntimeBackend;
}>[] = [
  { name: "PreviewBackend", create: createPreviewBackend },
  {
    name: "TauriBackend",
    create: () => createTauriBackend(createControlBridge()),
  },
];

test("PreviewBackend OSC Test uses the session target until Stop", async () => {
  const backend = createPreviewBackend();
  const details: string[] = [];
  const unsubscribe = await backend.listen((event) => {
    if (
      event.type === "diagnostic" &&
      event.payload.code === "osc.test_simulated"
    ) {
      details.push(event.payload.detail ?? "");
    }
  });

  try {
    await backend.startRuntime();
    await backend.saveConfig({
      ...initialConfig,
      osc: { enabled: true, host: "192.0.2.30", port: 9012 },
    });
    await backend.runCommand("send_osc_test_message");
    await backend.stopRuntime();
    await backend.runCommand("send_osc_test_message");
  } finally {
    unsubscribe();
  }

  expect(details).toHaveLength(2);
  expect(details[0]).toContain("127.0.0.1:9000");
  expect(details[1]).toContain("192.0.2.30:9012");
});

describe.each(cases)("$name runtime control contract", ({ create }) => {
  test("returns and publishes an authoritative session snapshot on Start", async () => {
    const backend = create();
    const observed: RuntimeControlSnapshot[] = [];
    const unsubscribe = await backend.listenControl((snapshot) => {
      observed.push(snapshot);
    });

    const initial = await backend.getControlSnapshot();
    const started = await backend.startRuntime();
    unsubscribe();

    expect(initial.contractVersion).toBe(2);
    expect(initial.session).toBeNull();
    expect(started.session?.selected.stt.provider).toBe("openai");
    expect(observed.at(-1)?.revision).toBe(started.revision);
  });

  test("saves desired settings without mutating the active session", async () => {
    const backend = create();
    const started = await backend.startRuntime();
    const changed: AppConfig = {
      ...initialConfig,
      audio: { inputDeviceId: "next-device" },
      stt: { provider: "mock", language: "zh", model: "next-model" },
      osc: { enabled: false, host: "192.0.2.30", port: 9012 },
      publication: { mode: "live" },
      ui: { showPartial: false },
    };

    const saved = await backend.saveConfig(changed);

    expect(saved.desired.config).toEqual(changed);
    expect(saved.session).toEqual(started.session);
    expect(saved.pendingChanges).toEqual([
      "microphone",
      "recognition",
      "chatboxOutput",
      "publication",
    ]);
    expect(saved.desired.runtimePlan.publication.state).toBe("ready");

    const reverted = await backend.saveConfig({
      ...initialConfig,
      ui: { showPartial: false },
    });

    expect(reverted.desired.config.ui.showPartial).toBe(false);
    expect(reverted.session).toEqual(started.session);
    expect(reverted.pendingChanges).toEqual([]);
  });

  test("defers an OpenAI credential change until the next session", async () => {
    const backend = create();
    const started = await backend.startRuntime();

    const saved = await backend.saveProviderSecret("openai", "sk-test-abcd");

    expect(saved.desired.providerSecrets).toContainEqual({
      provider: "openai",
      configured: true,
      storage: "systemCredentialStore",
      displaySuffix: "abcd",
      error: null,
    });
    expect(saved.session).toEqual(started.session);
    expect(saved.pendingChanges).toContain("credential");
  });

  test("preserves an incompatible saved mode and returns the backend plan", async () => {
    const backend = create();
    const saved = await backend.saveConfig({
      ...initialConfig,
      publication: { mode: "live" },
    });

    expect(saved.desired.config.publication.mode).toBe("live");
    expect(saved.desired.runtimePlan.publication).toMatchObject({
      state: "incompatible",
      requestedMode: "live",
      supportedModes: ["completed"],
    });
  });

  test("uses the immutable Mock session metadata after desired settings change", async () => {
    const backend = create();
    const mockConfig: AppConfig = {
      ...initialConfig,
      stt: { provider: "mock", language: "ja", model: "mock-model" },
    };
    await backend.saveConfig(mockConfig);
    await backend.startRuntime();
    await backend.saveConfig(initialConfig);
    const events: RuntimeEvent[] = [];
    const unsubscribe = await backend.listen((event) => {
      events.push(event);
    });

    try {
      await backend.runCommand("emit_mock_transcript");
    } finally {
      unsubscribe();
    }

    const completed = events
      .filter((event) => event.type === "captionSessionChanged")
      .flatMap((event) => event.payload.captions)
      .find((caption) => caption.state === "completed");
    expect(completed).toMatchObject({
      provider: "mock",
      language: "ja",
    });
  });

  test("does not make an OpenAI key change pending for an active Mock session", async () => {
    const backend = create();
    await backend.saveConfig({
      ...initialConfig,
      stt: { provider: "mock", language: "en", model: "mock-model" },
    });
    await backend.startRuntime();

    const saved = await backend.saveProviderSecret("openai", "sk-test-efgh");

    expect(saved.session?.selected.stt.provider).toBe("mock");
    expect(saved.pendingChanges).not.toContain("credential");
  });

  test("restores the active session and pending changes from a pull snapshot", async () => {
    const backend = create();
    const started = await backend.startRuntime();
    await backend.saveConfig({
      ...initialConfig,
      audio: { inputDeviceId: "next-device" },
    });

    const reloaded = await backend.getControlSnapshot();

    expect(reloaded.session).toEqual(started.session);
    expect(reloaded.pendingChanges).toEqual(["microphone"]);
    expect(reloaded.desired.config.audio.inputDeviceId).toBe("next-device");
  });

  test("keeps revisions monotonic and clears session drift on Stop", async () => {
    const backend = create();
    const saved = await backend.saveConfig({
      ...initialConfig,
      ui: { showPartial: false },
    });
    const started = await backend.startRuntime();
    const changed = await backend.saveConfig({
      ...initialConfig,
      audio: { inputDeviceId: "next-device" },
    });

    expect(started.revision).toBeGreaterThan(saved.revision);
    expect(changed.pendingChanges).toEqual(["microphone"]);

    const stopped = await backend.stopRuntime();
    expect(stopped.revision).toBeGreaterThan(changed.revision);
    expect(stopped.runtime.status).toBe("stopped");
    expect(stopped.session).toBeNull();
    expect(stopped.pendingChanges).toEqual([]);
  });
});
