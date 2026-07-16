import type {
  CaptionMode,
  DiagnosticEvent,
  RuntimeCommand,
  RuntimeEvent,
  RuntimeStatus,
  RuntimeStatusEvent,
  TranscriptEvent,
} from "./types";

const FINAL_TRANSCRIPT_LIMIT = 5;
const DIAGNOSTIC_LIMIT = 50;
const STATUS_SYNC_EVENT_LIMIT = 64;
const UTTERANCE_LEDGER_LIMIT = 256;

type TrackedUtterance = Readonly<{
  utteranceId: string;
  generation: number;
  startedAtMs: number;
  observedOrdinal: number;
  latestRevision: number | null;
  partialTranscript: TranscriptEvent | null;
  terminal: boolean;
}>;

type CompletedTranscript = Readonly<{
  transcript: TranscriptEvent;
  generation: number;
  unitStartedAtMs: number;
  unitOrdinal: number;
}>;

type PendingLifecycleCommand = Readonly<{
  attemptId: number;
  command: RuntimeLifecycleCommand;
  previousGeneration: number;
  previousGenerationStartedAtMs: number;
  previousLifecycleIntentCommand: RuntimeLifecycleCommand | null;
  previousLifecycleIntentAtMs: number;
  previousRuntimeStatus: RuntimeStatusEvent;
  previousCaptionFence: CaptionFence;
  previousStopAcknowledgedAtMs: number;
  statusObservationVersionAtRequest: number;
}>;

type CaptionFence = "open" | "runtimeInactive" | "localStop";

type RuntimeLifecycleCommand = Extract<
  RuntimeCommand,
  "start_runtime" | "stop_runtime"
>;

export type RuntimeStateInput =
  | { type: "backendEvent"; event: RuntimeEvent }
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
      snapshot: RuntimeStatusEvent;
    };

export type RuntimeState = Readonly<{
  runtimeStatus: RuntimeStatusEvent;
  generation: number;
  generationStartedAtMs: number;
  latestLifecycleIntentCommand: RuntimeLifecycleCommand | null;
  latestLifecycleIntentAtMs: number;
  captionFence: CaptionFence;
  stopAcknowledgedAtMs: number;
  activeUtteranceId: string | null;
  completedTranscripts: readonly CompletedTranscript[];
  diagnostics: readonly DiagnosticEvent[];
  trackedUtterances: readonly TrackedUtterance[];
  nextUtteranceOrdinal: number;
  pendingLifecycleCommand: PendingLifecycleCommand | null;
  statusTimestampWatermarkMs: number;
  statusEventVersion: number;
  statusObservationVersion: number;
  pendingStatusSync: Readonly<{
    requestId: number;
    statusEventVersion: number;
    captionEvents: readonly RuntimeEvent[];
  }> | null;
}>;

export type RuntimeView = Readonly<{
  runtimeStatus: RuntimeStatusEvent;
  captionMode: CaptionMode;
  visibleTranscript: TranscriptEvent | null;
  finalTranscripts: readonly TranscriptEvent[];
  diagnostics: readonly DiagnosticEvent[];
}>;

function isTerminalRuntimeStatus(status: RuntimeStatus) {
  return (
    status === "idle" ||
    status === "stopping" ||
    status === "stopped" ||
    status === "error"
  );
}

function compareUtteranceOrder(
  left: Pick<TrackedUtterance, "startedAtMs" | "observedOrdinal">,
  right: Pick<TrackedUtterance, "startedAtMs" | "observedOrdinal">,
) {
  return (
    left.startedAtMs - right.startedAtMs ||
    left.observedOrdinal - right.observedOrdinal
  );
}

function trackUtterance(
  trackedUtterances: readonly TrackedUtterance[],
  utterance: TrackedUtterance,
) {
  return [
    utterance,
    ...trackedUtterances.filter(
      (tracked) => tracked.utteranceId !== utterance.utteranceId,
    ),
  ]
    .sort(
      (left, right) =>
        right.generation - left.generation ||
        compareUtteranceOrder(right, left),
    )
    .slice(0, UTTERANCE_LEDGER_LIMIT);
}

function findTrackedUtterance(state: RuntimeState, utteranceId: string) {
  return state.trackedUtterances.find(
    (utterance) => utterance.utteranceId === utteranceId,
  );
}

function latestTrackedUtterance(
  trackedUtterances: readonly TrackedUtterance[],
  generation: number,
) {
  return trackedUtterances
    .filter((utterance) => utterance.generation === generation)
    .reduce<TrackedUtterance | null>((latest, utterance) => {
      if (!latest || compareUtteranceOrder(utterance, latest) > 0) {
        return utterance;
      }

      return latest;
    }, null);
}

