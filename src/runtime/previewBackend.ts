// Browser preview implementation of the runtime backend. It exists so the UI
// can be developed in a plain browser (`pnpm dev`) without the Tauri shell.
// Simulated activity is delivered through the same event handlers as the real
// backend, so preview mode exercises the actual caption state machine.

import type { RuntimeBackend, RuntimeEventListener } from "./backend";
import {
  APP_CONFIG_SCHEMA_VERSION,
  type AppConfig,
  type CaptionSessionSnapshotV1,
  type CaptionSnapshotV1,
  type DiagnosticCategory,
  type ProviderSecretStatus,
  type RuntimeCommand,
  type RuntimeControlSnapshot,
  type RuntimePlan,
  type RuntimeSession,
  type RuntimeStatus,
  type RuntimeStatusEvent,
  type SttProvider,
} from "./types";

const PREVIEW_DEFAULT_CONFIG: AppConfig = {
  schemaVersion: APP_CONFIG_SCHEMA_VERSION,
  audio: {
    inputDeviceId: null,
  },
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
  publication: {
    mode: "completed",
  },
  ui: {
    showPartial: true,
  },
};

export function previewRuntimePlan(config: AppConfig): RuntimePlan {
  const recognition =
    config.stt.provider === "openai"
      ? {
          path: "openAiBounded" as const,
          inputShape: "completedAudioUnits" as const,
          boundaryOwner: "application" as const,
          unitBehavior: "unitBased" as const,
          lanes: [
            {
              lane: "source" as const,
              updates: "completedOnly" as const,
              revisions: "appendOnly" as const,
            },
          ],
        }
      : config.stt.model === "mock-bounded"
        ? {
            path: "mockBounded" as const,
            inputShape: "completedAudioUnits" as const,
            boundaryOwner: "application" as const,
            unitBehavior: "unitBased" as const,
            lanes: [
              {
                lane: "source" as const,
                updates: "completedOnly" as const,
                revisions: "appendOnly" as const,
              },
            ],
          }
        : config.stt.model === "mock-ongoing-only"
          ? {
              path: "mockOngoingOnly" as const,
              inputShape: "continuousAudioFrames" as const,
              boundaryOwner: "none" as const,
              unitBehavior: "unitless" as const,
              lanes: [
                {
                  lane: "source" as const,
                  updates: "ongoingOnly" as const,
                  revisions: "revisableFullSnapshot" as const,
                },
              ],
            }
          : {
              path: "mockOngoingCompleted" as const,
              inputShape: "continuousAudioFrames" as const,
              boundaryOwner: "provider" as const,
              unitBehavior: "unitBased" as const,
              lanes: [
                {
                  lane: "source" as const,
                  updates: "ongoingAndCompleted" as const,
                  revisions: "revisableFullSnapshot" as const,
                },
              ],
            };
  const sourceUpdates = recognition.lanes[0]?.updates;
  if (!sourceUpdates) {
    throw new Error("Preview recognition profile must produce a source lane.");
  }
  const compatible =
    config.publication.mode === "completed"
      ? sourceUpdates !== "ongoingOnly"
      : sourceUpdates !== "completedOnly";

  return {
    recognition,
    publication: compatible
      ? {
          state: "ready",
          mode: config.publication.mode,
          policy:
            config.publication.mode === "completed"
              ? { policy: "completed" }
              : recognition.unitBehavior === "unitBased"
                ? { policy: "liveUnit", observationWindowMs: 1000 }
                : { policy: "liveUnitless", firstNonEmptyDelayMs: 1000 },
          selectedLanes: ["source"],
        }
      : {
          state: "incompatible",
          requestedMode: config.publication.mode,
          selectedLanes: ["source"],
          reason: { reason: "modeUnsupported", lanes: ["source"] },
          supportedModes: [
            config.publication.mode === "completed" ? "live" : "completed",
          ],
        },
  };
}

