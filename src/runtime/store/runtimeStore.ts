import { computed, ref, shallowRef } from "vue";
import { uiText } from "../../i18n/uiText";
import type { AppConfig } from "../appConfig";
import { normalizeAppFailure, type AppFailure } from "../appFailure";
import type { AudioInputDevice } from "../audio";
import {
  createCaptionAggregateState,
  reduceCaptionAggregateState,
  selectCaptionAggregateView,
  type CaptionAggregateStateInput,
  type CaptionDisplay,
} from "../captionAggregate";
import type { AppGateway, Unsubscribe } from "../gateway";
import { createLifecycleActionQueue } from "../lifecycleActionQueue";
import type { RuntimeAction } from "../lifecycle";
import {
  projectRuntimeControlSnapshot,
  runtimeStatusNeedsControlReconciliation,
  selectNewerRuntimeControlSnapshot,
  type CredentialId,
  type RuntimeControlSnapshot,
} from "../runtimeControl";
import {
  createRuntimeSynchronizationGate,
  type RuntimeSynchronizationSnapshot,
} from "../runtimeSynchronization";
import {
  createRuntimeState,
  reduceRuntimeState,
  selectRuntimeView,
  type RuntimeStateInput,
} from "../runtimeState";
import type { RuntimeStatusEvent } from "../runtimeEvents";
import { translationPresentation as projectTranslationPresentation } from "../translationPresentation";
import { createAudioInputState } from "./audioInput";

const RUNTIME_CONTROL_RECONCILE_INTERVAL_MS = 500;

function assertUnreachableRuntimeAction(action: never): never {
  throw new Error(`Unsupported runtime action: ${String(action)}`);
}

// One busy/error scope per action domain, so a slow settings save cannot
// disable runtime controls or surface its error on an unrelated page.
function createActionState() {
  const inFlightCount = ref(0);
  const failure = shallowRef<AppFailure | null>(null);
  const isBusy = computed(() => inFlightCount.value > 0);

  async function run(action: () => Promise<void>) {
    failure.value = null;
    inFlightCount.value += 1;

    try {
      await action();
      return true;
    } catch (cause) {
      failure.value = normalizeAppFailure(
        cause,
        uiText("runtime.errors.unknownAction"),
      );
      return false;
    } finally {
      inFlightCount.value -= 1;
    }
  }

  return { isBusy, failure, run };
}