function orderCompletedTranscripts(
  completedTranscripts: readonly CompletedTranscript[],
) {
  return [...completedTranscripts]
    .sort(
      (left, right) =>
        right.generation - left.generation ||
        right.unitStartedAtMs - left.unitStartedAtMs ||
        right.unitOrdinal - left.unitOrdinal,
    )
    .slice(0, FINAL_TRANSCRIPT_LIMIT);
}

function eventCanBelongToCurrentGeneration(
  state: RuntimeState,
  tracked: TrackedUtterance | undefined,
  timestampMs: number,
) {
  if (tracked) {
    return tracked.generation === state.generation;
  }

  // The current wire does not carry generation. A local generation fence plus
  // bounded utterance tombstones deterministically reject recently tracked old
  // units and any unseen event created before the latest Start intent. A
  // never-seen (or evicted) old unit stamped after that fence remains
  // indistinguishable until the versioned wire adds explicit generation.
  return timestampMs >= state.generationStartedAtMs;
}

function isValidRevision(revision: number) {
  return Number.isInteger(revision) && revision > 0;
}

export function createRuntimeState(
  runtimeStatus: RuntimeStatusEvent,
): RuntimeState {
  const runtimeIsInactive = isTerminalRuntimeStatus(runtimeStatus.status);

  return {
    runtimeStatus,
    generation: runtimeIsInactive ? 0 : 1,
    generationStartedAtMs: runtimeStatus.timestampMs,
    latestLifecycleIntentCommand: null,
    latestLifecycleIntentAtMs: Number.NEGATIVE_INFINITY,
    captionFence: runtimeIsInactive ? "runtimeInactive" : "open",
    stopAcknowledgedAtMs: Number.NEGATIVE_INFINITY,
    activeUtteranceId: null,
    completedTranscripts: [],
    diagnostics: [],
    trackedUtterances: [],
    nextUtteranceOrdinal: 0,
    pendingLifecycleCommand: null,
    statusTimestampWatermarkMs: Number.NEGATIVE_INFINITY,
    statusEventVersion: 0,
    statusObservationVersion: 0,
    pendingStatusSync: null,
  };
}

function applyRuntimeStatus(
  state: RuntimeState,
  runtimeStatus: RuntimeStatusEvent,
): RuntimeState {
  const pendingLifecycleCommand = (() => {
    const pending = state.pendingLifecycleCommand;

    if (!pending || pending.command === "start_runtime") {
      return null;
    }

    return runtimeStatus.status === "idle" ||
      runtimeStatus.status === "stopped" ||
      runtimeStatus.status === "error"
      ? null
      : pending;
  })();

  if (isTerminalRuntimeStatus(runtimeStatus.status)) {
    const stopAcknowledgedAtMs =
      state.captionFence === "localStop" && runtimeStatus.status !== "stopping"
        ? Math.max(state.stopAcknowledgedAtMs, runtimeStatus.timestampMs)
        : state.stopAcknowledgedAtMs;

    return {
      ...state,
      runtimeStatus,
      captionFence:
        state.captionFence === "localStop" ? "localStop" : "runtimeInactive",
      stopAcknowledgedAtMs,
      activeUtteranceId: null,
      pendingLifecycleCommand,
    };
  }

  if (state.captionFence === "localStop") {
    return {
      ...state,
      runtimeStatus,
      pendingLifecycleCommand,
    };
  }

  if (state.captionFence === "runtimeInactive") {
    return {
      ...state,
      runtimeStatus,
      generation: state.generation + 1,
      generationStartedAtMs: runtimeStatus.timestampMs,
      captionFence: "open",
      activeUtteranceId: null,
      pendingLifecycleCommand,
    };
  }

  return {
    ...state,
    runtimeStatus,
    pendingLifecycleCommand,
  };
}

function statusPredatesLifecycleIntent(
  state: RuntimeState,
  runtimeStatus: RuntimeStatusEvent,
) {
  if (runtimeStatus.timestampMs < state.latestLifecycleIntentAtMs) {
    return true;
  }

  // The current status wire has no sequence number. If an inactive status ties
  // with a local Start, prefer the newer local intent. Error is the exception:
  // accepting it lets a genuine same-millisecond startup failure fail closed.
  return (
    state.latestLifecycleIntentCommand === "start_runtime" &&
    isTerminalRuntimeStatus(runtimeStatus.status) &&
    runtimeStatus.status !== "error" &&
    runtimeStatus.timestampMs === state.latestLifecycleIntentAtMs
  );
}

