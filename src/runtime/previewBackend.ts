// Browser preview implementation of the runtime backend. It exists so the UI
// can be developed in a plain browser (`pnpm dev`) without the Tauri shell.
// Simulated activity is delivered through the same event handlers as the real
// backend, so preview mode exercises the actual caption state machine.

import type { RuntimeBackend, RuntimeEventHandlers } from "./backend";
import type {
  AppConfig,
  ProviderSecretStatus,
  RuntimeCommand,
  RuntimeStatus,
  RuntimeStatusEvent,
  SttProvider,
} from "./types";

const PREVIEW_DEFAULT_CONFIG: AppConfig = {
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
    minIntervalMs: 1200,
  },
  ui: {
    showPartial: true,
  },
};

export function createPreviewBackend(): RuntimeBackend {
  let handlers: RuntimeEventHandlers | null = null;
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

  function emitStatus(status: RuntimeStatus, message: string) {
    latestStatus = { status, message, timestampMs: Date.now() };
    handlers?.onStatus(latestStatus);
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

    handlers?.onUtteranceStarted({
      id: eventId("utterance-start"),
      utteranceId,
      timestampMs,
    });
    handlers?.onTranscriptPartial({
      ...transcriptBase,
      id: eventId("transcript"),
      kind: "partial",
      text: "Testing live caption preview...",
      revision: 1,
    });
    handlers?.onTranscriptFinal({
      ...transcriptBase,
      id: eventId("transcript"),
      kind: "final",
      text: "Testing live caption preview from the browser UI.",
      revision: 2,
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
    listen(eventHandlers: RuntimeEventHandlers) {
      handlers = eventHandlers;

      return Promise.resolve(() => {
        if (handlers === eventHandlers) {
          handlers = null;
        }
      });
    },

    runCommand(command: RuntimeCommand) {
      if (command === "start_runtime") {
        emitStatus("running", "Browser preview runtime is running");
      } else if (command === "stop_runtime") {
        emitStatus("stopped", "Browser preview runtime stopped");
      } else if (command === "emit_mock_transcript") {
        emitMockTranscript();
      } else {
        handlers?.onDiagnostic({
          id: eventId("diagnostic"),
          category: "osc",
          severity: "info",
          code: "osc.test_simulated",
          message: "OSC test simulated",
          detail: "Desktop-only command was simulated for UI preview.",
          timestampMs: Date.now(),
        });
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