export function createRuntimeStore(gateway: AppGateway) {
  const audioInput = createAudioInputState(gateway);
  const runLifecycleAction = createLifecycleActionQueue(async (action) => {
    const snapshot =
      action === "start"
        ? await gateway.startRuntime()
        : await gateway.stopRuntime();
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
  const captionAggregateState = shallowRef(createCaptionAggregateState());
  const inFlightRuntimeAction = ref<RuntimeAction | null>(null);
  const runtimeAction = createActionState();
  const settingsAction = createActionState();
  const credentialAction = createActionState();
  const runtimeSynchronization = shallowRef<RuntimeSynchronizationSnapshot>({
    isSynchronized: false,
    isSynchronizing: false,
    failure: null,
  });
  let unsubscribeListeners: Unsubscribe | null = null;
  let isDisposed = false;
  let nextLifecycleActionAttemptId = 0;
  let nextStatusSyncRequestId = 0;
  let runtimeControlGapObserved = false;
  let runtimeControlPullsInFlight = 0;
  let runtimeControlReconcileTimer: ReturnType<typeof setTimeout> | null = null;
  let runtimeControlReconcileInFlight = false;
  const runtimeSynchronizationGate = createRuntimeSynchronizationGate(
    (cause) =>
      normalizeAppFailure(cause, uiText("runtime.errors.unknownAction")),
    (snapshot) => {
      runtimeSynchronization.value = snapshot;
      updateRuntimeControlReconciliation();
    },
  );

  const controlView = computed(() =>
    projectRuntimeControlSnapshot(controlSnapshot.value),
  );
  const desiredConfig = computed(() => controlView.value.desiredConfig);
  const desiredCaptionPipelinePlan = computed(
    () => controlView.value.desiredCaptionPipelinePlan,
  );
  const currentGenerationCaptionPipelinePlan = computed(
    () => controlView.value.currentGenerationCaptionPipelinePlan,
  );
  const currentGeneration = computed(() => controlView.value.currentGeneration);
  const currentGenerationSelection = computed(
    () => controlView.value.currentGenerationSelection,
  );
  const pendingGenerationChanges = computed(
    () => controlView.value.pendingGenerationChanges,
  );
  const currentGenerationUploadsMicrophoneAudio = computed(
    () => controlView.value.currentGenerationUploadsMicrophoneAudio,
  );
  const credentialStatuses = computed(
    () => controlView.value.credentialStatuses,
  );
  const runtimeFailure = computed(
    () => runtimeSynchronization.value.failure ?? runtimeAction.failure.value,
  );
  const isRuntimeBusy = computed(
    () =>
      runtimeSynchronization.value.isSynchronizing ||
      runtimeAction.isBusy.value,
  );
  const settingsFailure = computed(
    () => settingsAction.failure.value ?? runtimeSynchronization.value.failure,
  );
  const credentialFailure = computed(
    () =>
      credentialAction.failure.value ?? runtimeSynchronization.value.failure,
  );

  const runtimeView = computed(() => selectRuntimeView(runtimeState.value));
  const captionView = computed(() =>
    selectCaptionAggregateView(
      captionAggregateState.value,
      desiredConfig.value?.ui.showOngoingPreview ?? true,
    ),
  );
  const translationPresentation = computed(() =>
    projectTranslationPresentation(
      currentGeneration.value,
      captionAggregateState.value.admission === "open"
        ? captionAggregateState.value.snapshot
        : null,
    ),
  );
  const runtimeStatus = computed(() => runtimeView.value.runtimeStatus);
  const completedCaptions = computed<readonly CaptionDisplay[]>(() =>
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
  const captionPreviewStatus = computed(
    () => captionView.value.captionPreviewStatus,
  );

  const visibleCaptionText = computed(() => {
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

  function dispatchCaptionAggregateState(input: CaptionAggregateStateInput) {
    const previousState = captionAggregateState.value;
    const nextState = reduceCaptionAggregateState(previousState, input);
    captionAggregateState.value = nextState;

    return nextState !== previousState;
  }

  function storeControlSnapshot(snapshot: RuntimeControlSnapshot) {
    const previous = controlSnapshot.value;
    const next = selectNewerRuntimeControlSnapshot(previous, snapshot);

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

    // The control revision admitted this snapshot. Its wall-clock timestamp is
    // display metadata and must not reorder authoritative runtime state.
    dispatchRuntimeState({
      type: "runtimeControlStatusReceived",
      revision: snapshot.revision,
      snapshot: snapshot.runtimeStatus,
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

    if (isDisposed || !needsReconciliation) {
      clearRuntimeControlReconciliation();
      return;
    }

    if (
      runtimeControlReconcileTimer !== null ||
      runtimeControlReconcileInFlight ||
      runtimeControlPullsInFlight > 0 ||
      runtimeSynchronization.value.isSynchronizing
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
      isDisposed ||
      !needsReconciliation ||
      runtimeControlReconcileInFlight ||
      runtimeControlPullsInFlight > 0
    ) {
      return;
    }

    runtimeControlReconcileInFlight = true;

    try {
      await synchronizeRuntimeState();
    } catch {
      // A later interval retries while the transition still needs a snapshot.
    } finally {
      runtimeControlReconcileInFlight = false;
      updateRuntimeControlReconciliation();
    }
  }

  async function runAction(action: RuntimeAction) {
    if (!(await ensureRuntimeSynchronized()) || isDisposed) {
      return;
    }

    inFlightRuntimeAction.value = action;
    let lifecycleActionAttemptId: number | null = null;

    if (action === "start" || action === "stop") {
      nextLifecycleActionAttemptId += 1;
      lifecycleActionAttemptId = nextLifecycleActionAttemptId;
      const actionAccepted = dispatchRuntimeState({
        type: "runtimeActionRequested",
        attemptId: lifecycleActionAttemptId,
        action,
        timestampMs: Date.now(),
      });

      if (!actionAccepted) {
        inFlightRuntimeAction.value = null;
        return;
      }

      if (action === "stop") {
        dispatchCaptionAggregateState({ type: "stopRequested" });
      }
    }

    try {
      const actionSucceeded = await runtimeAction.run(async () => {
        switch (action) {
          case "start":
          case "stop":
            await runLifecycleAction(action);
            break;
          case "testChatbox":
            await gateway.sendOscTestMessage();
            break;
          default:
            assertUnreachableRuntimeAction(action);
        }
      });

      if (
        lifecycleActionAttemptId !== null &&
        (action === "start" || action === "stop")
      ) {
        const lifecycleResultAccepted = dispatchRuntimeState(
          actionSucceeded
            ? {
                type: "runtimeActionSucceeded",
                attemptId: lifecycleActionAttemptId,
                action,
                timestampMs: Date.now(),
              }
            : {
                type: "runtimeActionFailed",
                attemptId: lifecycleActionAttemptId,
                action,
              },
        );

        if (lifecycleResultAccepted && actionSucceeded && action === "start") {
          dispatchCaptionAggregateState({ type: "startSucceeded" });
        }
        if (lifecycleResultAccepted && !actionSucceeded && action === "stop") {
          dispatchCaptionAggregateState({ type: "stopFailed" });
        }

        try {
          dispatchCaptionAggregateState({
            type: "snapshotReceived",
            snapshot: await gateway.getCaptionAggregateSnapshot(),
          });
        } catch {
          // The best-effort push channel or a later control reconciliation can
          // still converge after this action-level caption pull fails.
        }

        if (!actionSucceeded) {
          try {
            await synchronizeRuntimeState();
          } catch {
            // Keep the action failure visible if the authoritative pull also
            // fails. A later control event or reload can still resynchronize.
          }
        }
      }
    } finally {
      if (inFlightRuntimeAction.value === action) {
        inFlightRuntimeAction.value = null;
      }
    }
  }

  async function saveConfig(nextConfig: AppConfig) {
    let didSave = false;

    await settingsAction.run(async () => {
      applyControlSnapshot(await gateway.saveAppConfig(nextConfig));
      didSave = true;
    });

    return didSave;
  }

  async function loadAudioInputDevices() {
    await settingsAction.run(async () => {
      audioInputDevices.value = await gateway.listAudioInputDevices();
    });
  }

  async function saveCredential(id: CredentialId, secret: string) {
    await credentialAction.run(async () => {
      applyControlSnapshot(await gateway.saveCredential(id, secret));
    });
  }

  async function deleteCredential(id: CredentialId) {
    await credentialAction.run(async () => {
      applyControlSnapshot(await gateway.deleteCredential(id));
    });
  }

  async function registerRuntimeListeners() {
    if (unsubscribeListeners !== null) {
      return;
    }

    let unsubscribe: Unsubscribe | null = null;

    try {
      unsubscribe = await gateway.subscribeRuntimeEvents((event) => {
        if (event.type === "audioLevel") {
          audioInput.acceptAudioLevel(event.payload);
          return;
        }

        if (event.type === "captionAggregateChanged") {
          dispatchCaptionAggregateState({
            type: "snapshotReceived",
            snapshot: event.payload,
          });
          return;
        }

        const eventAccepted = dispatchRuntimeState({
          type: "runtimeEventReceived",
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
      const unsubscribeControl = await gateway.subscribeRuntimeControlSnapshots(
        (snapshot) => {
          applyControlSnapshot(snapshot);
          runtimeSynchronizationGate.markSynchronized();
        },
      );

      if (isDisposed) {
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

  function beginRuntimeStateSynchronization() {
    nextStatusSyncRequestId += 1;
    const requestId = nextStatusSyncRequestId;
    dispatchRuntimeState({
      type: "runtimeStateSynchronizationStarted",
      requestId,
    });

    return requestId;
  }

  async function completeRuntimeStateSynchronization(requestId: number) {
    runtimeControlPullsInFlight += 1;

    try {
      const [incoming, captions] = await Promise.all([
        gateway.getRuntimeControlSnapshot(),
        gateway.getCaptionAggregateSnapshot(),
      ]);
      storeControlSnapshot(incoming);
      dispatchCaptionAggregateState({
        type: "snapshotReceived",
        snapshot: captions,
      });
      const snapshot = controlSnapshot.value ?? incoming;
      dispatchRuntimeState({
        type: "runtimeStateSynchronizationCompleted",
        requestId,
        controlRevision: snapshot.revision,
        snapshot: snapshot.runtimeStatus,
      });

      if (unsubscribeListeners !== null) {
        runtimeSynchronizationGate.markSynchronized();
      }
    } catch (error) {
      dispatchRuntimeState({
        type: "runtimeStateSynchronizationCancelled",
        requestId,
      });
      throw error;
    } finally {
      runtimeControlPullsInFlight -= 1;
      updateRuntimeControlReconciliation();
    }
  }

  async function synchronizeRuntimeState() {
    await completeRuntimeStateSynchronization(
      beginRuntimeStateSynchronization(),
    );
  }

  async function establishRuntimeSynchronization() {
    const requestId = beginRuntimeStateSynchronization();

    try {
      // Open the reload buffer before registering listeners. Otherwise an event
      // emitted between the last listener registration and the pull snapshot
      // could be rejected against the synthetic initial status.
      await registerRuntimeListeners();
      await completeRuntimeStateSynchronization(requestId);
    } catch (error) {
      dispatchRuntimeState({
        type: "runtimeStateSynchronizationCancelled",
        requestId,
      });
      throw error;
    }
  }

  function ensureRuntimeSynchronized() {
    return runtimeSynchronizationGate.ensureSynchronized(
      establishRuntimeSynchronization,
    );
  }

  async function connect() {
    await ensureRuntimeSynchronized();

    if (!isDisposed) {
      await loadAudioInputDevices();
    }
  }

  function dispose() {
    if (isDisposed) {
      return;
    }

    isDisposed = true;
    clearRuntimeControlReconciliation();
    unsubscribeListeners?.();
    unsubscribeListeners = null;
  }

  return {
    connect,
    dispose,
    runtime: {
      audioInputDevices,
      audioProbeFailure: audioInput.audioProbeFailure,
      audioProbeResult: audioInput.audioProbeResult,
      captionPreviewStatus,
      completedCaptions,
      credentialFailure,
      credentialStatuses,
      currentGeneration,
      currentGenerationCaptionPipelinePlan,
      currentGenerationSelection,
      currentGenerationUploadsMicrophoneAudio,
      deleteCredential,
      desiredCaptionPipelinePlan,
      desiredConfig,
      diagnostics,
      isRuntimeBusy,
      isAudioProbeRunning: audioInput.isAudioProbeRunning,
      isCredentialBusy: credentialAction.isBusy,
      isSettingsBusy: settingsAction.isBusy,
      loadAudioInputDevices,
      latestAudioLevel: audioInput.latestAudioLevel,
      inFlightRuntimeAction,
      pendingGenerationChanges,
      probeAudioInput: audioInput.probeAudioInput,
      runAction,
      runtimeFailure,
      runtimeStatus,
      saveConfig,
      saveCredential,
      settingsFailure,
      translationPresentation,
      visibleCaptionText,
    },
  };
}