function statusPredatesStopAcknowledgement(
  state: RuntimeState,
  runtimeStatus: RuntimeStatusEvent,
) {
  return (
    state.captionFence === "localStop" &&
    runtimeStatus.timestampMs <= state.stopAcknowledgedAtMs
  );
}

function applyCaptionEvent(
  state: RuntimeState,
  event: Exclude<RuntimeEvent, { type: "status" | "diagnostic" }>,
): RuntimeState {
  if (event.type === "utteranceEnded") {
    const payload = event.payload;
    const tracked = findTrackedUtterance(state, payload.utteranceId);

    if (
      !eventCanBelongToCurrentGeneration(state, tracked, payload.timestampMs) ||
      tracked?.terminal
    ) {
      return state;
    }

    const terminalUtterance: TrackedUtterance = {
      utteranceId: payload.utteranceId,
      generation: state.generation,
      startedAtMs: tracked?.startedAtMs ?? payload.timestampMs,
      observedOrdinal:
        tracked?.observedOrdinal ?? state.nextUtteranceOrdinal + 1,
      latestRevision: tracked?.latestRevision ?? null,
      partialTranscript: null,
      terminal: true,
    };

    return {
      ...state,
      activeUtteranceId:
        state.activeUtteranceId === payload.utteranceId
          ? null
          : state.activeUtteranceId,
      trackedUtterances: trackUtterance(
        state.trackedUtterances,
        terminalUtterance,
      ),
      nextUtteranceOrdinal: Math.max(
        state.nextUtteranceOrdinal,
        terminalUtterance.observedOrdinal,
      ),
    };
  }

  if (event.type === "utteranceStarted") {
    const payload = event.payload;
    const tracked = findTrackedUtterance(state, payload.utteranceId);

    if (
      !eventCanBelongToCurrentGeneration(state, tracked, payload.timestampMs)
    ) {
      return state;
    }

    if (tracked) {
      if (payload.timestampMs >= tracked.startedAtMs) {
        return state;
      }

      const correctedUtterance = {
        ...tracked,
        startedAtMs: payload.timestampMs,
      };
      const nextTrackedUtterances = trackUtterance(
        state.trackedUtterances,
        correctedUtterance,
      );
      const completedTranscripts = orderCompletedTranscripts(
        state.completedTranscripts.map((completed) =>
          completed.transcript.utteranceId === payload.utteranceId
            ? { ...completed, unitStartedAtMs: payload.timestampMs }
            : completed,
        ),
      );

      const latest = latestTrackedUtterance(
        nextTrackedUtterances,
        state.generation,
      );
      const activeUtteranceId = latest?.terminal
        ? null
        : (latest?.utteranceId ?? null);

      return {
        ...state,
        activeUtteranceId,
        completedTranscripts,
        trackedUtterances: nextTrackedUtterances,
      };
    }

    const startedUtterance: TrackedUtterance = {
      utteranceId: payload.utteranceId,
      generation: state.generation,
      startedAtMs: payload.timestampMs,
      observedOrdinal: state.nextUtteranceOrdinal + 1,
      latestRevision: null,
      partialTranscript: null,
      terminal: false,
    };
    const nextTrackedUtterances = trackUtterance(
      state.trackedUtterances,
      startedUtterance,
    );

    const latest = latestTrackedUtterance(
      state.trackedUtterances,
      state.generation,
    );

    if (latest && compareUtteranceOrder(startedUtterance, latest) < 0) {
      return {
        ...state,
        trackedUtterances: nextTrackedUtterances,
        nextUtteranceOrdinal: startedUtterance.observedOrdinal,
      };
    }

    return {
      ...state,
      activeUtteranceId: payload.utteranceId,
      trackedUtterances: nextTrackedUtterances,
      nextUtteranceOrdinal: startedUtterance.observedOrdinal,
    };
  }

  const transcript = event.payload;

  if (
    (event.type === "transcriptPartial" && transcript.kind !== "partial") ||
    (event.type === "transcriptFinal" && transcript.kind !== "final") ||
    !isValidRevision(transcript.revision)
  ) {
    return state;
  }

  const tracked = findTrackedUtterance(state, transcript.utteranceId);

  if (
    !eventCanBelongToCurrentGeneration(
      state,
      tracked,
      transcript.timestampMs,
    ) ||
    tracked?.terminal ||
    (tracked?.latestRevision !== null &&
      tracked?.latestRevision !== undefined &&
      transcript.revision <= tracked.latestRevision)
  ) {
    return state;
  }

  const unitStartedAtMs = tracked?.startedAtMs ?? transcript.timestampMs;
  const observedOrdinal =
    tracked?.observedOrdinal ?? state.nextUtteranceOrdinal + 1;
  const nextUtterance: TrackedUtterance = {
    utteranceId: transcript.utteranceId,
    generation: state.generation,
    startedAtMs: unitStartedAtMs,
    observedOrdinal,
    latestRevision: transcript.revision,
    partialTranscript: event.type === "transcriptPartial" ? transcript : null,
    terminal: event.type === "transcriptFinal",
  };
  const nextTrackedUtterances = trackUtterance(
    state.trackedUtterances,
    nextUtterance,
  );

  if (event.type === "transcriptPartial") {
    const latest = latestTrackedUtterance(
      state.trackedUtterances,
      state.generation,
    );

    if (latest && compareUtteranceOrder(nextUtterance, latest) < 0) {
      return {
        ...state,
        trackedUtterances: nextTrackedUtterances,
        nextUtteranceOrdinal: Math.max(
          state.nextUtteranceOrdinal,
          observedOrdinal,
        ),
      };
    }

    return {
      ...state,
      activeUtteranceId: transcript.utteranceId,
      trackedUtterances: nextTrackedUtterances,
      nextUtteranceOrdinal: Math.max(
        state.nextUtteranceOrdinal,
        observedOrdinal,
      ),
    };
  }

  const completedTranscripts = orderCompletedTranscripts([
    {
      transcript,
      generation: state.generation,
      unitStartedAtMs,
      unitOrdinal: observedOrdinal,
    },
    ...state.completedTranscripts,
  ]);

  return {
    ...state,
    activeUtteranceId:
      state.activeUtteranceId === transcript.utteranceId
        ? null
        : state.activeUtteranceId,
    completedTranscripts,
    trackedUtterances: nextTrackedUtterances,
    nextUtteranceOrdinal: Math.max(state.nextUtteranceOrdinal, observedOrdinal),
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

    const latestLifecycleIntentAtMs = Math.max(
      state.latestLifecycleIntentAtMs,
      input.timestampMs,
    );
    const pendingLifecycleCommand: PendingLifecycleCommand = {
      attemptId: input.attemptId,
      command: input.command,
      previousGeneration: state.generation,
      previousGenerationStartedAtMs: state.generationStartedAtMs,
      previousLifecycleIntentCommand: state.latestLifecycleIntentCommand,
      previousLifecycleIntentAtMs: state.latestLifecycleIntentAtMs,
      previousRuntimeStatus: state.runtimeStatus,
      previousCaptionFence: state.captionFence,
      previousStopAcknowledgedAtMs: state.stopAcknowledgedAtMs,
      statusObservationVersionAtRequest: state.statusObservationVersion,
    };

    if (input.command === "start_runtime") {
      return {
        ...state,
        runtimeStatus: {
          status: "starting",
          timestampMs: input.timestampMs,
        },
        generation: state.generation + 1,
        generationStartedAtMs: input.timestampMs,
        latestLifecycleIntentCommand: input.command,
        latestLifecycleIntentAtMs,
        captionFence: "open",
        stopAcknowledgedAtMs: Number.NEGATIVE_INFINITY,
        activeUtteranceId: null,
        pendingLifecycleCommand,
      };
    }

    return {
      ...state,
      runtimeStatus: {
        status: "stopping",
        timestampMs: input.timestampMs,
      },
      latestLifecycleIntentCommand: input.command,
      latestLifecycleIntentAtMs,
      captionFence: "localStop",
      activeUtteranceId: null,
      pendingLifecycleCommand,
      pendingStatusSync: state.pendingStatusSync
        ? { ...state.pendingStatusSync, captionEvents: [] }
        : null,
    };
  }

  if (input.type === "runtimeCommandFailed") {
    const pending = state.pendingLifecycleCommand;

    if (
      !pending ||
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
      generation: pending.previousGeneration,
      generationStartedAtMs: pending.previousGenerationStartedAtMs,
      latestLifecycleIntentCommand: pending.previousLifecycleIntentCommand,
      latestLifecycleIntentAtMs: pending.previousLifecycleIntentAtMs,
      captionFence: pending.previousCaptionFence,
      stopAcknowledgedAtMs: pending.previousStopAcknowledgedAtMs,
      activeUtteranceId: null,
      completedTranscripts: state.completedTranscripts.filter(
        (completed) => completed.generation !== state.generation,
      ),
      trackedUtterances: state.trackedUtterances.filter(
        (utterance) => utterance.generation !== state.generation,
      ),
      pendingLifecycleCommand: null,
    };
  }

  if (input.type === "runtimeCommandSucceeded") {
    const pending = state.pendingLifecycleCommand;

    if (
      !pending ||
      pending.attemptId !== input.attemptId ||
      pending.command !== input.command
    ) {
      return state;
    }

    if (input.command === "stop_runtime") {
      // A successful Stop command returns only after the Rust runtime has
      // joined. Treat that command acknowledgement as authoritative even when
      // the best-effort stopping/stopped events were missed.
      return {
        ...state,
        runtimeStatus: {
          status: "stopped",
          timestampMs: input.timestampMs,
        },
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

    // A successful Start only confirms that the worker was spawned. Keep the
    // optimistic Starting state until a push or pull observes Running/Error,
    // but release the command attempt so Stop can always preempt it.
    return { ...state, pendingLifecycleCommand: null };
  }

  if (input.type === "runtimeStatusSyncStarted") {
    return {
      ...state,
      pendingStatusSync: {
        requestId: input.requestId,
        statusEventVersion: state.statusEventVersion,
        captionEvents: [],
      },
    };
  }

  if (input.type === "runtimeStatusSyncCancelled") {
    if (state.pendingStatusSync?.requestId !== input.requestId) {
      return state;
    }

    return { ...state, pendingStatusSync: null };
  }

  if (input.type === "runtimeStatusSyncCompleted") {
    const pending = state.pendingStatusSync;

    if (!pending || pending.requestId !== input.requestId) {
      return state;
    }

    const withoutPending = { ...state, pendingStatusSync: null };
    const statusEventArrived =
      state.statusEventVersion !== pending.statusEventVersion;
    const snapshotPredatesLocalIntent = statusPredatesLifecycleIntent(
      state,
      input.snapshot,
    );
    const snapshotPredatesStopAcknowledgement =
      statusPredatesStopAcknowledgement(state, input.snapshot);
    const snapshotPredatesAcceptedStatus =
      input.snapshot.timestampMs < state.statusTimestampWatermarkMs;
    let synchronizedState: RuntimeState;

    if (
      snapshotPredatesLocalIntent ||
      snapshotPredatesStopAcknowledgement ||
      snapshotPredatesAcceptedStatus ||
      (statusEventArrived &&
        input.snapshot.timestampMs <= state.statusTimestampWatermarkMs)
    ) {
      synchronizedState = withoutPending;
    } else {
      synchronizedState = applyRuntimeStatus(
        {
          ...withoutPending,
          statusTimestampWatermarkMs: input.snapshot.timestampMs,
          statusObservationVersion: state.statusObservationVersion + 1,
        },
        input.snapshot,
      );
    }

    return pending.captionEvents.reduce(
      (nextState, event) =>
        reduceRuntimeState(nextState, { type: "backendEvent", event }),
      synchronizedState,
    );
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
        statusEventVersion: state.statusEventVersion + 1,
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

  if (state.captionFence !== "open") {
    if (state.pendingStatusSync && state.captionFence === "runtimeInactive") {
      return {
        ...state,
        pendingStatusSync: {
          ...state.pendingStatusSync,
          captionEvents: [
            ...state.pendingStatusSync.captionEvents,
            input.event,
          ].slice(-STATUS_SYNC_EVENT_LIMIT),
        },
      };
    }

    return state;
  }

  return applyCaptionEvent(state, input.event);
}

export function selectRuntimeView(
  state: RuntimeState,
  options: Readonly<{ showPartial: boolean }>,
): RuntimeView {
  const activePartial = state.activeUtteranceId
    ? (findTrackedUtterance(state, state.activeUtteranceId)
        ?.partialTranscript ?? null)
    : null;
  const visiblePartial = options.showPartial ? activePartial : null;
  const finalTranscripts = state.completedTranscripts.map(
    (completed) => completed.transcript,
  );

  return {
    runtimeStatus: state.runtimeStatus,
    captionMode: visiblePartial
      ? "partial"
      : state.activeUtteranceId
        ? "listening"
        : finalTranscripts.length > 0
          ? "final"
          : "waiting",
    visibleTranscript: visiblePartial ?? finalTranscripts.at(0) ?? null,
    finalTranscripts,
    diagnostics: state.diagnostics,
  };
}
