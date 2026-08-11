// Reconciles optimistic Start/Stop actions with authoritative revisioned control
// status and best-effort status events. Revisions and lifecycle watermarks keep
// stale async observations from undoing newer evidence, especially Stop.
import type {
  DiagnosticEvent,
  RuntimeEvent,
  RuntimeStatus,
  RuntimeStatusEvent,
} from "./runtimeEvents";
import {
  isActiveRuntimeStatus,
  type RuntimeLifecycleAction,
} from "./lifecycle";

const DIAGNOSTIC_LIMIT = 50;

type InFlightLifecycleAction = Readonly<{
  attemptId: number;
  action: RuntimeLifecycleAction;
  previousLifecycleIntentAction: RuntimeLifecycleAction | null;
  previousLifecycleIntentAtMs: number;
  previousRuntimeStatus: RuntimeStatusEvent;
  previousStopAcknowledgedAtMs: number;
  statusObservationVersionAtRequest: number;
}>;

export type RuntimeStateInput =
  | { type: "runtimeEventReceived"; event: RuntimeEvent }
  | {
      type: "runtimeControlStatusReceived";
      revision: number;
      snapshot: RuntimeStatusEvent;
    }
  | {
      type: "runtimeActionRequested";
      attemptId: number;
      action: RuntimeLifecycleAction;
      timestampMs: number;
    }
  | {
      type: "runtimeActionFailed";
      attemptId: number;
      action: RuntimeLifecycleAction;
    }
  | {
      type: "runtimeActionSucceeded";
      attemptId: number;
      action: RuntimeLifecycleAction;
      timestampMs: number;
    }
  | { type: "runtimeStateSynchronizationStarted"; requestId: number }
  | { type: "runtimeStateSynchronizationCancelled"; requestId: number }
  | {
      type: "runtimeStateSynchronizationCompleted";
      requestId: number;
      controlRevision: number;
      snapshot: RuntimeStatusEvent;
    };

export type RuntimeState = Readonly<{
  runtimeStatus: RuntimeStatusEvent;
  latestLifecycleIntentAction: RuntimeLifecycleAction | null;
  latestLifecycleIntentAtMs: number;
  stopAcknowledgedAtMs: number;
  diagnostics: readonly DiagnosticEvent[];
  inFlightLifecycleAction: InFlightLifecycleAction | null;
  runtimeControlRevision: number;
  statusTimestampWatermarkMs: number;
  statusObservationVersion: number;
  inFlightRuntimeStateSynchronization: Readonly<{
    requestId: number;
  }> | null;
}>;

export type RuntimeView = Readonly<{
  runtimeStatus: RuntimeStatusEvent;
  diagnostics: readonly DiagnosticEvent[];
}>;

function isInactiveStatus(status: RuntimeStatus) {
  return (
    status === "idle" ||
    status === "stopping" ||
    status === "stopped" ||
    status === "error"
  );
}

function statusPredatesLifecycleIntent(
  state: RuntimeState,
  runtimeStatus: RuntimeStatusEvent,
) {
  if (runtimeStatus.timestampMs < state.latestLifecycleIntentAtMs) {
    return true;
  }

  return (
    state.latestLifecycleIntentAction === "start" &&
    isInactiveStatus(runtimeStatus.status) &&
    runtimeStatus.status !== "error" &&
    runtimeStatus.timestampMs === state.latestLifecycleIntentAtMs
  );
}

function statusPredatesStopAcknowledgement(
  state: RuntimeState,
  runtimeStatus: RuntimeStatusEvent,
) {
  return runtimeStatus.timestampMs <= state.stopAcknowledgedAtMs;
}

