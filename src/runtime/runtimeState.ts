import type {
  DiagnosticEvent,
  RuntimeCommand,
  RuntimeEvent,
  RuntimeStatus,
  RuntimeStatusEvent,
} from "./types";

const DIAGNOSTIC_LIMIT = 50;

type RuntimeLifecycleCommand = Extract<
  RuntimeCommand,
  "start_runtime" | "stop_runtime"
>;

type PendingLifecycleCommand = Readonly<{
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
  pendingLifecycleCommand: PendingLifecycleCommand | null;
  runtimeControlRevision: number;
  statusTimestampWatermarkMs: number;
  statusObservationVersion: number;
  pendingStatusSync: Readonly<{
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
  const pendingCommand = state.pendingLifecycleCommand?.command;

  if (
    pendingCommand === "start_runtime" &&
    isInactiveStatus(runtimeStatus.status) &&
    runtimeStatus.status !== "error"
  ) {
    return true;
  }

  return (
    pendingCommand === "stop_runtime" &&
    (runtimeStatus.status === "starting" || runtimeStatus.status === "running")
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
  const pending = state.pendingLifecycleCommand;
  const pendingLifecycleCommand =
    pending === null
      ? null
      : pending.command === "start_runtime"
        ? runtimeStatus.status === "starting"
          ? pending
          : null
        : isInactiveStatus(runtimeStatus.status) &&
            runtimeStatus.status !== "stopping"
          ? null
          : pending;

  return { ...state, runtimeStatus, pendingLifecycleCommand };
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
    pendingLifecycleCommand: null,
    runtimeControlRevision: Number.NEGATIVE_INFINITY,
    statusTimestampWatermarkMs: Number.NEGATIVE_INFINITY,
    statusObservationVersion: 0,
    pendingStatusSync: null,
  };
}

export function reduceRuntimeState(
  state: RuntimeState,
  input: RuntimeStateInput,
): RuntimeState {
  if (input.type === "runtimeCommandRequested") {
    if (
      (input.command === "start_runtime" &&
        (state.pendingLifecycleCommand !== null ||
          state.runtimeStatus.status === "starting" ||
          state.runtimeStatus.status === "running" ||
          state.runtimeStatus.status === "stopping")) ||
      (input.command === "stop_runtime" &&
        state.pendingLifecycleCommand?.command === "stop_runtime")
    ) {
      return state;
    }

    const pendingLifecycleCommand: PendingLifecycleCommand = {
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
      pendingLifecycleCommand,
    };
  }

  if (input.type === "runtimeCommandFailed") {
    const pending = state.pendingLifecycleCommand;

    if (
      pending === null ||
      pending.attemptId !== input.attemptId ||
      pending.command !== input.command
    ) {
      return state;
    }

    if (input.command === "stop_runtime") {
      const receivedStatusEvidence =
        state.statusObservationVersion !==
        pending.statusObservationVersionAtRequest;

      return {
        ...state,
        runtimeStatus: receivedStatusEvidence
          ? state.runtimeStatus
          : pending.previousRuntimeStatus,
        pendingLifecycleCommand: null,
      };
    }

    return {
      ...state,
      runtimeStatus: pending.previousRuntimeStatus,
      latestLifecycleIntentCommand: pending.previousLifecycleIntentCommand,
      latestLifecycleIntentAtMs: pending.previousLifecycleIntentAtMs,
      stopAcknowledgedAtMs: pending.previousStopAcknowledgedAtMs,
      pendingLifecycleCommand: null,
    };
  }

  if (input.type === "runtimeCommandSucceeded") {
    const pending = state.pendingLifecycleCommand;

    if (
      pending === null ||
      pending.attemptId !== input.attemptId ||
      pending.command !== input.command
    ) {
      return state;
    }

    if (input.command === "stop_runtime") {
      return {
        ...state,
        runtimeStatus: { status: "stopped", timestampMs: input.timestampMs },
        pendingLifecycleCommand: null,
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

    return { ...state, pendingLifecycleCommand: null };
  }

  if (input.type === "runtimeStatusSyncStarted") {
    return {
      ...state,
      pendingStatusSync: {
        requestId: input.requestId,
      },
    };
  }

  if (input.type === "runtimeStatusSyncCancelled") {
    return state.pendingStatusSync?.requestId === input.requestId
      ? { ...state, pendingStatusSync: null }
      : state;
  }

  if (input.type === "runtimeStatusSyncCompleted") {
    const pending = state.pendingStatusSync;

    if (pending === null || pending.requestId !== input.requestId) {
      return state;
    }

    const withoutPending = { ...state, pendingStatusSync: null };
    return applyRuntimeControlStatus(
      withoutPending,
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
