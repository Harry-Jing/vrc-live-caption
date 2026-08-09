import type {
  DiagnosticEvent,
  RuntimeEvent,
  RuntimeLifecycleCommand,
  RuntimeStatus,
  RuntimeStatusEvent,
} from "./types";
import { isActiveRuntimeStatus } from "./lifecycle";

const DIAGNOSTIC_LIMIT = 50;

type InFlightLifecycleCommand = Readonly<{
  attemptId: number;
  command: RuntimeLifecycleCommand;
  previousLifecycleIntentCommand: RuntimeLifecycleCommand | null;
  previousLifecycleIntentAtMs: number;
  previousRuntimeStatus: RuntimeStatusEvent;
  previousStopAcknowledgedAtMs: number;
  statusObservationVersionAtRequest: number;
}>;

export type RuntimeStateInput =
  | { type: "backendEvent"; event: RuntimeEvent }
  | {
      type: "runtimeControlStatusReceived";
      revision: number;
      snapshot: RuntimeStatusEvent;
    }
  | {
      type: "runtimeCommandRequested";
      attemptId: number;
      command: RuntimeLifecycleCommand;
      timestampMs: number;
    }
  | {
      type: "runtimeCommandFailed";
      attemptId: number;
      command: RuntimeLifecycleCommand;
    }
  | {
      type: "runtimeCommandSucceeded";
      attemptId: number;
      command: RuntimeLifecycleCommand;
      timestampMs: number;
    }
  | { type: "runtimeStatusSyncStarted"; requestId: number }
  | { type: "runtimeStatusSyncCancelled"; requestId: number }
  | {
      type: "runtimeStatusSyncCompleted";
      requestId: number;
      controlRevision: number;
      snapshot: RuntimeStatusEvent;
    };

