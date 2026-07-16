import { computed, onBeforeUnmount, onMounted, ref, shallowRef } from "vue";
import { uiText } from "../i18n/uiText";
import { createRuntimeBackend, type Unsubscribe } from "./backend";
import { createLifecycleCommandQueue } from "./lifecycleCommandQueue";
import {
  createRuntimeState,
  reduceRuntimeState,
  selectRuntimeView,
  type RuntimeStateInput,
} from "./runtimeState";
import type {
  AppConfig,
  AudioInputDevice,
  ProviderSecretStatus,
  RuntimeCommand,
  RuntimeStatusEvent,
  SttProvider,
} from "./types";

const STARTING_STATUS_RECONCILE_INTERVAL_MS = 500;

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
      return true;
    } catch (cause) {
      error.value = normalizeError(cause);
      return false;
    } finally {
      pendingCount.value -= 1;
    }
  }

  return { isBusy, error, run };
}

export function useRuntime() {
  const backend = createRuntimeBackend();
  const runLifecycleCommand = createLifecycleCommandQueue((command) =>
    backend.runCommand(command),
  );
  const audioInputDevices = ref<AudioInputDevice[]>([]);
  const config = ref<AppConfig | null>(null);
  const secretStatuses = ref<
    Partial<Record<SttProvider, ProviderSecretStatus>>
  >({});
  const initialRuntimeStatus: RuntimeStatusEvent = {
    status: "idle",
    message: uiText("runtime.status.initialIdleMessage"),
    timestampMs: Date.now(),
  };
  const runtimeState = shallowRef(createRuntimeState(initialRuntimeStatus));
  const requiresRuntimeRestart = ref(false);
  const pendingRuntimeCommand = ref<RuntimeCommand | null>(null);
  const runtimeAction = createActionState();
  const settingsAction = createActionState();
  const secretsAction = createActionState();
  let unsubscribeListeners: Unsubscribe | null = null;
  let isUnmounted = false;
  let nextLifecycleCommandAttemptId = 0;
  let nextStatusSyncRequestId = 0;
  let startingStatusReconcileTimer: ReturnType<typeof setTimeout> | null = null;
  let startingStatusSyncInFlight = false;

  const runtimeView = computed(() =>
    selectRuntimeView(runtimeState.value, {
      showPartial: config.value?.ui.showPartial ?? true,
    }),
  );
  const runtimeStatus = computed(() => runtimeView.value.runtimeStatus);
  const finalTranscripts = computed(() => runtimeView.value.finalTranscripts);
  const diagnostics = computed(() => runtimeView.value.diagnostics);
  const captionMode = computed(() => runtimeView.value.captionMode);

  const activeCaptionText = computed(() => {
    return (
      runtimeView.value.visibleTranscript?.text ??
      uiText("caption.state.waiting")
    );
  });

  function dispatchRuntimeState(input: RuntimeStateInput) {
    const previousState = runtimeState.value;
    const nextState = reduceRuntimeState(previousState, input);
    runtimeState.value = nextState;
    updateStartingStatusReconciliation();

    return nextState !== previousState;
  }

  function clearStartingStatusReconciliation() {
    if (startingStatusReconcileTimer !== null) {
      clearTimeout(startingStatusReconcileTimer);
      startingStatusReconcileTimer = null;
    }
  }

  function updateStartingStatusReconciliation() {
    if (isUnmounted || runtimeState.value.runtimeStatus.status !== "starting") {
      clearStartingStatusReconciliation();
      return;
    }

    if (startingStatusReconcileTimer !== null || startingStatusSyncInFlight) {
      return;
    }

    // Starting/Running pushes are best-effort and the Rust Start command can
    // return before its worker updates the status snapshot. Poll only while the
    // transition is genuinely unresolved so a missed push cannot strand the UI
    // in Starting; Stop immediately changes the state and cancels this loop.
    startingStatusReconcileTimer = setTimeout(() => {
      startingStatusReconcileTimer = null;
      void reconcileStartingStatus();
    }, STARTING_STATUS_RECONCILE_INTERVAL_MS);
  }

  async function reconcileStartingStatus() {
    if (isUnmounted || runtimeState.value.runtimeStatus.status !== "starting") {
      return;
    }

    startingStatusSyncInFlight = true;

    try {
      await synchronizeRuntimeStatus();
    } catch {
      // A later interval retries while the transition still needs a snapshot.
    } finally {
      startingStatusSyncInFlight = false;
      updateStartingStatusReconciliation();
    }
  }

  async function runCommand(command: RuntimeCommand) {
    pendingRuntimeCommand.value = command;
    let lifecycleCommandAttemptId: number | null = null;

    if (command === "start_runtime" || command === "stop_runtime") {
      nextLifecycleCommandAttemptId += 1;
      lifecycleCommandAttemptId = nextLifecycleCommandAttemptId;
      const commandAccepted = dispatchRuntimeState({
        type: "runtimeCommandRequested",
        attemptId: lifecycleCommandAttemptId,
        command,
        timestampMs: Date.now(),
      });

      if (!commandAccepted) {
        pendingRuntimeCommand.value = null;
        return;
      }
    }

    try {
      const commandSucceeded = await runtimeAction.run(async () => {
        if (command === "start_runtime" || command === "stop_runtime") {
          await runLifecycleCommand(command);
        } else {
          await backend.runCommand(command);
        }

        if (command === "start_runtime") {
          requiresRuntimeRestart.value = false;
        }
      });

      if (
        lifecycleCommandAttemptId !== null &&
        (command === "start_runtime" || command === "stop_runtime")
      ) {
        dispatchRuntimeState(
          commandSucceeded
            ? {
                type: "runtimeCommandSucceeded",
                attemptId: lifecycleCommandAttemptId,
                command,
                timestampMs: Date.now(),
              }
            : {
                type: "runtimeCommandFailed",
                attemptId: lifecycleCommandAttemptId,
                command,
              },
        );

        try {
          // Runtime events are best-effort. Reconcile every lifecycle command
          // with the pull snapshot so a missed stopped/running push cannot
          // leave the controls permanently in their optimistic state.
          await synchronizeRuntimeStatus();
        } catch {
          // Keep the command result visible. Stop has already failed closed (or
          // converged from its successful acknowledgement), while Start remains
          // safely optimistic until a later status push or reload snapshot.
        }
      }
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

  async function registerRuntimeListeners() {
    const unsubscribe = await backend.listen((event) => {
      const eventAccepted = dispatchRuntimeState({
        type: "backendEvent",
        event,
      });

      if (
        eventAccepted &&
        event.type === "status" &&
        event.payload.status === "starting"
      ) {
        requiresRuntimeRestart.value = false;
      }
    });

    if (isUnmounted) {
      unsubscribe();
      return;
    }

    unsubscribeListeners = unsubscribe;
  }

  function beginRuntimeStatusSync() {
    nextStatusSyncRequestId += 1;
    const requestId = nextStatusSyncRequestId;
    dispatchRuntimeState({ type: "runtimeStatusSyncStarted", requestId });

    return requestId;
  }

  async function completeRuntimeStatusSync(requestId: number) {
    try {
      const snapshot = await backend.getRuntimeStatus();
      dispatchRuntimeState({
        type: "runtimeStatusSyncCompleted",
        requestId,
        snapshot,
      });
    } catch (error) {
      dispatchRuntimeState({
        type: "runtimeStatusSyncCancelled",
        requestId,
      });
      throw error;
    }
  }

  async function synchronizeRuntimeStatus() {
    await completeRuntimeStatusSync(beginRuntimeStatusSync());
  }

  async function registerAndSynchronizeRuntime() {
    const requestId = beginRuntimeStatusSync();

    try {
      // Open the reload buffer before registering listeners. Otherwise an event
      // emitted between the last listener registration and the pull snapshot
      // could be rejected against the synthetic initial status.
      await registerRuntimeListeners();
      await completeRuntimeStatusSync(requestId);
    } catch (error) {
      dispatchRuntimeState({
        type: "runtimeStatusSyncCancelled",
        requestId,
      });
      throw error;
    }
  }

  onMounted(async () => {
    await runtimeAction.run(async () => {
      await registerAndSynchronizeRuntime();
    });
    await Promise.all([
      loadConfig(),
      loadAudioInputDevices(),
      loadProviderSecretStatus("openai"),
    ]);
  });

  onBeforeUnmount(() => {
    isUnmounted = true;
    clearStartingStatusReconciliation();
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