function controlStatusConflictsWithLifecycleTransition(
  state: RuntimeState,
  runtimeStatus: RuntimeStatusEvent,
) {
  const inFlightAction = state.inFlightLifecycleAction?.action;

  if (
    inFlightAction === "start" &&
    isInactiveStatus(runtimeStatus.status) &&
    runtimeStatus.status !== "error"
  ) {
    return true;
  }

  return (
    inFlightAction === "stop" &&
    (runtimeStatus.status === "starting" ||
      runtimeStatus.status === "running" ||
      runtimeStatus.status === "reconnecting")
  );
}

function applyRuntimeControlStatus(
  state: RuntimeState,
  revision: number,
  runtimeStatus: RuntimeStatusEvent,
) {
  if (
    revision <= state.runtimeControlRevision ||
    controlStatusConflictsWithLifecycleTransition(state, runtimeStatus)
  ) {
    return state;
  }

  return applyRuntimeStatus(
    {
      ...state,
      runtimeControlRevision: revision,
      statusObservationVersion: state.statusObservationVersion + 1,
      statusTimestampWatermarkMs: Math.max(
        state.statusTimestampWatermarkMs,
        runtimeStatus.timestampMs,
      ),
    },
    runtimeStatus,
  );
}

function applyRuntimeStatus(
  state: RuntimeState,
  runtimeStatus: RuntimeStatusEvent,
): RuntimeState {
  const inFlight = state.inFlightLifecycleAction;
  const inFlightLifecycleAction =
    inFlight === null
      ? null
      : inFlight.action === "start"
        ? inFlight
        : isInactiveStatus(runtimeStatus.status) &&
            runtimeStatus.status !== "stopping"
          ? null
          : inFlight;

  return { ...state, runtimeStatus, inFlightLifecycleAction };
}

export function createRuntimeState(
  runtimeStatus: RuntimeStatusEvent,
): RuntimeState {
  return {
    runtimeStatus,
    latestLifecycleIntentAction: null,
    latestLifecycleIntentAtMs: Number.NEGATIVE_INFINITY,
    stopAcknowledgedAtMs: Number.NEGATIVE_INFINITY,
    diagnostics: [],
    inFlightLifecycleAction: null,
    runtimeControlRevision: Number.NEGATIVE_INFINITY,
    statusTimestampWatermarkMs: Number.NEGATIVE_INFINITY,
    statusObservationVersion: 0,
    inFlightRuntimeStateSynchronization: null,
  };
}

