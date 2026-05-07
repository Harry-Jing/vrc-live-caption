import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import {
  RUNTIME_EVENTS,
  type AppConfig,
  type DiagnosticEvent,
  type RuntimeCommand,
  type RuntimeStatusEvent,
  type TranscriptEvent,
} from "./types";

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

export function useRuntime() {
  const config = ref<AppConfig | null>(null);
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
      await invoke(command);
    } catch (error) {
      actionError.value = normalizeError(error);
    } finally {
      isBusy.value = false;
    }
  }

  async function loadConfig() {
    actionError.value = "";

    try {
      config.value = await invoke<AppConfig>("get_app_config");
    } catch (error) {
      actionError.value = normalizeError(error);
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
    try {
      addUnlistener(
        await listen<RuntimeStatusEvent>(RUNTIME_EVENTS.status, (event) => {
          runtimeStatus.value = event.payload;
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
            partialTranscript.value = null;
            finalTranscripts.value = [
              event.payload,
              ...finalTranscripts.value,
            ].slice(0, 5);
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
    } catch (error) {
      actionError.value = normalizeError(error);
    }
  });

  onBeforeUnmount(() => {
    isUnmounted = true;
    cleanupListeners();
  });

  return {
    actionError,
    activeCaptionText,
    config,
    diagnostics,
    finalTranscripts,
    isBusy,
    partialTranscript,
    runCommand,
    runtimeStatus,
  };
}