export type RuntimeState = Readonly<{
  runtimeStatus: RuntimeStatusEvent;
  latestLifecycleIntentCommand: RuntimeLifecycleCommand | null;
  latestLifecycleIntentAtMs: number;
  stopAcknowledgedAtMs: number;
  diagnostics: readonly DiagnosticEvent[];
  inFlightLifecycleCommand: InFlightLifecycleCommand | null;
  runtimeControlRevision: number;
  statusTimestampWatermarkMs: number;
  statusObservationVersion: number;
  inFlightStatusSync: Readonly<{
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
    state.latestLifecycleIntentCommand === "start_runtime" &&
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
  const inFlightCommand = state.inFlightLifecycleCommand?.command;

  if (
    inFlightCommand === "start_runtime" &&
    isInactiveStatus(runtimeStatus.status) &&
    runtimeStatus.status !== "error"
  ) {
    return true;
  }

  return (
    inFlightCommand === "stop_runtime" &&
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
  const inFlight = state.inFlightLifecycleCommand;
  const inFlightLifecycleCommand =
    inFlight === null
      ? null
      : inFlight.command === "start_runtime"
        ? inFlight
        : isInactiveStatus(runtimeStatus.status) &&
            runtimeStatus.status !== "stopping"
          ? null
          : inFlight;

  return { ...state, runtimeStatus, inFlightLifecycleCommand };
}

export function createRuntimeState(
  runtimeStatus: RuntimeStatusEvent,
): RuntimeState {
  return {
    runtimeStatus,
    latestLifecycleIntentCommand: null,
    latestLifecycleIntentAtMs: Number.NEGATIVE_INFINITY,
    stopAcknowledgedAtMs: Number.NEGATIVE_INFINITY,
    diagnostics: [],
    inFlightLifecycleCommand: null,
    runtimeControlRevision: Number.NEGATIVE_INFINITY,
    statusTimestampWatermarkMs: Number.NEGATIVE_INFINITY,
    statusObservationVersion: 0,
    inFlightStatusSync: null,
  };
}

export function reduceRuntimeState(
  state: RuntimeState,
  input: RuntimeStateInput,
): RuntimeState {
  if (input.type === "runtimeCommandRequested") {
    if (
      (input.command === "start_runtime" &&
        (state.inFlightLifecycleCommand !== null ||
          isActiveRuntimeStatus(state.runtimeStatus.status))) ||
      (input.command === "stop_runtime" &&
        state.inFlightLifecycleCommand?.command === "stop_runtime")
    ) {
      return state;
    }

    const inFlightLifecycleCommand: InFlightLifecycleCommand = {
      attemptId: input.attemptId,
      command: input.command,
      previousLifecycleIntentCommand: state.latestLifecycleIntentCommand,
      previousLifecycleIntentAtMs: state.latestLifecycleIntentAtMs,
      previousRuntimeStatus: state.runtimeStatus,
      previousStopAcknowledgedAtMs: state.stopAcknowledgedAtMs,
      statusObservationVersionAtRequest: state.statusObservationVersion,
    };

    return {
      ...state,
      runtimeStatus: {
        status: input.command === "start_runtime" ? "starting" : "stopping",
        timestampMs: input.timestampMs,
      },
      latestLifecycleIntentCommand: input.command,
      latestLifecycleIntentAtMs: Math.max(
        state.latestLifecycleIntentAtMs,
        input.timestampMs,
      ),
      stopAcknowledgedAtMs:
        input.command === "start_runtime"
          ? Number.NEGATIVE_INFINITY
          : state.stopAcknowledgedAtMs,
      inFlightLifecycleCommand,
    };
  }

  if (input.type === "runtimeCommandFailed") {
    const inFlight = state.inFlightLifecycleCommand;

    if (
      inFlight === null ||
      inFlight.attemptId !== input.attemptId ||
      inFlight.command !== input.command
    ) {
      return state;
    }

    const receivedStatusEvidence =
      state.statusObservationVersion !==
      inFlight.statusObservationVersionAtRequest;

    if (input.command === "stop_runtime") {
      return {
        ...state,
        runtimeStatus: receivedStatusEvidence
          ? state.runtimeStatus
          : inFlight.previousRuntimeStatus,
        inFlightLifecycleCommand: null,
      };
    }

    if (receivedStatusEvidence) {
      return { ...state, inFlightLifecycleCommand: null };
    }

    return {
      ...state,
      runtimeStatus: inFlight.previousRuntimeStatus,
      latestLifecycleIntentCommand: inFlight.previousLifecycleIntentCommand,
      latestLifecycleIntentAtMs: inFlight.previousLifecycleIntentAtMs,
      stopAcknowledgedAtMs: inFlight.previousStopAcknowledgedAtMs,
      inFlightLifecycleCommand: null,
    };
  }

  if (input.type === "runtimeCommandSucceeded") {
    const inFlight = state.inFlightLifecycleCommand;

    if (
      inFlight === null ||
      inFlight.attemptId !== input.attemptId ||
      inFlight.command !== input.command
    ) {
      return state;
    }

    if (input.command === "stop_runtime") {
      return {
        ...state,
        runtimeStatus: { status: "stopped", timestampMs: input.timestampMs },
        inFlightLifecycleCommand: null,
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

    return { ...state, inFlightLifecycleCommand: null };
  }

  if (input.type === "runtimeStatusSyncStarted") {
    return {
      ...state,
      inFlightStatusSync: {
        requestId: input.requestId,
      },
    };
  }

  if (input.type === "runtimeStatusSyncCancelled") {
    return state.inFlightStatusSync?.requestId === input.requestId
      ? { ...state, inFlightStatusSync: null }
      : state;
  }

  if (input.type === "runtimeStatusSyncCompleted") {
    const inFlight = state.inFlightStatusSync;

    if (inFlight === null || inFlight.requestId !== input.requestId) {
      return state;
    }

    const withoutInFlightStatusSync = { ...state, inFlightStatusSync: null };
    return applyRuntimeControlStatus(
      withoutInFlightStatusSync,
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
