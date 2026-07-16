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
  let config = structuredClone(PREVIEW_DEFAULT_CONFIG);
  let openAiSecretSuffix: string | null = null;
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

  function emitMockTranscript() {
    const utteranceId = eventId("utterance");
    const timestampMs = Date.now();
    const transcriptBase = {
      utteranceId,
      language: config.stt.language,
      provider: config.stt.provider,
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

  function secretStatusFor(provider: SttProvider): ProviderSecretStatus {
    if (provider === "openai") {
      return openAiSecretStatus();
    }

    return {
      provider,
      configured: false,
      storage: null,
      displaySuffix: null,
      error: null,
    };
  }

  return {
    listen(eventListener: RuntimeEventListener) {
      const subscription = { listener: eventListener };
      subscriptions.add(subscription);

      return Promise.resolve(() => {
        subscriptions.delete(subscription);
      });
    },

    runCommand(command: RuntimeCommand) {
      if (command === "start_runtime") {
        if (["starting", "running", "stopping"].includes(latestStatus.status)) {
          return Promise.reject(
            new Error("The browser preview runtime is already active."),
          );
        }

        emitStatus("starting", "Starting browser preview runtime");
        emitStatus("running", "Browser preview runtime is running");
      } else if (command === "stop_runtime") {
        if (
          latestStatus.status === "idle" ||
          latestStatus.status === "stopped"
        ) {
          emitStatus("stopped", "Browser preview runtime is already stopped");
          return Promise.resolve();
        }

        emitStatus("stopping", "Stopping browser preview runtime");
        emitStatus("stopped", "Browser preview runtime stopped");
        emitDiagnostic(
          "runtime",
          "runtime.stopped",
          "Runtime stopped",
          "Browser preview capture has been released.",
        );
      } else if (command === "emit_mock_transcript") {
        emitMockTranscript();
      } else {
        emitDiagnostic(
          "osc",
          "osc.test_simulated",
          "OSC test simulated",
          "Desktop-only command was simulated for UI preview.",
        );
      }

      return Promise.resolve();
    },

    getRuntimeStatus() {
      return Promise.resolve({ ...latestStatus });
    },

    getConfig() {
      return Promise.resolve(structuredClone(config));
    },

    saveConfig(nextConfig: AppConfig) {
      config = structuredClone(nextConfig);
      return Promise.resolve(structuredClone(config));
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

    getProviderSecretStatus(provider: SttProvider) {
      return Promise.resolve(secretStatusFor(provider));
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
      return Promise.resolve(openAiSecretStatus());
    },

    deleteProviderSecret(provider: SttProvider) {
      if (provider === "openai") {
        openAiSecretSuffix = null;
      }

      return Promise.resolve(secretStatusFor(provider));
    },
  };
}
