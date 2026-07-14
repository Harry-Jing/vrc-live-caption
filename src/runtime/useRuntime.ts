import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { uiText } from "../i18n/uiText";
import {
  createRuntimeBackend,
  type RuntimeEventHandlers,
  type Unsubscribe,
} from "./backend";
import type {
  AppConfig,
  AudioInputDevice,
  CaptionMode,
  DiagnosticEvent,
  ProviderSecretStatus,
  RuntimeCommand,
  RuntimeStatusEvent,
  SttProvider,
  TranscriptEvent,
} from "./types";

const FINAL_TRANSCRIPT_LIMIT = 5;
const DIAGNOSTIC_LIMIT = 50;

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

  return uiText("runtime.errors.unknownAction");
}

// One busy/error scope per action domain, so a slow settings save cannot
// disable runtime controls or surface its error on an unrelated page.
function createActionState() {
  const pendingCount = ref(0);
  const error = ref("");
  const isBusy = computed(() => pendingCount.value > 0);

  async function run(action: () => Promise<void>) {
    error.value = "";
    pendingCount.value += 1;

    try {
      await action();
    } catch (cause) {
      error.value = normalizeError(cause);
    } finally {
      pendingCount.value -= 1;
    }
  }

  return { isBusy, error, run };
}

export function useRuntime() {
  const backend = createRuntimeBackend();
  const audioInputDevices = ref<AudioInputDevice[]>([]);
  const config = ref<AppConfig | null>(null);
  const secretStatuses = ref<
    Partial<Record<SttProvider, ProviderSecretStatus>>
  >({});
  const runtimeStatus = ref<RuntimeStatusEvent>({
    status: "idle",
    message: uiText("runtime.status.initialIdleMessage"),
    timestampMs: Date.now(),
  });
  const activeUtteranceId = ref<string | null>(null);
  const partialTranscript = ref<TranscriptEvent | null>(null);
  const finalTranscripts = ref<TranscriptEvent[]>([]);
  const diagnostics = ref<DiagnosticEvent[]>([]);
  const requiresRuntimeRestart = ref(false);
  const pendingRuntimeCommand = ref<RuntimeCommand | null>(null);
  const runtimeAction = createActionState();
  const settingsAction = createActionState();
  const secretsAction = createActionState();
  let unsubscribeListeners: Unsubscribe | null = null;
  let isUnmounted = false;
  let receivedStatusEventVersion = 0;

  const latestFinalTranscript = computed<TranscriptEvent | null>(
    () => finalTranscripts.value.at(0) ?? null,
  );

  // showPartial only gates partial transcript text. The listening indicator
  // stays active regardless: it is the only feedback between speaking and the
  // final transcript, and the only in-app signal that the microphone is live.
  const captionMode = computed<CaptionMode>(() => {
    if (partialTranscript.value && config.value?.ui.showPartial) {
      return "partial";
    }

    if (activeUtteranceId.value) {
      return "listening";
    }

    return "final";
  });

  const activeCaptionText = computed(() => {
    if (captionMode.value === "partial" && partialTranscript.value) {
      return partialTranscript.value.text;
    }

    if (captionMode.value === "listening") {
      return uiText("caption.state.listening");
    }

    return latestFinalTranscript.value?.text ?? uiText("caption.state.waiting");
  });

  async function runCommand(command: RuntimeCommand) {
    pendingRuntimeCommand.value = command;

    try {
      await runtimeAction.run(async () => {
        await backend.runCommand(command);

        if (command === "start_runtime") {
          requiresRuntimeRestart.value = false;
        }
      });
    } finally {
      if (pendingRuntimeCommand.value === command) {
        pendingRuntimeCommand.value = null;
      }
    }
  }

  async function loadConfig() {
    await settingsAction.run(async () => {
      config.value = await backend.getConfig();
    });
  }

  async function saveConfig(nextConfig: AppConfig) {
    requiresRuntimeRestart.value = false;
    let didSave = false;
    const requiresRestart = ["starting", "running", "stopping"].includes(
      runtimeStatus.value.status,
    );

    await settingsAction.run(async () => {
      config.value = await backend.saveConfig(nextConfig);
      didSave = true;

      if (requiresRestart) {
        requiresRuntimeRestart.value = true;
      }
    });

    return didSave;
  }

  async function loadAudioInputDevices() {
    await settingsAction.run(async () => {
      audioInputDevices.value = await backend.listAudioInputDevices();
    });
  }

  function setSecretStatus(
    provider: SttProvider,
    status: ProviderSecretStatus,
  ) {
    secretStatuses.value = { ...secretStatuses.value, [provider]: status };
  }

  async function loadProviderSecretStatus(provider: SttProvider) {
    await secretsAction.run(async () => {
      setSecretStatus(
        provider,
        await backend.getProviderSecretStatus(provider),
      );
    });
  }

  async function saveProviderSecret(provider: SttProvider, secret: string) {
    await secretsAction.run(async () => {
      setSecretStatus(
        provider,
        await backend.saveProviderSecret(provider, secret),
      );
    });
  }

  async function deleteProviderSecret(provider: SttProvider) {
    await secretsAction.run(async () => {
      setSecretStatus(provider, await backend.deleteProviderSecret(provider));
    });
  }

  // Only the utterance that owns the live caption state may clear it; a final
  // or utterance-end for an older utterance must not wipe a newer one.
  function clearUtteranceState(utteranceId: string) {
    if (activeUtteranceId.value === utteranceId) {
      activeUtteranceId.value = null;
    }

    if (partialTranscript.value?.utteranceId === utteranceId) {
      partialTranscript.value = null;
    }
  }

  function applyRuntimeStatus(event: RuntimeStatusEvent) {
    runtimeStatus.value = event;

    if (event.status === "starting") {
      requiresRuntimeRestart.value = false;
    }

    if (event.status === "stopped" || event.status === "error") {
      activeUtteranceId.value = null;
      partialTranscript.value = null;
    }
  }

  const eventHandlers: RuntimeEventHandlers = {
    onStatus(event) {
      receivedStatusEventVersion += 1;
      applyRuntimeStatus(event);
    },
    onUtteranceStarted(event) {
      activeUtteranceId.value = event.utteranceId;
    },
    onTranscriptPartial(event) {
      partialTranscript.value = event;
    },
    onTranscriptFinal(event) {
      clearUtteranceState(event.utteranceId);
      finalTranscripts.value = [event, ...finalTranscripts.value].slice(
        0,
        FINAL_TRANSCRIPT_LIMIT,
      );
    },
    onUtteranceEnded(event) {
      clearUtteranceState(event.utteranceId);
    },
    onDiagnostic(event) {
      diagnostics.value = [event, ...diagnostics.value].slice(
        0,
        DIAGNOSTIC_LIMIT,
      );
    },
  };

  async function registerRuntimeListeners() {
    const unsubscribe = await backend.listen(eventHandlers);

    if (isUnmounted) {
      unsubscribe();
      return;
    }

    unsubscribeListeners = unsubscribe;
  }

  async function synchronizeRuntimeStatus() {
    const eventVersionBeforeRequest = receivedStatusEventVersion;
    const snapshot = await backend.getRuntimeStatus();

    // A status event may arrive while the command is in flight. Keep that
    // event unless the snapshot is strictly newer; equal timestamps can occur
    // within one millisecond and must not roll the UI backwards.
    if (
      receivedStatusEventVersion === eventVersionBeforeRequest ||
      snapshot.timestampMs > runtimeStatus.value.timestampMs
    ) {
      applyRuntimeStatus(snapshot);
    }
  }

  onMounted(async () => {
    await runtimeAction.run(async () => {
      await registerRuntimeListeners();
      await synchronizeRuntimeStatus();
    });
    await Promise.all([
      loadConfig(),
      loadAudioInputDevices(),
      loadProviderSecretStatus("openai"),
    ]);
  });

  onBeforeUnmount(() => {
    isUnmounted = true;
    unsubscribeListeners?.();
    unsubscribeListeners = null;
  });

  return {
    activeCaptionText,
    audioInputDevices,
    captionMode,
    config,
    deleteProviderSecret,
    diagnostics,
    finalTranscripts,
    isRuntimeBusy: runtimeAction.isBusy,
    isSecretsBusy: secretsAction.isBusy,
    isSettingsBusy: settingsAction.isBusy,
    loadAudioInputDevices,
    pendingRuntimeCommand,
    partialTranscript,
    runCommand,
    runtimeError: runtimeAction.error,
    runtimeStatus,
    saveConfig,
    saveProviderSecret,
    secretStatuses,
    secretsError: secretsAction.error,
    settingsError: settingsAction.error,
    requiresRuntimeRestart,
  };
}
