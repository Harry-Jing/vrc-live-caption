// Browser preview implementation of the runtime backend. It exists so the UI
// can be developed in a plain browser (`pnpm dev`) without the Tauri shell.
// Simulated activity is delivered through the same event handlers as the real
// backend, so preview mode exercises the actual caption state machine.

import type { RuntimeBackend, RuntimeEventListener } from "./backend";
import { isActiveRuntimeStatus } from "./lifecycle";
import {
  APP_CONFIG_SCHEMA_VERSION,
  type AppConfig,
  type CaptionSessionSnapshotV1,
  type DiagnosticCategory,
  type ProviderSecretStatus,
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
    languages: ["en"],
    model: "gpt-transcribe",
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
    config.stt.model === "gpt-transcribe"
      ? {
          path: "openAiGptTranscribe" as const,
          inputShape: "continuousAudioFrames" as const,
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
      : {
          path: "openAiGptLiveTranscribe" as const,
          inputShape: "continuousAudioFrames" as const,
          boundaryOwner: "application" as const,
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
    config.publication.mode === "completed" ||
    sourceUpdates === "ongoingAndCompleted";

  return {
    recognition,
    publication: compatible
      ? {
          state: "ready",
          mode: config.publication.mode,
          policy:
            config.publication.mode === "completed"
              ? { policy: "completed" }
              : { policy: "liveUnit", observationWindowMs: 1000 },
          selectedLanes: ["source"],
        }
      : {
          state: "incompatible",
          requestedMode: config.publication.mode,
          selectedLanes: ["source"],
          reason: { reason: "modeUnsupported", lanes: ["source"] },
          supportedModes: ["completed"],
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
      contractVersion: 3,
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
      session.selected.stt.languages.length !== config.stt.languages.length ||
      session.selected.stt.languages.some(
        (language, index) => language !== config.stt.languages[index],
      ) ||
      session.selected.stt.model !== config.stt.model
    ) {
      pending.push("recognition");
    }
    if (sessionSecretRevision !== secretRevision) {
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
        openAiSecretSuffix !== null
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
      uploadsMicrophoneAudio: true,
    };
  }

  function startRuntimeControl(): Promise<RuntimeControlSnapshot> {
    if (isActiveRuntimeStatus(latestStatus.status)) {
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
    sessionSecretRevision = secretRevision;
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
    emit({
      type: "audioLevel",
      payload: {
        generation: nextGeneration,
        revision: 1,
        rmsDbfs: -24,
        peakDbfs: -6,
        clipping: false,
        gateOpen: true,
        timestampMs: Date.now(),
      },
    });

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

    runCommand() {
      const oscConfig = oscConfigForTest();

      emitDiagnostic(
        "osc",
        "osc.test_simulated",
        "OSC test simulated",
        `Desktop-only OSC test to ${oscConfig.host}:${String(oscConfig.port)} was simulated for UI preview.`,
      );

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

    probeAudioInput(request) {
      return Promise.resolve({
        sampleRate: 48_000,
        durationMs: request.durationMs,
        rmsDbfs: -24,
        peakDbfs: -6,
        clipping: false,
        gateOpen: true,
      });
    },

    saveProviderSecret(provider: SttProvider, secret: string) {
      void provider;
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
      void provider;
      openAiSecretSuffix = null;
      secretRevision += 1;
      publishControl();

      return Promise.resolve(controlSnapshot());
    },
  };
}
