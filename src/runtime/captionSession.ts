import type {
  CaptionPreviewStatus,
  CaptionSessionSnapshotV1,
  CaptionSnapshotV1,
} from "./types";

const COMPLETED_CAPTION_DISPLAY_LIMIT = 5;

type CaptionAdmission = "open" | "stopped" | "awaitingStartSnapshot";

export type CaptionSessionState = Readonly<{
  snapshot: CaptionSessionSnapshotV1 | null;
  admission: CaptionAdmission;
  admissionBeforeStop: Exclude<CaptionAdmission, "stopped"> | null;
  highestGenerationSeen: number;
}>;

export type CaptionSessionStateInput =
  | Readonly<{
      type: "snapshotReceived";
      snapshot: CaptionSessionSnapshotV1;
    }>
  | Readonly<{ type: "stopRequested" }>
  | Readonly<{ type: "stopFailed" }>
  | Readonly<{ type: "startSucceeded" }>;

export type CaptionSessionView = Readonly<{
  captionPreviewStatus: CaptionPreviewStatus;
  visibleCaption: CaptionSnapshotV1 | null;
  completedCaptions: readonly CaptionSnapshotV1[];
}>;

export function createCaptionSessionState(): CaptionSessionState {
  return {
    snapshot: null,
    admission: "open",
    admissionBeforeStop: null,
    highestGenerationSeen: 0,
  };
}

export function reduceCaptionSessionState(
  state: CaptionSessionState,
  input: CaptionSessionStateInput,
): CaptionSessionState {
  if (input.type === "stopRequested") {
    return state.admission === "stopped"
      ? state
      : {
          ...state,
          admission: "stopped",
          admissionBeforeStop: state.admission,
        };
  }

  if (input.type === "stopFailed") {
    return state.admission === "stopped" && state.admissionBeforeStop !== null
      ? {
          ...state,
          admission: state.admissionBeforeStop,
          admissionBeforeStop: null,
        }
      : state;
  }

  if (input.type === "startSucceeded") {
    return state.admission === "open" && state.snapshot?.active != null
      ? state
      : {
          ...state,
          admission: "awaitingStartSnapshot",
          admissionBeforeStop: null,
        };
  }

  if (
    state.snapshot !== null &&
    input.snapshot.snapshotRevision <= state.snapshot.snapshotRevision
  ) {
    return state;
  }

  if (state.admission === "stopped" && input.snapshot.active !== null) {
    return state;
  }

  if (
    input.snapshot.active !== null &&
    input.snapshot.active.generation < state.highestGenerationSeen
  ) {
    return state;
  }
  if (
    state.admission === "awaitingStartSnapshot" &&
    input.snapshot.active !== null &&
    input.snapshot.active.generation <= state.highestGenerationSeen
  ) {
    return state;
  }

  const admission =
    state.admission === "awaitingStartSnapshot" &&
    input.snapshot.active !== null
      ? "open"
      : state.admission;

  return {
    ...state,
    admission,
    admissionBeforeStop:
      admission === "stopped" ? state.admissionBeforeStop : null,
    snapshot: input.snapshot,
    highestGenerationSeen: Math.max(
      state.highestGenerationSeen,
      input.snapshot.active?.generation ?? 0,
    ),
  };
}

export function selectCaptionSessionView(
  state: CaptionSessionState,
  showOngoing: boolean,
): CaptionSessionView {
  const captions = state.snapshot?.captions ?? [];
  const completedCaptions = captions
    .filter(
      (caption) => caption.lane === "source" && caption.state === "completed",
    )
    .slice(0, COMPLETED_CAPTION_DISPLAY_LIMIT);
  const sessionIsAdmitted = state.admission === "open";
  const ongoingCaption =
    sessionIsAdmitted && showOngoing
      ? (captions.find(
          (caption) => caption.lane === "source" && caption.state === "ongoing",
        ) ?? null)
      : null;
  const hasActiveUnit =
    sessionIsAdmitted && (state.snapshot?.activeUnits.length ?? 0) > 0;

  return {
    captionPreviewStatus: ongoingCaption
      ? "ongoing"
      : hasActiveUnit
        ? "listening"
        : completedCaptions.length > 0
          ? "completed"
          : "waiting",
    visibleCaption: ongoingCaption ?? completedCaptions[0] ?? null,
    completedCaptions,
  };
}
