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
import type { CaptionAggregateSnapshot } from "../runtime/captionAggregate";
import {
  RUNTIME_CONTROL_CONTRACT_VERSION,
  type CredentialId,
  type RuntimeControlSnapshot,
  type RuntimePendingGenerationChange,
} from "../runtime/runtimeControl";

const initialConfig: AppConfig = {
  schemaVersion: APP_CONFIG_SCHEMA_VERSION,
  audio: { inputDeviceId: null },
  recognition: {
    path: "openai/gpt-transcribe",
    expectedLanguages: ["en"],
  },
  translation: null,
  osc: { enabled: true, host: "127.0.0.1", port: 9000 },
  publication: { mode: "completed", content: "sourceOnly" },
  ui: { showOngoingPreview: true },
};

function createControlTauriIpcBridge(): TauriIpcBridge {
  const listeners = new Map<
    string,
    Set<(event: Readonly<{ payload: unknown }>) => void>
  >();
  let snapshot: RuntimeControlSnapshot = {
    contractVersion: RUNTIME_CONTROL_CONTRACT_VERSION,
    revision: 1,
    runtimeStatus: { status: "idle", timestampMs: 1 },
    desired: {
      revision: 1,
      config: initialConfig,
      captionPipelinePlan: previewCaptionPipelinePlan(initialConfig),
      credentials: [
        { state: "unconfigured", id: "openai" },
        { state: "unconfigured", id: "customTranslation" },
      ],
    },
    generation: null,
    pendingGenerationChanges: [],
  };
  const credentialRevisions: Record<CredentialId, number> = {
    openai: 0,
    customTranslation: 0,
  };
  let captionAggregate: CaptionAggregateSnapshot = {
    contractVersion: 1,
    snapshotRevision: 0,
    activeStream: null,
    openSourceUnits: [],
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

  function pendingGenerationChanges(
    config: AppConfig,
  ): RuntimeControlSnapshot["pendingGenerationChanges"] {
    const selection = snapshot.generation?.selection;

    if (!selection) {
      return [];
    }

    const pending: RuntimePendingGenerationChange[] = [];
    if (selection.audio.inputDeviceId !== config.audio.inputDeviceId) {
      pending.push("microphone");
    }
    if (
      selection.recognition.expectedLanguages.length !==
        config.recognition.expectedLanguages.length ||
      selection.recognition.expectedLanguages.some(
        (language, index) =>
          language !== config.recognition.expectedLanguages[index],
      ) ||
      selection.recognition.path !== config.recognition.path
    ) {
      pending.push("recognition");
    }
    const desiredTranslation =
      config.publication.content === "sourceOnly" ? null : config.translation;
    if (
      selection.publication.content === config.publication.content &&
      JSON.stringify(selection.translation) !==
        JSON.stringify(desiredTranslation)
    ) {
      pending.push("translation");
    }
    if (
      snapshot.generation?.credentials.some(
        (credential) =>
          credential.revision !== credentialRevisions[credential.id],
      )
    ) {
      pending.push("credential");
    }
    if (
      selection.osc.enabled !== config.osc.enabled ||
      selection.osc.host !== config.osc.host ||
      selection.osc.port !== config.osc.port
    ) {
      pending.push("chatboxOutput");
    }
    if (
      selection.publication.mode !== config.publication.mode ||
      selection.publication.content !== config.publication.content
    ) {
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
      if (command === TAURI_COMMANDS.startRuntime) {
        if (
          snapshot.desired.captionPipelinePlan.publication.state ===
          "incompatible"
        ) {
          return Promise.reject(
            new Error(
              "The selected recognition path and publication mode are incompatible.",
            ),
          );
        }
        if (snapshot.desired.captionPipelinePlan.translation !== null) {
          return Promise.reject(
            new Error(
              "The selected Translation path is not implemented yet (translation.module_unavailable).",
            ),
          );
        }
        const selected = snapshot.desired.config;
        snapshot = {
          ...snapshot,
          revision: snapshot.revision + 1,
          runtimeStatus: { status: "running", timestampMs: 2 },
          generation: {
            id: 1,
            phase: "running",
            startedFromConfigRevision: snapshot.desired.revision,
            selection: {
              audio: structuredClone(selected.audio),
              recognition: structuredClone(selected.recognition),
              translation: null,
              osc: structuredClone(selected.osc),
              publication: structuredClone(selected.publication),
            },
            captionPipelinePlan: previewCaptionPipelinePlan(selected),
            credentials: [
              {
                id: "openai",
                storage: "systemCredentialStore",
                displaySuffix: null,
                revision: credentialRevisions.openai,
              },
            ],
            chatboxPublication: {
              state: selected.osc.enabled ? "ready" : "disabled",
              host: selected.osc.host,
              port: selected.osc.port,
            },
            uploadsMicrophoneAudio: true,
            uploadsSourceText: false,
          },
        };
        emitCaptionAggregateUpdate({
          activeStream: { generation: 1, streamId: "recognition-1-1" },
          openSourceUnits: [],
          captions: captionAggregate.captions,
        });
        emitControl();
      } else if (command === TAURI_COMMANDS.stopRuntime) {
        snapshot = {
          ...snapshot,
          revision: snapshot.revision + 1,
          runtimeStatus: {
            status: "stopped",
            message: "Runtime stopped",
            timestampMs: snapshot.runtimeStatus.timestampMs + 1,
          },
          generation: null,
          pendingGenerationChanges: [],
        };
        emitCaptionAggregateUpdate({
          activeStream: null,
          openSourceUnits: [],
          captions: captionAggregate.captions.filter(
            (caption) => caption.state === "completed",
          ),
        });
        emitControl();
      } else if (command === TAURI_COMMANDS.saveAppConfig) {
        const config = structuredClone(args?.["config"] as AppConfig);
        snapshot = {
          ...snapshot,
          revision: snapshot.revision + 1,
          desired: {
            ...snapshot.desired,
            revision: snapshot.desired.revision + 1,
            config,
            captionPipelinePlan: previewCaptionPipelinePlan(config),
          },
          pendingGenerationChanges: pendingGenerationChanges(config),
        };
        emitControl();
      } else if (command === TAURI_COMMANDS.saveCredential) {
        const id = args?.["id"] as CredentialId;
        const secretArgument = args?.["secret"];
        const secret =
          typeof secretArgument === "string" ? secretArgument.trim() : "";
        credentialRevisions[id] += 1;
        snapshot = {
          ...snapshot,
          revision: snapshot.revision + 1,
          desired: {
            ...snapshot.desired,
            credentials: snapshot.desired.credentials.map((credential) =>
              credential.id === id
                ? {
                    state: "configured" as const,
                    id,
                    storage: "systemCredentialStore" as const,
                    displaySuffix: secret.slice(-4),
                  }
                : credential,
            ),
          },
          pendingGenerationChanges: pendingGenerationChanges(
            snapshot.desired.config,
          ),
        };
        emitControl();
      } else if (command === TAURI_COMMANDS.getCaptionAggregateSnapshot) {
        return Promise.resolve(structuredClone(captionAggregate) as Result);
      }

      return Promise.resolve(structuredClone(snapshot) as Result);
    },
  };
}

const cases: readonly Readonly<{
  name: string;
  create: () => AppGateway;
}>[] = [
  { name: "PreviewAppGateway", create: createPreviewAppGateway },
  {
    name: "TauriAppGateway",
    create: () => createTauriAppGateway(createControlTauriIpcBridge()),
  },
];

test("PreviewAppGateway OSC Test uses the generation target until Stop", async () => {
  const gateway = createPreviewAppGateway();
  const details: string[] = [];
  const unsubscribe = await gateway.subscribeRuntimeEvents((event) => {
    if (
      event.type === "diagnostic" &&
      event.payload.code === "osc.test_simulated"
    ) {
      details.push(event.payload.detail ?? "");
    }
  });

  try {
    await gateway.startRuntime();
    await gateway.saveAppConfig({
      ...initialConfig,
      osc: { enabled: true, host: "192.0.2.30", port: 9012 },
    });
    await gateway.sendOscTestMessage();
    await gateway.stopRuntime();
    await gateway.sendOscTestMessage();
  } finally {
    unsubscribe();
  }

  expect(details).toHaveLength(2);
  expect(details[0]).toContain("127.0.0.1:9000");
  expect(details[1]).toContain("192.0.2.30:9012");
});

test("TauriAppGateway rejects an invalid runtime-control pull", async () => {
  const gateway = createTauriAppGateway({
    listen() {
      return Promise.resolve(() => undefined);
    },
    invoke<Result>() {
      return Promise.resolve({ contractVersion: 4 } as Result);
    },
  });

  await expect(gateway.getRuntimeControlSnapshot()).rejects.toThrow(
    "Invalid runtime control payload at $.contractVersion: expected 2.",
  );
});

test.each([
  ["Start", (gateway: AppGateway) => gateway.startRuntime()],
  ["Stop", (gateway: AppGateway) => gateway.stopRuntime()],
  [
    "config save",
    (gateway: AppGateway) => gateway.saveAppConfig(initialConfig),
  ],
  [
    "credential save",
    (gateway: AppGateway) => gateway.saveCredential("openai", "sk-test-abcd"),
  ],
  [
    "credential delete",
    (gateway: AppGateway) => gateway.deleteCredential("openai"),
  ],
])("TauriAppGateway decodes the %s control result", async (_name, invoke) => {
  const gateway = createTauriAppGateway({
    listen() {
      return Promise.resolve(() => undefined);
    },
    invoke<Result>() {
      return Promise.resolve({ contractVersion: 4 } as Result);
    },
  });

  await expect(invoke(gateway)).rejects.toThrow(
    "Invalid runtime control payload at $.contractVersion: expected 2.",
  );
});

test("TauriAppGateway decodes runtime-control pushes before delivery", async () => {
  let deliver: ((event: Readonly<{ payload: unknown }>) => void) | undefined;
  const gateway = createTauriAppGateway({
    listen(eventName, listener) {
      if (eventName === RUNTIME_CONTROL_EVENT) {
        deliver = listener;
      }

      return Promise.resolve(() => undefined);
    },
    invoke<Result>() {
      return Promise.resolve(undefined as Result);
    },
  });
  const received: RuntimeControlSnapshot[] = [];
  const unsubscribe = await gateway.subscribeRuntimeControlSnapshots(
    (snapshot) => {
      received.push(snapshot);
    },
  );

  expect(() => deliver?.({ payload: { contractVersion: 4 } })).toThrow(
    "Invalid runtime control payload at $.contractVersion: expected 2.",
  );
  expect(received).toEqual([]);
  unsubscribe();
});

describe.each(cases)("$name runtime control contract", ({ create }) => {
  test("returns and publishes an authoritative generation snapshot on Start", async () => {
    const gateway = create();
    const observed: RuntimeControlSnapshot[] = [];
    const unsubscribe = await gateway.subscribeRuntimeControlSnapshots(
      (snapshot) => {
        observed.push(snapshot);
      },
    );

    const initial = await gateway.getRuntimeControlSnapshot();
    const started = await gateway.startRuntime();
    unsubscribe();

    expect(initial.contractVersion).toBe(2);
    expect(initial.generation).toBeNull();
    expect(started.generation?.selection.recognition.path).toBe(
      "openai/gpt-transcribe",
    );
    expect(observed.at(-1)?.revision).toBe(started.revision);
  });

  test("saves desired settings without mutating the active generation", async () => {
    const gateway = create();
    const started = await gateway.startRuntime();
    const changed: AppConfig = {
      ...initialConfig,
      audio: { inputDeviceId: "next-device" },
      recognition: {
        path: "openai/gpt-live-transcribe",
        expectedLanguages: ["zh", "en"],
      },
      osc: { enabled: false, host: "192.0.2.30", port: 9012 },
      publication: { mode: "live", content: "sourceOnly" },
      ui: { showOngoingPreview: false },
    };

    const saved = await gateway.saveAppConfig(changed);

    expect(saved.desired.config).toEqual(changed);
    expect(saved.generation).toEqual(started.generation);
    expect(saved.pendingGenerationChanges).toEqual([
      "microphone",
      "recognition",
      "chatboxOutput",
      "publication",
    ]);
    expect(saved.desired.captionPipelinePlan.publication.state).toBe(
      "compatible",
    );

    const reverted = await gateway.saveAppConfig({
      ...initialConfig,
      ui: { showOngoingPreview: false },
    });

    expect(reverted.desired.config.ui.showOngoingPreview).toBe(false);
    expect(reverted.generation).toEqual(started.generation);
    expect(reverted.pendingGenerationChanges).toEqual([]);
  });

  test("defers an OpenAI credential change until the next generation", async () => {
    const gateway = create();
    const started = await gateway.startRuntime();

    const saved = await gateway.saveCredential("openai", "sk-test-abcd");

    expect(saved.desired.credentials).toContainEqual({
      state: "configured",
      id: "openai",
      storage: "systemCredentialStore",
      displaySuffix: "abcd",
    });
    expect(saved.generation).toEqual(started.generation);
    expect(saved.pendingGenerationChanges).toContain("credential");
  });

  test("keeps dormant Translation settings and credentials outside the active generation", async () => {
    const gateway = create();
    const dormantConfig: AppConfig = {
      ...initialConfig,
      translation: {
        path: "openai/responses-completed-text",
        target: "zh-Hans",
        endpoint: {
          kind: "custom",
          apiBaseUrl: "https://example.com/v1",
        },
      },
    };
    await gateway.saveAppConfig(dormantConfig);
    const started = await gateway.startRuntime();

    const saved = await gateway.saveCredential(
      "customTranslation",
      "custom-test-abcd",
    );

    expect(started.generation?.selection.translation).toBeNull();
    expect(started.generation?.credentials.map(({ id }) => id)).toEqual([
      "openai",
    ]);
    expect(saved.desired.config.translation).toEqual(dormantConfig.translation);
    expect(saved.pendingGenerationChanges).toEqual([]);
  });

  test("rejects active Translation before creating a generation", async () => {
    const gateway = create();
    const activeConfig: AppConfig = {
      ...initialConfig,
      translation: {
        path: "openai/responses-completed-text",
        target: "zh-Hans",
        endpoint: { kind: "official" },
      },
      publication: { mode: "completed", content: "translationOnly" },
    };
    await gateway.saveAppConfig(activeConfig);

    await expect(gateway.startRuntime()).rejects.toThrow(
      /translation\.module_unavailable/u,
    );

    const retained = await gateway.getRuntimeControlSnapshot();
    expect(retained.desired.config).toEqual(activeConfig);
    expect(retained.generation).toBeNull();
  });

  test("preserves an incompatible saved mode and returns the gateway plan", async () => {
    const gateway = create();
    const saved = await gateway.saveAppConfig({
      ...initialConfig,
      publication: { mode: "live", content: "sourceOnly" },
    });

    expect(saved.desired.config.publication.mode).toBe("live");
    expect(saved.desired.captionPipelinePlan.publication).toMatchObject({
      state: "incompatible",
      requestedMode: "live",
      supportedModes: ["completed"],
    });
  });

  test("rejects an incompatible Start without rewriting the desired mode", async () => {
    const gateway = create();
    await gateway.saveAppConfig({
      ...initialConfig,
      publication: { mode: "live", content: "sourceOnly" },
    });

    await expect(gateway.startRuntime()).rejects.toThrow(/incompatible/u);

    const retained = await gateway.getRuntimeControlSnapshot();
    expect(retained.desired.config.publication.mode).toBe("live");
    expect(retained.desired.captionPipelinePlan.publication).toMatchObject({
      state: "incompatible",
      requestedMode: "live",
      supportedModes: ["completed"],
    });
    expect(retained.generation).toBeNull();
  });

  test("restores the active generation and pending changes from a pull snapshot", async () => {
    const gateway = create();
    const started = await gateway.startRuntime();
    await gateway.saveAppConfig({
      ...initialConfig,
      audio: { inputDeviceId: "next-device" },
    });

    const reloaded = await gateway.getRuntimeControlSnapshot();

    expect(reloaded.generation).toEqual(started.generation);
    expect(reloaded.pendingGenerationChanges).toEqual(["microphone"]);
    expect(reloaded.desired.config.audio.inputDeviceId).toBe("next-device");
  });

  test("keeps revisions monotonic and clears generation drift on Stop", async () => {
    const gateway = create();
    const saved = await gateway.saveAppConfig({
      ...initialConfig,
      ui: { showOngoingPreview: false },
    });
    const started = await gateway.startRuntime();
    const changed = await gateway.saveAppConfig({
      ...initialConfig,
      audio: { inputDeviceId: "next-device" },
    });

    expect(started.revision).toBeGreaterThan(saved.revision);
    expect(changed.pendingGenerationChanges).toEqual(["microphone"]);

    const stopped = await gateway.stopRuntime();
    expect(stopped.revision).toBeGreaterThan(changed.revision);
    expect(stopped.runtimeStatus.status).toBe("stopped");
    expect(stopped.generation).toBeNull();
    expect(stopped.pendingGenerationChanges).toEqual([]);
  });
});
