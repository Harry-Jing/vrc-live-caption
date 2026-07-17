// Browser preview implementation of the runtime backend. It exists so the UI
// can be developed in a plain browser (`pnpm dev`) without the Tauri shell.
// Simulated activity is delivered through the same event handlers as the real
// backend, so preview mode exercises the actual caption state machine.

import type { RuntimeBackend, RuntimeEventListener } from "./backend";
import {
  APP_CONFIG_SCHEMA_VERSION,
  type AppConfig,
  type DiagnosticCategory,
  type ProviderSecretStatus,
  type RuntimeCommand,
  type RuntimeControlSnapshot,
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
  ui: {
    showPartial: true,
  },
};

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

  function emitMockTranscript(): Error | null {
    if (
      latestStatus.status !== "running" ||
      session?.selected.stt.provider !== "mock"
    ) {
      return new Error(
        "Mock Transcript requires an active Mock runtime session.",
      );
    }

    const utteranceId = eventId("utterance");
    const timestampMs = Date.now();
    const transcriptBase = {
      utteranceId,
      language: session.selected.stt.language,
      provider: session.selected.stt.provider,
      timestampMs,
    };

    emit({
      type: "utteranceStarted",
      payload: {
        id: eventId("utterance-start"),
        utteranceId,
        timestampMs,
      },
    });
    emit({
      type: "transcriptPartial",
      payload: {
        ...transcriptBase,
        id: eventId("transcript"),
        kind: "partial",
        text: "Testing live caption preview...",
        revision: 1,
      },
    });
    emit({
      type: "transcriptFinal",
      payload: {
        ...transcriptBase,
        id: eventId("transcript"),
        kind: "final",
        text: "Testing live caption preview from the mock runtime.",
        revision: 2,
      },
    });
    emitDiagnostic(
      "stt",
      "stt.mock_transcript_emitted",
      "Mock transcript emitted",
      "The UI received normalized partial and final transcript events.",
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
      contractVersion: 1,
      revision: controlRevision,
      runtime: { ...latestStatus },
      desired: {
        revision: configRevision,
        config: structuredClone(config),
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
    };

    return {
      generation: nextGeneration,
      phase,
      startedFromConfigRevision: configRevision,
      selected,
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

    nextGeneration += 1;
    sessionSecretRevision =
      config.stt.provider === "openai" ? secretRevision : null;
    session = createSession("starting");
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