export function createPreviewBackend(): RuntimeBackend {
  const subscriptions = new Set<Readonly<{ listener: RuntimeEventListener }>>();
  const controlSubscriptions = new Set<
    Readonly<{ listener: (snapshot: RuntimeControlSnapshot) => void }>
  >();
  let config = structuredClone(PREVIEW_DEFAULT_CONFIG);
  let openAiSecretSuffix: string | null = null;
  let secretRevision = 0;
  let configRevision = 1;
  let controlRevision = 1;
  let nextGeneration = 0;
  let session: RuntimeSession | null = null;
  let sessionSecretRevision: number | null = null;
  let nextEventNumber = 1;
  let captionSession: CaptionSessionSnapshotV1 = {
    contractVersion: 1,
    snapshotRevision: 0,
    active: null,
    activeUnits: [],
    captions: [],
  };
  let latestStatus: RuntimeStatusEvent = {
    status: "idle",
    message: "Runtime is idle",
    timestampMs: Date.now(),
  };

  function eventId(prefix: string) {
    nextEventNumber += 1;
    return `${prefix}-preview-${String(nextEventNumber)}`;
  }

  function emit(event: Parameters<RuntimeEventListener>[0]) {
    for (const subscription of subscriptions) {
      subscription.listener(event);
    }
  }

  function emitStatus(status: RuntimeStatus, message: string) {
    latestStatus = { status, message, timestampMs: Date.now() };
    publishControl();
    emit({ type: "status", payload: latestStatus });
  }

  function emitDiagnostic(
    category: DiagnosticCategory,
    code: string,
    message: string,
    detail: string,
  ) {
    emit({
      type: "diagnostic",
      payload: {
        id: eventId("diagnostic"),
        category,
        severity: "info",
        code,
        message,
        detail,
        timestampMs: Date.now(),
      },
    });
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
    emit({
      type: "captionSessionChanged",
      payload: structuredClone(captionSession),
    });
  }

  function emitMockTranscript(): Error | null {
    if (
      latestStatus.status !== "running" ||
      session?.selected.stt.provider !== "mock"
    ) {
      return new Error(
        "Mock Transcript requires an active Mock runtime session.",
      );
    }

    const active = captionSession.active;

    if (active === null) {
      return new Error("Mock runtime has no active recognition stream.");
    }

    if (session.selected.stt.model === "mock-ongoing-only") {
      const selectedStt = session.selected.stt;
      const latestRevision =
        captionSession.captions.find(
          (caption) =>
            caption.generation === active.generation &&
            caption.streamId === active.streamId &&
            caption.unitId === null &&
            caption.lane === "source",
        )?.revision ?? 0;
      const scriptedTexts = [
        "Testing live caption preview...",
        "Testing live caption preview from the ongoing-only mock runtime.",
      ];

      scriptedTexts.forEach((text, index) => {
        const caption: CaptionSnapshotV1 = {
          generation: active.generation,
          streamId: active.streamId,
          unitId: null,
          lane: "source",
          revision: latestRevision + index + 1,
          text,
          state: "ongoing",
          language: selectedStt.language,
          provider: selectedStt.provider,
          model: selectedStt.model,
          unitStartedAtMs: null,
          timestampMs: Date.now(),
        };
        publishCaptionSession({
          active,
          activeUnits: [],
          captions: [
            caption,
            ...captionSession.captions.filter(
              (candidate) => candidate.state === "completed",
            ),
          ],
        });
      });
      emitDiagnostic(
        "stt",
        "stt.mock_transcript_emitted",
        "Mock transcript emitted",
        "The UI received full ongoing unitless caption snapshots without a completion.",
      );

      return null;
    }

    const utteranceId = eventId("utterance");
    const timestampMs = Date.now();
    const captionBase = {
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

    emit({
      type: "utteranceStarted",
      payload: {
        id: eventId("utterance-start"),
        generation: active.generation,
        streamId: active.streamId,
        utteranceId,
        timestampMs,
      },
    });
    publishCaptionSession({
      active,
      activeUnits: [{ unitId: utteranceId, startedAtMs: timestampMs }],
      captions: captionSession.captions.filter(
        (caption) => caption.state === "completed",
      ),
    });
    if (session.selected.stt.model === "mock-bounded") {
      const completed: CaptionSnapshotV1 = {
        ...captionBase,
        revision: 1,
        text: "Testing bounded caption preview from the mock runtime.",
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
        "The UI received one completed bounded caption snapshot.",
      );

      return null;
    }
    const ongoing: CaptionSnapshotV1 = {
      ...captionBase,
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
      ...captionBase,
      revision: 2,
      text: "Testing live caption preview from the mock runtime.",
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
      "The UI received ongoing and completed caption-session snapshots.",
    );

    return null;
  }

  function openAiSecretStatus(): ProviderSecretStatus {
    return {
      provider: "openai",
      configured: openAiSecretSuffix !== null,
      storage: openAiSecretSuffix !== null ? "systemCredentialStore" : null,
      displaySuffix: openAiSecretSuffix,
      error: null,
    };
  }

  function controlSnapshot(): RuntimeControlSnapshot {
    return {
      contractVersion: 2,
      revision: controlRevision,
      runtime: { ...latestStatus },
      desired: {
        revision: configRevision,
        config: structuredClone(config),
        runtimePlan: previewRuntimePlan(config),
        providerSecrets: [openAiSecretStatus()],
      },
      session: session ? structuredClone(session) : null,
      pendingChanges: pendingChanges(),
    };
  }

  function pendingChanges(): RuntimeControlSnapshot["pendingChanges"] {
    if (session === null) {
      return [];
    }

    const pending: RuntimeControlSnapshot["pendingChanges"] = [];

    if (session.selected.audio.inputDeviceId !== config.audio.inputDeviceId) {
      pending.push("microphone");
    }
    if (
      session.selected.stt.provider !== config.stt.provider ||
      session.selected.stt.language !== config.stt.language ||
      session.selected.stt.model !== config.stt.model
    ) {
      pending.push("recognition");
    }
    if (
      session.selected.stt.provider === "openai" &&
      sessionSecretRevision !== secretRevision
    ) {
      pending.push("credential");
    }
    if (
      session.selected.osc.enabled !== config.osc.enabled ||
      session.selected.osc.host !== config.osc.host ||
      session.selected.osc.port !== config.osc.port
    ) {
      pending.push("chatboxOutput");
    }
    if (session.selected.publication.mode !== config.publication.mode) {
      pending.push("publication");
    }

    return pending;
  }

  function oscConfigForTest() {
    return session?.selected.osc ?? config.osc;
  }

  function publishControl() {
    controlRevision += 1;
    const snapshot = controlSnapshot();

    for (const subscription of controlSubscriptions) {
      subscription.listener(structuredClone(snapshot));
    }
  }

  function createSession(phase: RuntimeSession["phase"]): RuntimeSession {
    const selected = {
      audio: structuredClone(config.audio),
      stt: structuredClone(config.stt),
      osc: structuredClone(config.osc),
      publication: structuredClone(config.publication),
    };

    return {
      generation: nextGeneration,
      phase,
      startedFromConfigRevision: configRevision,
      selected,
      runtimePlan: previewRuntimePlan(config),
      credential:
        selected.stt.provider === "openai" && openAiSecretSuffix !== null
          ? {
              provider: "openai",
              storage: "systemCredentialStore",
              displaySuffix: openAiSecretSuffix,
              revision: secretRevision,
            }
          : null,
      chatbox: {
        state: selected.osc.enabled ? "ready" : "disabled",
        host: selected.osc.host,
        port: selected.osc.port,
      },
      uploadsMicrophoneAudio: selected.stt.provider === "openai",
    };
  }

  function startRuntimeControl(): Promise<RuntimeControlSnapshot> {
    if (["starting", "running", "stopping"].includes(latestStatus.status)) {
      return Promise.reject(
        new Error("The browser preview runtime is already active."),
      );
    }

    const runtimePlan = previewRuntimePlan(config);
    if (runtimePlan.publication.state === "incompatible") {
      return Promise.reject(
        new Error(
          "The selected recognition path and publication mode are incompatible.",
        ),
      );
    }
    nextGeneration += 1;
    sessionSecretRevision =
      config.stt.provider === "openai" ? secretRevision : null;
    session = createSession("starting");
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
    emitStatus("starting", "Starting browser preview runtime");
    session = { ...session, phase: "running" };
    emitStatus("running", "Browser preview runtime is running");

    return Promise.resolve(controlSnapshot());
  }

  function stopRuntimeControl(): Promise<RuntimeControlSnapshot> {
    if (latestStatus.status === "idle" || latestStatus.status === "stopped") {
      session = null;
      sessionSecretRevision = null;
      emitStatus("stopped", "Browser preview runtime is already stopped");
      return Promise.resolve(controlSnapshot());
    }

    if (session) {
      session = { ...session, phase: "stopping" };
    }
    emitStatus("stopping", "Stopping browser preview runtime");
    publishCaptionSession({
      active: null,
      activeUnits: [],
      captions: captionSession.captions.filter(
        (caption) => caption.state === "completed",
      ),
    });
    session = null;
    sessionSecretRevision = null;
    emitStatus("stopped", "Browser preview runtime stopped");
    emitDiagnostic(
      "runtime",
      "runtime.stopped",
      "Runtime stopped",
      "Browser preview capture has been released.",
    );

    return Promise.resolve(controlSnapshot());
  }

  return {
    listen(eventListener: RuntimeEventListener) {
      const subscription = { listener: eventListener };
      subscriptions.add(subscription);

      return Promise.resolve(() => {
        subscriptions.delete(subscription);
      });
    },

    listenControl(listener) {
      const subscription = { listener };
      controlSubscriptions.add(subscription);

      return Promise.resolve(() => {
        controlSubscriptions.delete(subscription);
      });
    },

    runCommand(command: RuntimeCommand) {
      if (command === "start_runtime") {
        return startRuntimeControl().then(() => undefined);
      } else if (command === "stop_runtime") {
        return stopRuntimeControl().then(() => undefined);
      } else if (command === "emit_mock_transcript") {
        const error = emitMockTranscript();

        if (error) {
          return Promise.reject(error);
        }
      } else {
        const oscConfig = oscConfigForTest();

        emitDiagnostic(
          "osc",
          "osc.test_simulated",
          "OSC test simulated",
          `Desktop-only OSC test to ${oscConfig.host}:${String(oscConfig.port)} was simulated for UI preview.`,
        );
      }

      return Promise.resolve();
    },

    startRuntime: startRuntimeControl,

    stopRuntime: stopRuntimeControl,

    getControlSnapshot() {
      return Promise.resolve(controlSnapshot());
    },

    getCaptionSessionSnapshot() {
      return Promise.resolve(structuredClone(captionSession));
    },

    saveConfig(nextConfig: AppConfig) {
      config = structuredClone(nextConfig);
      configRevision += 1;
      publishControl();
      return Promise.resolve(controlSnapshot());
    },

    listAudioInputDevices() {
      return Promise.resolve([
        {
          id: "browser-preview-default",
          name: "Browser preview device",
          isDefault: true,
        },
      ]);
    },

    saveProviderSecret(provider: SttProvider, secret: string) {
      if (provider !== "openai") {
        return Promise.reject(new Error("Mock STT does not use an API key."));
      }

      const trimmed = secret.trim();

      if (!trimmed) {
        return Promise.reject(new Error("API key cannot be empty."));
      }

      // Mirrors the desktop backend's normalize_secret control-character rule.
      if (/\p{Cc}/u.test(trimmed)) {
        return Promise.reject(
          new Error("API key cannot contain control characters."),
        );
      }

      openAiSecretSuffix = trimmed.slice(-4);
      secretRevision += 1;
      publishControl();
      return Promise.resolve(controlSnapshot());
    },

    deleteProviderSecret(provider: SttProvider) {
      if (provider === "openai") {
        openAiSecretSuffix = null;
        secretRevision += 1;
        publishControl();
      }

      return Promise.resolve(controlSnapshot());
    },
  };
}
