import { computed, onBeforeUnmount, onMounted, ref, shallowRef } from "vue";
import { uiText } from "../i18n/uiText";
import { createRuntimeBackend, type Unsubscribe } from "./backend";
import {
  createCaptionSessionState,
  reduceCaptionSessionState,
  selectCaptionSessionView,
  type CaptionSessionStateInput,
} from "./captionSession";
import { createLifecycleCommandQueue } from "./lifecycleCommandQueue";
import {
  projectRuntimeControlSnapshot,
  reconcileRuntimeControlSnapshot,
  runtimeStatusNeedsControlReconciliation,
} from "./runtimeControl";
import {
  createRuntimeReadinessGate,
  type RuntimeReadinessSnapshot,
} from "./runtimeReadiness";
import {
  createRuntimeState,
  reduceRuntimeState,
  selectRuntimeView,
  type RuntimeStateInput,
} from "./runtimeState";
import type {
  AppConfig,
  AudioInputDevice,
  CaptionDisplay,
  RuntimeCommand,
  RuntimeControlSnapshot,
  RuntimeStatusEvent,
  SttProvider,
} from "./types";

const RUNTIME_CONTROL_RECONCILE_INTERVAL_MS = 500;

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
  const runLifecycleCommand = createLifecycleCommandQueue(async (command) => {
    const snapshot =
      command === "start_runtime"
        ? await backend.startRuntime()
        : await backend.stopRuntime();
    applyControlSnapshot(snapshot);
  });
  const audioInputDevices = ref<AudioInputDevice[]>([]);
  const controlSnapshot = shallowRef<RuntimeControlSnapshot | null>(null);
  const initialRuntimeStatus: RuntimeStatusEvent = {
    status: "idle",
    message: uiText("runtime.status.initialIdleMessage"),
    timestampMs: Date.now(),
  };
  const runtimeState = shallowRef(createRuntimeState(initialRuntimeStatus));
  const captionSessionState = shallowRef(createCaptionSessionState());
  const pendingRuntimeCommand = ref<RuntimeCommand | null>(null);
  const runtimeAction = createActionState();
  const settingsAction = createActionState();
  const secretsAction = createActionState();
  const runtimeReadiness = shallowRef<RuntimeReadinessSnapshot>({
    ready: false,
    isBusy: false,
    error: "",
  });
  let unsubscribeListeners: Unsubscribe | null = null;
  let isUnmounted = false;
  let nextLifecycleCommandAttemptId = 0;
  let nextStatusSyncRequestId = 0;
  let runtimeControlGapObserved = false;
  let runtimeControlPullsInFlight = 0;
  let runtimeControlReconcileTimer: ReturnType<typeof setTimeout> | null = null;
  let runtimeControlReconcileInFlight = false;
  const runtimeReadinessGate = createRuntimeReadinessGate(
    normalizeError,
    (snapshot) => {
      runtimeReadiness.value = snapshot;
      updateRuntimeControlReconciliation();
    },
  );

  const controlView = computed(() =>
    projectRuntimeControlSnapshot(controlSnapshot.value),
  );
  const config = computed(() => controlView.value.config);
  const currentSession = computed(() => controlView.value.currentSession);
  const currentSetupConfig = computed(
    () => controlView.value.currentSetupConfig,
  );
  const pendingSessionChanges = computed(
    () => controlView.value.pendingSessionChanges,
  );
  const sessionUploadsMicrophoneAudio = computed(
    () => controlView.value.sessionUploadsMicrophoneAudio,
  );
  const secretStatuses = computed(() => controlView.value.secretStatuses);
  const runtimeError = computed(
    () => runtimeReadiness.value.error || runtimeAction.error.value,
  );
  const isRuntimeBusy = computed(
    () => runtimeReadiness.value.isBusy || runtimeAction.isBusy.value,
  );
  const settingsError = computed(
    () => settingsAction.error.value || runtimeReadiness.value.error,
  );
  const secretsError = computed(
    () => secretsAction.error.value || runtimeReadiness.value.error,
  );

  const runtimeView = computed(() => selectRuntimeView(runtimeState.value));
  const captionView = computed(() =>
    selectCaptionSessionView(
      captionSessionState.value,
      config.value?.ui.showPartial ?? true,
    ),
  );
  const runtimeStatus = computed(() => runtimeView.value.runtimeStatus);
  const finalTranscripts = computed<readonly CaptionDisplay[]>(() =>
    captionView.value.completedCaptions.map((caption) => ({
      ...caption,
      id: [
        caption.generation,
        caption.streamId,
        caption.unitId ?? "unitless",
        caption.lane,
        caption.revision,
      ].join(":"),
    })),
  );
  const diagnostics = computed(() => runtimeView.value.diagnostics);
  const captionMode = computed(() => captionView.value.captionMode);

  const activeCaptionText = computed(() => {
    return (
      captionView.value.visibleCaption?.text ?? uiText("caption.state.waiting")
    );
  });

  function dispatchRuntimeState(input: RuntimeStateInput) {
    const previousState = runtimeState.value;
    const nextState = reduceRuntimeState(previousState, input);
    runtimeState.value = nextState;
    updateRuntimeControlReconciliation();

    return nextState !== previousState;
  }

  function dispatchCaptionSessionState(input: CaptionSessionStateInput) {
    const previousState = captionSessionState.value;
    const nextState = reduceCaptionSessionState(previousState, input);
    captionSessionState.value = nextState;

    return nextState !== previousState;
  }

  function storeControlSnapshot(snapshot: RuntimeControlSnapshot) {
    const previous = controlSnapshot.value;
    const next = reconcileRuntimeControlSnapshot(previous, snapshot);

    if (next === previous) {
      return false;
    }

    controlSnapshot.value = next;
    return true;
  }

  function applyControlSnapshot(snapshot: RuntimeControlSnapshot) {
    if (!storeControlSnapshot(snapshot)) {
      return false;
    }

    dispatchRuntimeState({
      type: "backendEvent",
      event: { type: "status", payload: snapshot.runtime },
    });
    return true;
  }

  function clearRuntimeControlReconciliation() {
    if (runtimeControlReconcileTimer !== null) {
      clearTimeout(runtimeControlReconcileTimer);
      runtimeControlReconcileTimer = null;
    }
  }

  function updateRuntimeControlReconciliation() {
    if (
      runtimeControlGapObserved &&
      !runtimeStatusNeedsControlReconciliation(
        controlSnapshot.value,
        runtimeStatus.value,
      )
    ) {
      runtimeControlGapObserved = false;
    }

    const needsReconciliation =
      runtimeStatus.value.status === "starting" || runtimeControlGapObserved;

    if (isUnmounted || !needsReconciliation) {
      clearRuntimeControlReconciliation();
      return;
    }

    if (
      runtimeControlReconcileTimer !== null ||
      runtimeControlReconcileInFlight ||
      runtimeControlPullsInFlight > 0 ||
      runtimeReadiness.value.isBusy
    ) {
      return;
    }

    // Both control and legacy status events are best-effort. Poll only while a
    // Start remains unresolved or an accepted legacy status proves the full
    // control snapshot fell behind. A successful pull/control event cancels the
    // loop; failures retry without entering a user action's error scope.
    runtimeControlReconcileTimer = setTimeout(() => {
      runtimeControlReconcileTimer = null;
      void reconcileRuntimeControl();
    }, RUNTIME_CONTROL_RECONCILE_INTERVAL_MS);
  }

  async function reconcileRuntimeControl() {
    const needsReconciliation =
      runtimeStatus.value.status === "starting" || runtimeControlGapObserved;

    if (
      isUnmounted ||
      !needsReconciliation ||
      runtimeControlReconcileInFlight ||
      runtimeControlPullsInFlight > 0
    ) {
      return;
    }

    runtimeControlReconcileInFlight = true;

    try {
      await synchronizeRuntimeControl();
    } catch {
      // A later interval retries while the transition still needs a snapshot.
    } finally {
      runtimeControlReconcileInFlight = false;
      updateRuntimeControlReconciliation();
    }
  }

  async function runCommand(command: RuntimeCommand) {
    if (!(await ensureRuntimeReady()) || isUnmounted) {
      return;
    }

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

      if (command === "stop_runtime") {
        dispatchCaptionSessionState({ type: "stopRequested" });
      }
    }

    try {
      const commandSucceeded = await runtimeAction.run(async () => {
        if (command === "start_runtime" || command === "stop_runtime") {
          await runLifecycleCommand(command);
        } else {
          await backend.runCommand(command);
        }
      });

      if (
        lifecycleCommandAttemptId !== null &&
        (command === "start_runtime" || command === "stop_runtime")
      ) {
        const lifecycleResultAccepted = dispatchRuntimeState(
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

        if (
          lifecycleResultAccepted &&
          commandSucceeded &&
          command === "start_runtime"
        ) {
          dispatchCaptionSessionState({ type: "startSucceeded" });
        }
        if (
          lifecycleResultAccepted &&
          !commandSucceeded &&
          command === "stop_runtime"
        ) {
          dispatchCaptionSessionState({ type: "stopFailed" });
        }

        try {
          dispatchCaptionSessionState({
            type: "snapshotReceived",
            snapshot: await backend.getCaptionSessionSnapshot(),
          });
        } catch {
          // The best-effort push channel or a later control reconciliation can
          // still converge after this command-level caption pull fails.
        }

        if (!commandSucceeded) {
          try {
            await synchronizeRuntimeControl();
          } catch {
            // Keep the command failure visible if the authoritative pull also
            // fails. A later control event or reload can still resynchronize.
          }
        }
      }
    } finally {
      if (pendingRuntimeCommand.value === command) {
        pendingRuntimeCommand.value = null;
      }
    }
  }

  async function saveConfig(nextConfig: AppConfig) {
    let didSave = false;

    await settingsAction.run(async () => {
      applyControlSnapshot(await backend.saveConfig(nextConfig));
      didSave = true;
    });

    return didSave;
  }

  async function loadAudioInputDevices() {
    await settingsAction.run(async () => {
      audioInputDevices.value = await backend.listAudioInputDevices();
    });
  }

  async function saveProviderSecret(provider: SttProvider, secret: string) {
    await secretsAction.run(async () => {
      applyControlSnapshot(await backend.saveProviderSecret(provider, secret));
    });
  }

  async function deleteProviderSecret(provider: SttProvider) {
    await secretsAction.run(async () => {
      applyControlSnapshot(await backend.deleteProviderSecret(provider));
    });
  }

  async function registerRuntimeListeners() {
    if (unsubscribeListeners !== null) {
      return;
    }

    let unsubscribe: Unsubscribe | null = null;

    try {
      unsubscribe = await backend.listen((event) => {
        if (event.type === "captionSessionChanged") {
          dispatchCaptionSessionState({
            type: "snapshotReceived",
            snapshot: event.payload,
          });
          return;
        }

        const eventAccepted = dispatchRuntimeState({
          type: "backendEvent",
          event,
        });

        if (eventAccepted && event.type === "status") {
          runtimeControlGapObserved = runtimeStatusNeedsControlReconciliation(
            controlSnapshot.value,
            event.payload,
          );
          updateRuntimeControlReconciliation();
        }
      });
      const unsubscribeControl = await backend.listenControl((snapshot) => {
        applyControlSnapshot(snapshot);
        runtimeReadinessGate.markReady();
      });

      if (isUnmounted) {
        unsubscribe();
        unsubscribeControl();
        throw new Error("Runtime listener registration was cancelled.");
      }

      unsubscribeListeners = () => {
        unsubscribe?.();
        unsubscribeControl();
      };
    } catch (error) {
      unsubscribe?.();
      throw error;
    }
  }

  function beginRuntimeStatusSync() {
    nextStatusSyncRequestId += 1;
    const requestId = nextStatusSyncRequestId;
    dispatchRuntimeState({ type: "runtimeStatusSyncStarted", requestId });

    return requestId;
  }

  async function completeRuntimeControlSync(requestId: number) {
    runtimeControlPullsInFlight += 1;

    try {
      const [incoming, captions] = await Promise.all([
        backend.getControlSnapshot(),
        backend.getCaptionSessionSnapshot(),
      ]);
      storeControlSnapshot(incoming);
      dispatchCaptionSessionState({
        type: "snapshotReceived",
        snapshot: captions,
      });
      const snapshot = controlSnapshot.value?.runtime ?? incoming.runtime;
      dispatchRuntimeState({
        type: "runtimeStatusSyncCompleted",
        requestId,
        snapshot,
      });

      if (unsubscribeListeners !== null) {
        runtimeReadinessGate.markReady();
      }
    } catch (error) {
      dispatchRuntimeState({
        type: "runtimeStatusSyncCancelled",
        requestId,
      });
      throw error;
    } finally {
      runtimeControlPullsInFlight -= 1;
      updateRuntimeControlReconciliation();
    }
  }

  async function synchronizeRuntimeControl() {
    await completeRuntimeControlSync(beginRuntimeStatusSync());
  }

  async function establishRuntimeReadiness() {
    const requestId = beginRuntimeStatusSync();

    try {
      // Open the reload buffer before registering listeners. Otherwise an event
      // emitted between the last listener registration and the pull snapshot
      // could be rejected against the synthetic initial status.
      await registerRuntimeListeners();
      await completeRuntimeControlSync(requestId);
    } catch (error) {
      dispatchRuntimeState({
        type: "runtimeStatusSyncCancelled",
        requestId,
      });
      throw error;
    }
  }

  function ensureRuntimeReady() {
    return runtimeReadinessGate.ensure(establishRuntimeReadiness);
  }

  onMounted(async () => {
    await ensureRuntimeReady();

    if (!isUnmounted) {
      await loadAudioInputDevices();
    }
  });

  onBeforeUnmount(() => {
    isUnmounted = true;
    clearRuntimeControlReconciliation();
    unsubscribeListeners?.();
    unsubscribeListeners = null;
  });

  return {
    activeCaptionText,
    audioInputDevices,
    captionMode,
    config,
    currentSession,
    currentSetupConfig,
    deleteProviderSecret,
    diagnostics,
    finalTranscripts,
    isRuntimeBusy,
    isSecretsBusy: secretsAction.isBusy,
    isSettingsBusy: settingsAction.isBusy,
    loadAudioInputDevices,
    pendingRuntimeCommand,
    pendingSessionChanges,
    runCommand,
    runtimeError,
    runtimeStatus,
    saveConfig,
    saveProviderSecret,
    secretStatuses,
    sessionUploadsMicrophoneAudio,
    secretsError,
    settingsError,
  };
}
