import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import {
  RUNTIME_EVENTS,
  type AudioInputDevice,
  type AppConfig,
  type DiagnosticEvent,
  type ProviderSecretStatus,
  type RuntimeCommand,
  type RuntimeStatusEvent,
  type SttProvider,
  type TranscriptEvent,
  type UtteranceEndedEvent,
} from "./types";

const defaultConfig: AppConfig = {
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

function normalizeError(error: unknown) {
  if (typeof error === "string") {
    return error;
  }

  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof error.message === "string"
  ) {
    return error.message;
  }

  return "Action failed.";
}

function desktopAppRequiredError() {
  return new Error("This feature requires the Tauri desktop app.");
}

export function useRuntime() {
  const isTauriApp = isTauri();
  const browserPreviewEnabled = import.meta.env.DEV && !isTauriApp;
  const audioInputDevices = ref<AudioInputDevice[]>([]);
  const config = ref<AppConfig | null>(null);
  const openAiSecretStatus = ref<ProviderSecretStatus | null>(null);
  const runtimeStatus = ref<RuntimeStatusEvent>({
    status: "idle",
    message: "Runtime is idle",
    timestampMs: Date.now(),
  });
  const partialTranscript = ref<TranscriptEvent | null>(null);
  const finalTranscripts = ref<TranscriptEvent[]>([]);
  const diagnostics = ref<DiagnosticEvent[]>([]);
  const actionError = ref("");
  const isBusy = ref(false);
  const unlisteners: UnlistenFn[] = [];
  let isUnmounted = false;

  const latestFinalTranscript = computed<TranscriptEvent | null>(
    () => finalTranscripts.value.at(0) ?? null,
  );

  const activeCaptionText = computed(() => {
    if (config.value?.ui.showPartial && partialTranscript.value) {
      return partialTranscript.value.text;
    }

    return (
      latestFinalTranscript.value?.text ?? "Waiting for transcript events."
    );
  });

  async function runCommand(command: RuntimeCommand) {
    actionError.value = "";
    isBusy.value = true;

    try {
      if (!isTauriApp) {
        if (browserPreviewEnabled) {
          runBrowserPreviewCommand(command);
          return;
        }

        throw desktopAppRequiredError();
      }

      await invoke(command);
    } catch (error) {
      actionError.value = normalizeError(error);
    } finally {
      isBusy.value = false;
    }
  }

  async function saveConfig(nextConfig: AppConfig) {
    actionError.value = "";
    isBusy.value = true;

    try {
      if (!isTauriApp) {
        if (browserPreviewEnabled) {
          config.value = nextConfig;
          return;
        }

        throw desktopAppRequiredError();
      }

      config.value = await invoke<AppConfig>("save_app_config", {
        config: nextConfig,
      });
    } catch (error) {
      actionError.value = normalizeError(error);
    } finally {
      isBusy.value = false;
    }
  }

  async function loadProviderSecretStatus(provider: SttProvider) {
    actionError.value = "";

    try {
      if (!isTauriApp) {
        if (browserPreviewEnabled) {
          setBrowserPreviewSecretStatus(provider);
          return;
        }

        throw desktopAppRequiredError();
      }

      const status = await invoke<ProviderSecretStatus>(
        "get_provider_secret_status",
        { provider },
      );

      if (provider === "openai") {
        openAiSecretStatus.value = status;
      }
    } catch (error) {
      actionError.value = normalizeError(error);
    }
  }

  async function saveProviderSecret(provider: SttProvider, secret: string) {
    actionError.value = "";
    isBusy.value = true;

    try {
      if (!isTauriApp) {
        throw desktopAppRequiredError();
      }

      const status = await invoke<ProviderSecretStatus>(
        "save_provider_secret",
        { provider, secret },
      );

      if (provider === "openai") {
        openAiSecretStatus.value = status;
      }
    } catch (error) {
      actionError.value = normalizeError(error);
    } finally {
      isBusy.value = false;
    }
  }

  async function deleteProviderSecret(provider: SttProvider) {
    actionError.value = "";
    isBusy.value = true;

    try {
      if (!isTauriApp) {
        if (browserPreviewEnabled) {
          setBrowserPreviewSecretStatus(provider);
          return;
        }

        throw desktopAppRequiredError();
      }

      const status = await invoke<ProviderSecretStatus>(
        "delete_provider_secret",
        { provider },
      );

      if (provider === "openai") {
        openAiSecretStatus.value = status;
      }
    } catch (error) {
      actionError.value = normalizeError(error);
    } finally {
      isBusy.value = false;
    }
  }

  async function loadConfig() {
    actionError.value = "";

    try {
      if (!isTauriApp) {
        if (browserPreviewEnabled) {
          config.value = { ...defaultConfig };
          return;
        }

        throw desktopAppRequiredError();
      }

      config.value = await invoke<AppConfig>("get_app_config");
    } catch (error) {
      actionError.value = normalizeError(error);
    }
  }

  async function loadAudioInputDevices() {
    actionError.value = "";

    try {
      if (!isTauriApp) {
        if (browserPreviewEnabled) {
          audioInputDevices.value = [
            {
              id: "browser-preview-default",
              name: "Browser preview device",
              isDefault: true,
            },
          ];
          return;
        }

        throw desktopAppRequiredError();
      }

      audioInputDevices.value = await invoke<AudioInputDevice[]>(
        "list_audio_input_devices",
      );
    } catch (error) {
      actionError.value = normalizeError(error);
    }
  }

  // Only the utterance that owns the partial may clear it; a final or
  // utterance-end for an older utterance must not wipe a newer "Listening...".
  function clearPartialForUtterance(utteranceId: string) {
    if (partialTranscript.value?.utteranceId === utteranceId) {
      partialTranscript.value = null;
    }
  }

  function addUnlistener(unlisten: UnlistenFn) {
    if (isUnmounted) {
      unlisten();
      return;
    }

    unlisteners.push(unlisten);
  }

  function cleanupListeners() {
    for (const unlisten of unlisteners.splice(0)) {
      unlisten();
    }
  }

  async function registerRuntimeListeners() {
    if (!isTauriApp) {
      if (browserPreviewEnabled) {
        return;
      }

      throw desktopAppRequiredError();
    }

    try {
      addUnlistener(
        await listen<RuntimeStatusEvent>(RUNTIME_EVENTS.status, (event) => {
          runtimeStatus.value = event.payload;

          if (
            event.payload.status === "stopped" ||
            event.payload.status === "error"
          ) {
            partialTranscript.value = null;
          }
        }),
      );

      addUnlistener(
        await listen<TranscriptEvent>(
          RUNTIME_EVENTS.transcriptPartial,
          (event) => {
            partialTranscript.value = event.payload;
          },
        ),
      );

      addUnlistener(
        await listen<TranscriptEvent>(
          RUNTIME_EVENTS.transcriptFinal,
          (event) => {
            clearPartialForUtterance(event.payload.utteranceId);
            finalTranscripts.value = [
              event.payload,
              ...finalTranscripts.value,
            ].slice(0, 5);
          },
        ),
      );

      addUnlistener(
        await listen<UtteranceEndedEvent>(
          RUNTIME_EVENTS.utteranceEnded,
          (event) => {
            clearPartialForUtterance(event.payload.utteranceId);
          },
        ),
      );

      addUnlistener(
        await listen<DiagnosticEvent>(RUNTIME_EVENTS.diagnostic, (event) => {
          diagnostics.value = [event.payload, ...diagnostics.value].slice(0, 8);
        }),
      );
    } catch (error) {
      cleanupListeners();
      throw error;
    }
  }

  onMounted(async () => {
    try {
      await registerRuntimeListeners();
      await loadConfig();
      await loadProviderSecretStatus("openai");
      await loadAudioInputDevices();
    } catch (error) {
      actionError.value = normalizeError(error);
    }
  });

  function runBrowserPreviewCommand(command: RuntimeCommand) {
    const timestampMs = Date.now();
    const timestampId = String(timestampMs);

    if (command === "start_runtime" || command === "start_mock_runtime") {
      runtimeStatus.value = {
        status: "running",
        message: "Browser preview runtime is running",
        timestampMs,
      };
      return;
    }

    if (command === "stop_runtime") {
      runtimeStatus.value = {
        status: "stopped",
        message: "Browser preview runtime stopped",
        timestampMs,
      };
      return;
    }

    if (command === "emit_mock_transcript") {
      const transcript: TranscriptEvent = {
        id: `browser-transcript-${timestampId}`,
        utteranceId: `browser-${timestampId}`,
        kind: "final",
        text: "Testing live caption preview from the browser UI.",
        language: config.value?.stt.language ?? "en",
        provider: config.value?.stt.provider ?? "mock",
        revision: 1,
        timestampMs,
      };

      partialTranscript.value = null;
      finalTranscripts.value = [transcript, ...finalTranscripts.value].slice(
        0,
        5,
      );
      return;
    }

    const diagnostic: DiagnosticEvent = {
      id: `browser-diagnostic-${timestampId}`,
      category: "osc",
      severity: "info",
      code: "browser.preview_action",
      message: "Browser preview action",
      detail: "Desktop-only command was simulated for UI preview.",
      timestampMs,
    };

    diagnostics.value = [diagnostic, ...diagnostics.value].slice(0, 8);
  }

  function setBrowserPreviewSecretStatus(provider: SttProvider) {
    if (provider !== "openai") {
      return;
    }

    openAiSecretStatus.value = {
      provider,
      configured: false,
      storage: null,
      displaySuffix: null,
      error: null,
    };
  }

  onBeforeUnmount(() => {
    isUnmounted = true;
    cleanupListeners();
  });

  return {
    actionError,
    activeCaptionText,
    audioInputDevices,
    config,
    diagnostics,
    finalTranscripts,
    isBusy,
    loadAudioInputDevices,
    loadProviderSecretStatus,
    openAiSecretStatus,
    partialTranscript,
    deleteProviderSecret,
    runCommand,
    saveConfig,
    saveProviderSecret,
    runtimeStatus,
  };
}