export function reduceRuntimeState(
  state: RuntimeState,
  input: RuntimeStateInput,
): RuntimeState {
  if (input.type === "runtimeActionRequested") {
    if (
      (input.action === "start" &&
        (state.inFlightLifecycleAction !== null ||
          isActiveRuntimeStatus(state.runtimeStatus.status))) ||
      (input.action === "stop" &&
        state.inFlightLifecycleAction?.action === "stop")
    ) {
      return state;
    }

    const inFlightLifecycleAction: InFlightLifecycleAction = {
      attemptId: input.attemptId,
      action: input.action,
      previousLifecycleIntentAction: state.latestLifecycleIntentAction,
      previousLifecycleIntentAtMs: state.latestLifecycleIntentAtMs,
      previousRuntimeStatus: state.runtimeStatus,
      previousStopAcknowledgedAtMs: state.stopAcknowledgedAtMs,
      statusObservationVersionAtRequest: state.statusObservationVersion,
    };

    return {
      ...state,
      runtimeStatus: {
        status: input.action === "start" ? "starting" : "stopping",
        timestampMs: input.timestampMs,
      },
      latestLifecycleIntentAction: input.action,
      latestLifecycleIntentAtMs: Math.max(
        state.latestLifecycleIntentAtMs,
        input.timestampMs,
      ),
      stopAcknowledgedAtMs:
        input.action === "start"
          ? Number.NEGATIVE_INFINITY
          : state.stopAcknowledgedAtMs,
      inFlightLifecycleAction,
    };
  }

  if (input.type === "runtimeActionFailed") {
    const inFlight = state.inFlightLifecycleAction;

    if (
      inFlight === null ||
      inFlight.attemptId !== input.attemptId ||
      inFlight.action !== input.action
    ) {
      return state;
    }

    const receivedStatusEvidence =
      state.statusObservationVersion !==
      inFlight.statusObservationVersionAtRequest;

    if (input.action === "stop") {
      return {
        ...state,
        runtimeStatus: receivedStatusEvidence
          ? state.runtimeStatus
          : inFlight.previousRuntimeStatus,
        inFlightLifecycleAction: null,
      };
    }

    if (receivedStatusEvidence) {
      return { ...state, inFlightLifecycleAction: null };
    }

    return {
      ...state,
      runtimeStatus: inFlight.previousRuntimeStatus,
      latestLifecycleIntentAction: inFlight.previousLifecycleIntentAction,
      latestLifecycleIntentAtMs: inFlight.previousLifecycleIntentAtMs,
      stopAcknowledgedAtMs: inFlight.previousStopAcknowledgedAtMs,
      inFlightLifecycleAction: null,
    };
  }

  if (input.type === "runtimeActionSucceeded") {
    const inFlight = state.inFlightLifecycleAction;

    if (
      inFlight === null ||
      inFlight.attemptId !== input.attemptId ||
      inFlight.action !== input.action
    ) {
      return state;
    }

    if (input.action === "stop") {
      return {
        ...state,
        runtimeStatus: { status: "stopped", timestampMs: input.timestampMs },
        inFlightLifecycleAction: null,
        stopAcknowledgedAtMs: Math.max(
          state.stopAcknowledgedAtMs,
          input.timestampMs,
        ),
        statusTimestampWatermarkMs: Math.max(
          state.statusTimestampWatermarkMs,
          input.timestampMs,
        ),
      };
    }

    return { ...state, inFlightLifecycleAction: null };
  }

  if (input.type === "runtimeStateSynchronizationStarted") {
    return {
      ...state,
      inFlightRuntimeStateSynchronization: {
        requestId: input.requestId,
      },
    };
  }

  if (input.type === "runtimeStateSynchronizationCancelled") {
    return state.inFlightRuntimeStateSynchronization?.requestId ===
      input.requestId
      ? { ...state, inFlightRuntimeStateSynchronization: null }
      : state;
  }

  if (input.type === "runtimeStateSynchronizationCompleted") {
    const inFlight = state.inFlightRuntimeStateSynchronization;

    if (inFlight === null || inFlight.requestId !== input.requestId) {
      return state;
    }

    const withoutInFlightRuntimeStateSynchronization = {
      ...state,
      inFlightRuntimeStateSynchronization: null,
    };
    return applyRuntimeControlStatus(
      withoutInFlightRuntimeStateSynchronization,
      input.controlRevision,
      input.snapshot,
    );
  }

  if (input.type === "runtimeControlStatusReceived") {
    return applyRuntimeControlStatus(state, input.revision, input.snapshot);
  }

  if (input.event.type === "status") {
    const status = input.event.payload;

    if (
      statusPredatesLifecycleIntent(state, status) ||
      statusPredatesStopAcknowledgement(state, status) ||
      status.timestampMs < state.statusTimestampWatermarkMs
    ) {
      return state;
    }

    return applyRuntimeStatus(
      {
        ...state,
        statusObservationVersion: state.statusObservationVersion + 1,
        statusTimestampWatermarkMs: status.timestampMs,
      },
      status,
    );
  }

  if (input.event.type === "diagnostic") {
    const event = input.event.payload;

    if (state.diagnostics.some((diagnostic) => diagnostic.id === event.id)) {
      return state;
    }

    return {
      ...state,
      diagnostics: [event, ...state.diagnostics].slice(0, DIAGNOSTIC_LIMIT),
    };
  }

  return state;
}

export function selectRuntimeView(state: RuntimeState): RuntimeView {
  return {
    runtimeStatus: state.runtimeStatus,
    diagnostics: state.diagnostics,
  };
}
